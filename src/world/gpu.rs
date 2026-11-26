//! Foil 显卡重叠检测。调用方自己改位置。
//! 检测耗时默认不记。`set_timing(true)` 之后才读显卡时间戳。

use std::sync::mpsc;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};

use super::Foil;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuBody {
    x: f32,
    y: f32,
    ox: f32,
    oy: f32,
    hw: f32,
    hh: f32,
    group: u32,
    mask: u32,
    kind: u32,
    sensor: u32,
    id: u32,
    alive: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuProxy {
    slot: u32,
    next: u32,
    _p0: u32,
    _p1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMover {
    slot: u32,
    dx: f32,
    dy: f32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuHit {
    me: u32,
    other: u32,
    _p0: u32,
    _p1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuGrid {
    ox: i32,
    oy: i32,
    gw: i32,
    gh: i32,
    n_body: u32,
    n_proxy: u32,
    n_in: u32,
    hit_cap: u32,
}

struct Buf {
    buf: wgpu::Buffer,
    cap: usize,
}

struct Ts {
    set: wgpu::QuerySet,
    buf: wgpu::Buffer,
    stage: wgpu::Buffer,
    /// 1 个时间戳计数等于多少纳秒。
    period: f32,
}

/// 着色器本身的耗时，单位纳秒，多次查询累加。只有 `set_timing(true)` 之后才记。
#[derive(Clone, Copy, Default)]
pub(crate) struct GpuTiming {
    pub detect_ns: u64,
}

pub(crate) struct FoilGpu {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    query_pipe: wgpu::ComputePipeline,
    query_bgl: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    bodies: Buf,
    proxies: Buf,
    heads: Buf,
    inputs: Buf,
    hit_count: wgpu::Buffer,
    hits: Buf,
    staging: Buf,
    body_scratch: Vec<GpuBody>,
    proxy_scratch: Vec<GpuProxy>,
    head_scratch: Vec<u32>,
    input_scratch: Vec<GpuMover>,
    ts: Option<Ts>,
    detail: bool,
    timing: GpuTiming,
    cache_pose: u64,
    cache_grid: u64,
    cache_bodies: u32,
    cache_proxies: u32,
    cache_heads: u32,
}

fn storage(device: &wgpu::Device, label: &str, bytes: u64, extra: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | extra,
        mapped_at_creation: false,
    })
}

fn entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl FoilGpu {
    pub(crate) fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, String> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("foil2_gpu"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../foil.wgsl").into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("foil2_uniform"),
            size: std::mem::size_of::<GpuGrid>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let query_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("foil2_query"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, wgpu::BufferBindingType::Storage { read_only: false }),
                entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
                entry(3, wgpu::BufferBindingType::Storage { read_only: true }),
                entry(4, wgpu::BufferBindingType::Storage { read_only: true }),
                entry(6, wgpu::BufferBindingType::Storage { read_only: false }),
                entry(7, wgpu::BufferBindingType::Storage { read_only: false }),
            ],
        });
        let query_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("foil2_query_pl"),
            bind_group_layouts: &[&query_bgl],
            push_constant_ranges: &[],
        });
        let query_pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("foil2_query"),
            layout: Some(&query_layout),
            module: &shader,
            entry_point: "query",
            compilation_options: Default::default(),
            cache: None,
        });
        let bodies = Buf { buf: storage(&device, "foil2_bodies", 64, wgpu::BufferUsages::empty()), cap: 1 };
        let proxies = Buf { buf: storage(&device, "foil2_proxies", 64, wgpu::BufferUsages::empty()), cap: 1 };
        let heads = Buf { buf: storage(&device, "foil2_heads", 64, wgpu::BufferUsages::empty()), cap: 1 };
        let inputs = Buf { buf: storage(&device, "foil2_inputs", 64, wgpu::BufferUsages::empty()), cap: 1 };
        let hits = Buf { buf: storage(&device, "foil2_hits", 64, wgpu::BufferUsages::COPY_SRC), cap: 1 };
        let hit_count = storage(&device, "foil2_hit_count", 16, wgpu::BufferUsages::COPY_SRC);
        let staging = Buf {
            buf: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("foil2_staging"),
                size: 64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            cap: 64,
        };
        Ok(Self {
            device,
            queue,
            query_pipe,
            query_bgl,
            uniform,
            bodies,
            proxies,
            heads,
            inputs,
            hit_count,
            hits,
            staging,
            body_scratch: Vec::new(),
            proxy_scratch: Vec::new(),
            head_scratch: Vec::new(),
            input_scratch: Vec::new(),
            ts: None,
            detail: false,
            timing: GpuTiming::default(),
            cache_pose: 0,
            cache_grid: 0,
            cache_bodies: 0,
            cache_proxies: 0,
            cache_heads: 0,
        })
    }

    /// 打开之后，每次 `query` 才读显卡时间戳，累加到 `detect_ns`。关掉就不插时间戳。
    pub(crate) fn set_timing(&mut self, on: bool) {
        self.detail = on;
        if on {
            self.ensure_timestamps();
        }
    }

    #[allow(dead_code)]
    pub(crate) fn reset_timing(&mut self) {
        self.timing = GpuTiming::default();
    }

    pub(crate) fn detect_ns(&self) -> u64 {
        self.timing.detect_ns
    }

    fn ensure_timestamps(&mut self) {
        if self.ts.is_some() || !self.device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return;
        }
        let period = self.queue.get_timestamp_period();
        self.ts = Some(Ts {
            set: self.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("foil2_ts"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            buf: self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("foil2_ts"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            stage: self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("foil2_ts_stage"),
                size: 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            period,
        });
    }

    /// 用当前格子查重叠。先把散落的格子收进密铺表。
    ///
    /// 表按物体围成的矩形长大，不再卡 512×512。矩形的下标放不进 u32、内存申请失败，
    /// 或显存装不下整张表时返回 false，不写出部分结果。调用方改走 [`Foil::query`]。
    pub(crate) fn query(&mut self, world: &mut Foil, indices: &[usize], out: &mut Vec<(usize, usize)>) -> bool {
        world.absorb_extra_for_gpu();
        if indices.is_empty() {
            return true;
        }
        if !world.extra.is_empty() {
            return false;
        }
        self.input_scratch.clear();
        for &idx in indices {
            let Some(slot) = world.slot(idx) else {
                continue;
            };
            self.input_scratch.push(GpuMover {
                slot: slot as u32,
                dx: 0.0,
                dy: 0.0,
                _pad: 0,
            });
        }
        if self.input_scratch.is_empty() {
            return true;
        }
        self.dispatch_query(world, out)
    }

    fn dispatch_query(&mut self, world: &Foil, out: &mut Vec<(usize, usize)>) -> bool {
        let n = self.input_scratch.len();
        let mut cap = n.saturating_mul(16).max(256);
        let device = Arc::clone(&self.device);
        for _ in 0..2 {
            if self.upload_world(world, n, cap as u32).is_none() {
                return false;
            }
            if !Self::ensure_copy_dst(&device, &mut self.hits, cap, std::mem::size_of::<GpuHit>(), "foil2_hits") {
                return false;
            }
            self.queue.write_buffer(&self.hit_count, 0, &[0u8; 4]);
            let hit_bytes = (cap * std::mem::size_of::<GpuHit>()) as u64;
            let total = 256 + hit_bytes;
            if !self.ensure_staging(total) {
                return false;
            }
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("foil2_query_bg"),
                layout: &self.query_bgl,
                entries: &[
                    bind_uniform(0, &self.uniform),
                    bind_all(1, &self.bodies.buf),
                    bind_all(2, &self.proxies.buf),
                    bind_all(3, &self.heads.buf),
                    bind_all(4, &self.inputs.buf),
                    bind_all(6, &self.hit_count),
                    bind_all(7, &self.hits.buf),
                ],
            });
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("foil2_query") });
            {
                let writes = self.timestamp_writes();
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("foil2_query"),
                    timestamp_writes: writes,
                });
                pass.set_pipeline(&self.query_pipe);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(groups(n), 1, 1);
            }
            let read_ts = self.detail && self.ts.is_some();
            if read_ts {
                let ts = self.ts.as_ref().unwrap();
                encoder.resolve_query_set(&ts.set, 0..2, &ts.buf, 0);
                encoder.copy_buffer_to_buffer(&ts.buf, 0, &ts.stage, 0, 16);
            }
            encoder.copy_buffer_to_buffer(&self.hit_count, 0, &self.staging.buf, 0, 4);
            encoder.copy_buffer_to_buffer(&self.hits.buf, 0, &self.staging.buf, 256, hit_bytes);
            self.queue.submit(std::iter::once(encoder.finish()));

            let slice = self.staging.buf.slice(0..total);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            let ts_rx = if read_ts {
                let (tx2, rx2) = mpsc::channel();
                self.ts.as_ref().unwrap().stage.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    let _ = tx2.send(r);
                });
                Some((rx2, self.ts.as_ref().unwrap().period))
            } else {
                None
            };
            self.device.poll(wgpu::Maintain::Wait);
            if rx.recv().ok().and_then(|r| r.ok()).is_none() {
                return false;
            }
            if let Some((rx2, period)) = ts_rx {
                if let Some(ns) = read_mapped_ns(rx2, &self.ts.as_ref().unwrap().stage, period) {
                    self.timing.detect_ns += ns;
                }
            }
            let count = {
                let data = slice.get_mapped_range();
                let count = u32::from_le_bytes(data[0..4].try_into().unwrap_or([0; 4])) as usize;
                if count <= cap && count > 0 {
                    let start = 256;
                    let nbytes = count * std::mem::size_of::<GpuHit>();
                    let hits: &[GpuHit] = bytemuck::cast_slice(&data[start..start + nbytes]);
                    out.extend(hits.iter().map(|h| (h.me as usize, h.other as usize)));
                }
                count
            };
            self.staging.buf.unmap();
            if count > cap {
                cap = count;
                continue;
            }
            return true;
        }
        false
    }


    fn upload_world(&mut self, world: &Foil, n_in: usize, hit_cap: u32) -> Option<()> {
        if world.gw <= 0 || world.gh <= 0 || world.heads.is_empty() {
            return None;
        }
        let proxy_count = world.proxies.len().max(1) as u32;
        let head_count = (world.heads.len() * 2) as u32;
        let same_pose = self.cache_pose == world.pose_gen && self.cache_bodies == world.bodies.len() as u32;
        let same_grid = self.cache_grid == world.grid_gen
            && self.cache_proxies == proxy_count
            && self.cache_heads == head_count
            && world.extra.is_empty();
        if !same_pose {
            self.body_scratch.clear();
            self.body_scratch.reserve(world.bodies.len());
            for b in &world.bodies {
                self.body_scratch.push(GpuBody {
                    x: b.x as f32,
                    y: b.y as f32,
                    ox: b.ox as f32,
                    oy: b.oy as f32,
                    hw: b.hw as f32,
                    hh: b.hh as f32,
                    group: b.group,
                    mask: b.mask,
                    kind: b.kind as u32,
                    sensor: b.sensor as u32,
                    id: b.id as u32,
                    alive: b.alive as u32,
                });
            }
        }
        if !same_grid {
            self.proxy_scratch.clear();
            self.proxy_scratch.reserve(world.proxies.len().max(1));
            if world.proxies.is_empty() {
                self.proxy_scratch.push(GpuProxy { slot: 0, next: u32::MAX, _p0: 0, _p1: 0 });
            } else {
                for p in &world.proxies {
                    self.proxy_scratch.push(GpuProxy { slot: p.slot, next: p.bucket_next, _p0: 0, _p1: 0 });
                }
            }
            self.head_scratch.clear();
            self.head_scratch.reserve(world.heads.len() * 2);
            for cell in &world.heads {
                self.head_scratch.push(cell.lists[0]);
                self.head_scratch.push(cell.lists[1]);
            }
            if self.head_scratch.is_empty() {
                return None;
            }
        }
        let n_body = if same_pose { world.bodies.len() as u32 } else { self.body_scratch.len() as u32 };
        let n_proxy = if same_grid { proxy_count } else { self.proxy_scratch.len() as u32 };
        let grid = GpuGrid {
            ox: world.ox,
            oy: world.oy,
            gw: world.gw,
            gh: world.gh,
            n_body,
            n_proxy,
            n_in: n_in as u32,
            hit_cap,
        };
        let device = Arc::clone(&self.device);
        if !same_pose {
            if !Self::ensure_storage(&device, &mut self.bodies, self.body_scratch.len(), std::mem::size_of::<GpuBody>(), "foil2_bodies") {
                return None;
            }
            self.queue.write_buffer(&self.bodies.buf, 0, bytemuck::cast_slice(&self.body_scratch));
        }
        if !same_grid {
            if !Self::ensure_storage(&device, &mut self.proxies, self.proxy_scratch.len(), std::mem::size_of::<GpuProxy>(), "foil2_proxies")
                || !Self::ensure_storage(&device, &mut self.heads, self.head_scratch.len(), std::mem::size_of::<u32>(), "foil2_heads")
            {
                return None;
            }
            self.queue.write_buffer(&self.proxies.buf, 0, bytemuck::cast_slice(&self.proxy_scratch));
            self.queue.write_buffer(&self.heads.buf, 0, bytemuck::cast_slice(&self.head_scratch));
        }
        if !Self::ensure_storage(&device, &mut self.inputs, n_in.max(1), std::mem::size_of::<GpuMover>(), "foil2_inputs") {
            return None;
        }
        self.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&grid));
        self.queue.write_buffer(&self.inputs.buf, 0, bytemuck::cast_slice(&self.input_scratch));
        if !same_pose {
            self.cache_pose = world.pose_gen;
            self.cache_bodies = world.bodies.len() as u32;
        }
        if !same_grid {
            self.cache_grid = world.grid_gen;
            self.cache_proxies = self.proxy_scratch.len() as u32;
            self.cache_heads = self.head_scratch.len() as u32;
        }
        Some(())
    }

    fn timestamp_writes(&self) -> Option<wgpu::ComputePassTimestampWrites> {
        if !self.detail {
            return None;
        }
        self.ts.as_ref().map(|ts| wgpu::ComputePassTimestampWrites {
            query_set: &ts.set,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        })
    }

    fn fits(device: &wgpu::Device, bytes: u64, storage_binding: bool) -> bool {
        let limits = device.limits();
        bytes <= limits.max_buffer_size
            && (!storage_binding || bytes <= limits.max_storage_buffer_binding_size as u64)
    }

    fn ensure_storage(device: &wgpu::Device, slot: &mut Buf, count: usize, stride: usize, label: &str) -> bool {
        if slot.cap >= count {
            return true;
        }
        let cap = count.max(slot.cap.saturating_mul(2)).max(64);
        let bytes = (cap as u64).saturating_mul(stride as u64);
        if !Self::fits(device, bytes, true) {
            return false;
        }
        slot.buf = storage(device, label, bytes, wgpu::BufferUsages::empty());
        slot.cap = cap;
        true
    }

    fn ensure_copy_dst(device: &wgpu::Device, slot: &mut Buf, count: usize, stride: usize, label: &str) -> bool {
        if slot.cap >= count {
            return true;
        }
        let cap = count.max(slot.cap.saturating_mul(2)).max(64);
        let bytes = (cap as u64).saturating_mul(stride as u64);
        if !Self::fits(device, bytes, true) {
            return false;
        }
        slot.buf = storage(device, label, bytes, wgpu::BufferUsages::COPY_SRC);
        slot.cap = cap;
        true
    }

    fn ensure_staging(&mut self, bytes: u64) -> bool {
        let need = bytes.max(16);
        if self.staging.cap as u64 >= need {
            return true;
        }
        let cap = need.max(self.staging.cap as u64 * 2);
        if !Self::fits(&self.device, cap, false) {
            return false;
        }
        self.staging.buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("foil2_staging"),
            size: cap,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.staging.cap = cap as usize;
        true
    }
}

fn read_mapped_ns(rx: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>, stage: &wgpu::Buffer, period: f32) -> Option<u64> {
    rx.recv().ok()?.ok()?;
    let (start, end) = {
        let data = stage.slice(..).get_mapped_range();
        let start = u64::from_le_bytes(data[0..8].try_into().unwrap_or([0; 8]));
        let end = u64::from_le_bytes(data[8..16].try_into().unwrap_or([0; 8]));
        (start, end)
    };
    stage.unmap();
    let ticks = end.saturating_sub(start);
    Some((ticks as f64 * period as f64) as u64)
}

fn groups(n: usize) -> u32 {
    ((n.max(1) + 63) / 64) as u32
}

fn bind_uniform(binding: u32, buf: &wgpu::Buffer) -> wgpu::BindGroupEntry {
    wgpu::BindGroupEntry {
        binding,
        resource: buf.as_entire_binding(),
    }
}

/// 按名字开一块计算设备。`dx12`、`vulkan`、`metal`、`gl`。
pub(crate) fn open_device(api: &str) -> Result<(Arc<wgpu::Device>, Arc<wgpu::Queue>, String), String> {
    let key = api.trim().to_ascii_lowercase();
    let backends = match key.as_str() {
        "vulkan" => wgpu::Backends::VULKAN,
        "dx12" | "d3d12" | "directx12" => wgpu::Backends::DX12,
        "gl" | "opengl" | "gles" | "angle" => wgpu::Backends::GL,
        "metal" => wgpu::Backends::METAL,
        "wgpu" | "webgpu" | "gpu" => wgpu::Backends::PRIMARY,
        _ => {
            return Err(format!(
                "后端写 cpu、dx12、vulkan、metal 或 gl，不认识 {api}"
            ))
        }
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    });
    let adapter = instance
        .enumerate_adapters(backends)
        .into_iter()
        .next()
        .ok_or_else(|| format!("没有可用的 GPU 适配器（{api}）"))?;
    let info = adapter.get_info();
    let features = if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        wgpu::Features::TIMESTAMP_QUERY
    } else {
        wgpu::Features::empty()
    };
    let supported = adapter.limits();
    let mut limits = wgpu::Limits::default();
    limits.max_buffer_size = supported.max_buffer_size;
    limits.max_storage_buffer_binding_size = supported.max_storage_buffer_binding_size;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("foil"),
            required_features: features,
            required_limits: limits,
            memory_hints: Default::default(),
        },
        None,
    ))
    .map_err(|e| e.to_string())?;
    Ok((
        Arc::new(device),
        Arc::new(queue),
        format!("{:?} {}", info.backend, info.name),
    ))
}

fn bind_all(binding: u32, buf: &wgpu::Buffer) -> wgpu::BindGroupEntry {
    wgpu::BindGroupEntry {
        binding,
        resource: buf.as_entire_binding(),
    }
}

#[cfg(test)]
mod pressure {
    use std::sync::Arc;
    use std::time::Instant;

    use super::super::Foil;
    use super::FoilGpu;

    const N: i32 = 100;
    const M: i32 = 120;
    const BODIES: usize = (N * M) as usize;
    const SIZE: f32 = 48.0;
    const STEP: f32 = 32.0;
    const WARMUP: usize = 10;
    const FRAMES: usize = 40;

    fn place(world: &mut Foil, idx: usize, x: f32, y: f32) {
        world.add(idx, x as f64, y as f64, SIZE as f64, SIZE as f64, "rect", false, 0, 0).unwrap();
    }

    fn build() -> (Foil, Vec<usize>) {
        let mut world = Foil::new();
        let mut id = 0usize;
        let origin_x = -(N as f32) * STEP * 0.5;
        let origin_y = -(M as f32) * STEP * 0.5;
        for edge in 0..N {
            let x = origin_x + edge as f32 * STEP;
            place(&mut world, id, x, origin_y - STEP);
            id += 1;
            place(&mut world, id, x, origin_y + M as f32 * STEP);
            id += 1;
        }
        for edge in 0..M {
            let y = origin_y + edge as f32 * STEP;
            place(&mut world, id, origin_x - STEP, y);
            id += 1;
            place(&mut world, id, origin_x + N as f32 * STEP, y);
            id += 1;
        }
        let walls = id;
        let mut movers = Vec::with_capacity(BODIES);
        for r in 0..M {
            for c in 0..N {
                let x = origin_x + c as f32 * STEP;
                let y = origin_y + r as f32 * STEP;
                place(&mut world, id, x, y);
                movers.push(id);
                id += 1;
            }
        }
        assert_eq!(movers.len(), BODIES);
        assert!(walls + movers.len() > 10_000, "walls {walls} movers {}", movers.len());
        (world, movers)
    }

    fn open_gpu() -> Option<FoilGpu> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..Default::default()
        });
        let adapter = instance.enumerate_adapters(wgpu::Backends::DX12).into_iter().next()?;
        let features = if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: features,
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
        ))
        .ok()?;
        FoilGpu::new(Arc::new(device), Arc::new(queue)).ok()
    }

    #[test]
    #[ignore = "上万碰撞体，release 下单独跑"]
    fn pressure_foil2() {
        {
            let (mut world, movers) = build();
            let mut cpu = Vec::new();
            world.query_many(&movers, &mut cpu);
            let mut gpu = open_gpu().expect("需要一块 DX12 显卡");
            let mut got = Vec::new();
            assert!(gpu.query(&mut world, &movers, &mut got), "显卡查询没接上");
            cpu.sort_unstable();
            got.sort_unstable();
            assert_eq!(cpu.len(), got.len(), "对数不一致");
            for (i, (a, b)) in cpu.iter().zip(got.iter()).enumerate() {
                if a != b {
                    panic!("第 {i} 对不一致 cpu={a:?} gpu={b:?}");
                }
            }
        }
        let (world, movers) = build();
        let walls = world.bodies.iter().filter(|b| b.alive != 0 && b.sensor == 0).count() - movers.len();
        let mut query_ns = 0u128;
        let mut pairs_sum = 0usize;
        for frame in 0..(WARMUP + FRAMES) {
            let t = Instant::now();
            let mut pairs = Vec::new();
            world.query_many(&movers, &mut pairs);
            if frame >= WARMUP {
                query_ns += t.elapsed().as_nanos();
                pairs_sum += pairs.len();
            }
        }
        let frames = FRAMES as f64;
        let cpu_query = query_ns as f64 / frames / 1e6;

        let (mut world, movers) = build();
        let mut gpu = open_gpu().expect("需要一块 DX12 显卡");
        gpu.set_timing(true);
        let mut gquery_ns = 0u128;
        let mut gpairs = 0usize;
        for frame in 0..(WARMUP + FRAMES) {
            if frame == WARMUP {
                gpu.reset_timing();
            }
            let t = Instant::now();
            let mut pairs = Vec::new();
            assert!(gpu.query(&mut world, &movers, &mut pairs));
            if frame >= WARMUP {
                gquery_ns += t.elapsed().as_nanos();
                gpairs += pairs.len();
            }
        }
        let detect = gpu.detect_ns() as f64 / frames / 1e6;
        println!(
            "[pressure] movers={BODIES} walls={walls} total={} size={SIZE} spacing={STEP} frames={FRAMES}",
            walls + BODIES
        );
        println!(
            "[cpu] pairs {:.0}  query {:.2} ms",
            pairs_sum as f64 / frames,
            cpu_query
        );
        println!(
            "[gpu] pairs {:.0}  query {:.2} ms  detect {:.2} ms",
            gpairs as f64 / frames,
            gquery_ns as f64 / frames / 1e6,
            detect
        );
    }
}

