//! 世界：创建、线程上限，以及显卡入口。

mod edit;
mod grid;
mod query;
mod types;

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(test)]
mod tests;

pub use types::{BodyView, SpecError};

use std::collections::HashMap;
use std::sync::Mutex;

use types::{Body, CellHeads, MoveScratch, Proxy, SolidHit, Wide, NONE};

/// 少于这么多个物体时，拆线程的开销比查询本身还大，走单线程。
pub(super) const QUERY_PARALLEL_AT: usize = 192;
/// `0` 表示不另设上限，用进程里现成的线程池。
const DEFAULT_THREADS: usize = 16;

pub struct Foil {
    bodies: Vec<Body>,
    /// 连续编号直接按下标取槽。0 表示空，其余是槽号 + 1。
    index: Vec<u32>,
    /// 离已有编号太远的 id，例如 `u32::MAX`，不把数组撑到那个下标。
    sparse: HashMap<usize, u32>,
    /// 为真时坐标按 f64 保存和比较，查询只走 CPU。
    wide: bool,
    wide_pose: Vec<Wide>,
    free: Vec<u32>,
    proxies: Vec<Proxy>,
    free_proxy: u32,
    ox: i32,
    oy: i32,
    gw: i32,
    gh: i32,
    heads: Vec<CellHeads>,
    extra: HashMap<(i32, i32), CellHeads>,
    /// 和密铺格子对齐的实体邻居。查询不读它。
    solid: Vec<Vec<SolidHit>>,
    extra_solid: HashMap<(i32, i32), Vec<SolidHit>>,
    solid_count: usize,
    /// CPU 上改过显卡要看的形状或坐标。显卡缓存用它决定要不要重传物体。
    pub(crate) pose_gen: u64,
    /// 格子链有没有重挂。没变就继续用显存里的那份。
    pub(crate) grid_gen: u64,
    backend: String,
    /// `0` 不封顶。`1` 强制单线程。其余是查询最多开这么多线程。
    thread_limit: usize,
    thread_pool: Mutex<Option<rayon::ThreadPool>>,
    move_scratch: MoveScratch,
    move_stripes: Vec<Mutex<()>>,
    #[cfg(feature = "gpu")]
    gpu: Option<gpu::FoilGpu>,
}

impl Foil {
    /// CPU、f32，批量查询线程上限 16。
    pub fn new() -> Self {
        Self {
            bodies: Vec::new(),
            index: Vec::new(),
            sparse: HashMap::new(),
            wide: false,
            wide_pose: Vec::new(),
            free: Vec::new(),
            proxies: Vec::new(),
            free_proxy: NONE,
            ox: 0,
            oy: 0,
            gw: 0,
            gh: 0,
            heads: Vec::new(),
            extra: HashMap::new(),
            solid: Vec::new(),
            extra_solid: HashMap::new(),
            solid_count: 0,
            pose_gen: 1,
            grid_gen: 1,
            backend: "cpu".into(),
            thread_limit: DEFAULT_THREADS,
            thread_pool: Mutex::new(None),
            move_scratch: MoveScratch::default(),
            move_stripes: (0..64).map(|_| Mutex::new(())).collect(),
            #[cfg(feature = "gpu")]
            gpu: None,
        }
    }

    /// 按后端创建，精度是 f32。
    /// 名字可以是 `cpu`、`dx12`（也认 `d3d12`、`directx12`）、`vulkan`、`metal`、
    /// `gl`（也认 `opengl`、`gles`、`angle`），以及 `wgpu`、`webgpu`、`gpu`。
    /// 显卡查询是实验功能，传输开销目前大于收益。f64 用 [`Foil::open_with`]。
    pub fn open(backend: &str) -> Result<Self, String> {
        Self::open_with(backend, "f32")
    }

    /// `real` 写 `f32` 或 `f64`。f64 只支持 `cpu`：显卡着色器是 32 位浮点。
    pub fn open_with(backend: &str, real: &str) -> Result<Self, String> {
        let wide = match real.trim().to_ascii_lowercase().as_str() {
            "f32" | "" => false,
            "f64" => true,
            _ => return Err(format!("精度写 f32 或 f64，不认识 {real}")),
        };
        let key = backend.trim().to_ascii_lowercase();
        if wide && !(key.is_empty() || key == "cpu") {
            return Err("f64 只支持 cpu".into());
        }
        if key.is_empty() || key == "cpu" {
            let mut world = Self::new();
            world.wide = wide;
            return Ok(world);
        }
        #[cfg(feature = "gpu")]
        {
            let (device, queue, name) = gpu::open_device(&key)?;
            let gpu = gpu::FoilGpu::new(device, queue)?;
            let mut world = Self::new();
            world.gpu = Some(gpu);
            world.backend = name;
            return Ok(world);
        }
        #[cfg(not(feature = "gpu"))]
        {
            let _ = key;
            Err("当前编译没有 gpu feature，只能用 cpu".into())
        }
    }

    /// 创建时选定的后端名字。某次查询改走 CPU 时，这个字符串不变。
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// `f32` 或 `f64`。
    pub fn real(&self) -> &'static str {
        if self.wide { "f64" } else { "f32" }
    }

    /// 查询最多开几个线程。`0` 不封顶，用进程里现成的池。默认 16。
    pub fn threads(&self) -> usize {
        self.thread_limit
    }

    /// 改查询线程上限。`0` 不封顶，`1` 始终单线程。下次并行查询才按新的数量建池。
    pub fn set_threads(&mut self, n: usize) {
        self.thread_limit = n;
        *self.thread_pool.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub(super) fn fit(&self, v: f64) -> f64 {
        if self.wide { v } else { v as f32 as f64 }
    }

    /// 打开之后，`query` 才读显卡上着色器的耗时。关掉就不插时间戳。CPU 后端没有这个数。
    pub fn set_timing(&mut self, on: bool) {
        #[cfg(feature = "gpu")]
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.set_timing(on);
        }
        let _ = on;
    }

    /// 程序里已经有一块 `wgpu` 设备时接上。之后 `query` 走这块设备，查不了再改回 CPU。
    #[cfg(feature = "gpu")]
    pub fn bind_device(
        &mut self,
        device: std::sync::Arc<wgpu::Device>,
        queue: std::sync::Arc<wgpu::Queue>,
    ) -> Result<(), String> {
        if self.wide {
            return Err("f64 只支持 cpu".into());
        }
        self.gpu = Some(gpu::FoilGpu::new(device, queue)?);
        Ok(())
    }

    /// 创建时选了显卡，或 `bind_device` 接上过一块设备。
    pub fn has_device(&self) -> bool {
        #[cfg(feature = "gpu")]
        {
            self.gpu.is_some()
        }
        #[cfg(not(feature = "gpu"))]
        {
            false
        }
    }

    /// 打开计时之后，历次 `query` 的检测耗时合计，单位纳秒。
    pub fn detect_ns(&self) -> u64 {
        #[cfg(feature = "gpu")]
        if let Some(gpu) = self.gpu.as_ref() {
            return gpu.detect_ns();
        }
        0
    }
}
