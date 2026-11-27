//! Python 接口。`import Foil` 之后用 `Foil.Foil`。

use ::Foil::Foil;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

/// 二维重叠检测。调用方用 `set_pose` 摆位置，用 `overlaps` 或 `query` 取重叠结果。
/// 边贴着不算重叠。`mask == 0` 会碰上所有人，对方 `group == 0` 也会碰上。
#[pyclass(name = "Foil", module = "Foil")]
pub struct FoilPy {
    world: Foil,
}

#[pymethods]
impl FoilPy {
    /// 创建世界。`backend` 可以是 cpu、dx12、vulkan、metal、gl，以及 wgpu。
    /// `real` 写 f32 或 f64，f64 只能配 cpu。
    /// `threads` 是批量查询的线程数，写多少用多少，0 表示用进程里的线程池。少于 192 个物体时仍是单线程。
    #[new]
    #[pyo3(signature = (backend="cpu", real="f32", threads=16))]
    fn new(backend: &str, real: &str, threads: usize) -> PyResult<Self> {
        let mut world = Foil::open_with(backend, real).map_err(PyRuntimeError::new_err)?;
        world.set_threads(threads);
        Ok(Self { world })
    }

    /// 创建时选定的后端名字。
    fn backend(&self) -> String {
        self.world.backend().to_string()
    }

    /// `"f32"` 或 `"f64"`。
    fn real(&self) -> &'static str {
        self.world.real()
    }

    /// 当前线程上限。0 表示用进程里的线程池。少于 192 个物体时查询仍是单线程。
    fn threads(&self) -> usize {
        self.world.threads()
    }

    /// 改线程上限。1 始终单线程。下次批量查询才按新的数量建池。
    fn set_threads(&mut self, n: usize) {
        self.world.set_threads(n);
    }

    /// 打开后，`query` 才累计显卡检测耗时。CPU 后端没有这个数。
    fn set_timing(&mut self, on: bool) {
        self.world.set_timing(on);
    }

    /// 历次显卡检测耗时合计，单位纳秒。
    fn detect_ns(&self) -> u64 {
        self.world.detect_ns()
    }

    /// 放进一个物体。`x` `y` 是中心，`w` `h` 是宽高。`shape` 只接受 rect 和 circle。
    /// 圆的半径是宽高里较大的一半。形状写错会抛出 ValueError。
    /// `mask` 为 0 碰上所有人，对方 `group` 为 0 也碰上。
    #[pyo3(signature = (idx, x, y, w, h, shape="rect", sensor=false, group=0, mask=0))]
    fn add(
        &mut self,
        idx: usize,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        shape: &str,
        sensor: bool,
        group: u32,
        mask: u32,
    ) -> PyResult<()> {
        self.world
            .add(idx, x, y, w, h, shape, sensor, group, mask)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// 按编号删掉。编号不在世界里时什么也不做。
    fn remove(&mut self, idx: usize) {
        self.world.remove(idx);
    }

    /// 清空全部物体和格子。
    fn clear(&mut self) {
        self.world.clear();
    }

    /// 改中心。不存在的编号会被忽略。
    fn set_pose(&mut self, idx: usize, x: f64, y: f64) {
        self.world.set_pose(idx, x, y);
    }

    /// 沿位移贴着别的实体滑，并写进新中心。先水平，再竖直。
    /// 编号不存在时返回 (0, 0)。
    fn move_by(&mut self, idx: usize, dx: f64, dy: f64) -> (f64, f64) {
        self.world.move_by(idx, dx, dy)
    }

    /// 读中心。编号不存在时返回 None。
    fn pose(&self, idx: usize) -> Option<(f64, f64)> {
        self.world.pose(idx)
    }

    /// 改宽高。`shape` 为空字符串时沿用现在的形状。形状名写错才抛出 ValueError。
    #[pyo3(signature = (idx, w, h, shape=""))]
    fn set_size(&mut self, idx: usize, w: f64, h: f64, shape: &str) -> PyResult<()> {
        self.world
            .set_size(idx, shape, w, h)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// 读宽高。编号不存在时返回 None。
    fn size(&self, idx: usize) -> Option<(f64, f64)> {
        self.world.size(idx)
    }

    /// 返回 `"rect"` 或 `"circle"`。编号不存在时返回 None。
    fn shape(&self, idx: usize) -> Option<&str> {
        self.world.shape(idx)
    }

    /// 改自己的分组位。
    fn set_group(&mut self, idx: usize, group: u32) {
        self.world.set_group(idx, group);
    }

    /// 读分组位。编号不存在时返回 None。
    fn group(&self, idx: usize) -> Option<u32> {
        self.world.group(idx)
    }

    /// 改掩码。0 表示碰上所有人。
    fn set_mask(&mut self, idx: usize, mask: u32) {
        self.world.set_mask(idx, mask);
    }

    /// 读掩码。编号不存在时返回 None。
    fn mask(&self, idx: usize) -> Option<u32> {
        self.world.mask(idx)
    }

    /// 改成传感器或实体。传感器互相看不见，实体仍能看见传感器。
    fn set_sensor(&mut self, idx: usize, sensor: bool) {
        self.world.set_sensor(idx, sensor);
    }

    /// 是否为传感器。编号不存在时返回 False。
    fn is_sensor(&self, idx: usize) -> bool {
        self.world.is_sensor(idx)
    }

    /// 还在世界里的物体个数，含传感器。
    fn count(&self) -> usize {
        self.world.count()
    }

    /// 实体个数。传感器不算。
    fn len(&self) -> usize {
        self.world.len()
    }

    /// 形状中心相对 `set_pose` 的偏移。
    fn set_offset(&mut self, idx: usize, ox: f64, oy: f64) {
        self.world.set_offset(idx, ox, oy);
    }

    /// 用存着的位置，返回重叠到的编号。边贴着不算。传感器不会列出别的传感器。
    fn overlaps(&self, idx: usize) -> Vec<usize> {
        self.world.overlaps(idx)
    }

    /// 一次查很多个编号。每一对是 (被查询的编号, 对方)。两边都在名单里时，两个方向都留。
    fn query(&mut self, indices: Vec<usize>) -> Vec<(usize, usize)> {
        self.world.query(&indices)
    }
}

#[pymodule]
#[pyo3(name = "Foil")]
fn foil(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<FoilPy>()?;
    Ok(())
}
