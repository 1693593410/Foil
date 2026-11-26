//! Python 接口。`import Foil` 之后用 `Foil.Foil`。

use ::Foil::Foil;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

#[pyclass(name = "Foil", module = "Foil")]
pub struct FoilPy {
    world: Foil,
}

#[pymethods]
impl FoilPy {
    #[new]
    #[pyo3(signature = (backend="cpu", real="f32", threads=16))]
    fn new(backend: &str, real: &str, threads: usize) -> PyResult<Self> {
        let mut world = Foil::open_with(backend, real).map_err(PyRuntimeError::new_err)?;
        world.set_threads(threads);
        Ok(Self { world })
    }

    fn backend(&self) -> String {
        self.world.backend().to_string()
    }

    fn real(&self) -> &'static str {
        self.world.real()
    }

    /// 查询最多开几个线程。`0` 不封顶。默认 16。
    fn threads(&self) -> usize {
        self.world.threads()
    }

    fn set_threads(&mut self, n: usize) {
        self.world.set_threads(n);
    }

    fn set_timing(&mut self, on: bool) {
        self.world.set_timing(on);
    }

    fn detect_ns(&self) -> u64 {
        self.world.detect_ns()
    }

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

    fn remove(&mut self, idx: usize) {
        self.world.remove(idx);
    }

    fn clear(&mut self) {
        self.world.clear();
    }

    fn set_pose(&mut self, idx: usize, x: f64, y: f64) {
        self.world.set_pose(idx, x, y);
    }

    fn pose(&self, idx: usize) -> Option<(f64, f64)> {
        self.world.pose(idx)
    }

    #[pyo3(signature = (idx, w, h, shape=""))]
    fn set_size(&mut self, idx: usize, w: f64, h: f64, shape: &str) -> PyResult<()> {
        self.world
            .set_size(idx, shape, w, h)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    fn size(&self, idx: usize) -> Option<(f64, f64)> {
        self.world.size(idx)
    }

    fn shape(&self, idx: usize) -> Option<&str> {
        self.world.shape(idx)
    }

    fn set_group(&mut self, idx: usize, group: u32) {
        self.world.set_group(idx, group);
    }

    fn group(&self, idx: usize) -> Option<u32> {
        self.world.group(idx)
    }

    fn set_mask(&mut self, idx: usize, mask: u32) {
        self.world.set_mask(idx, mask);
    }

    fn mask(&self, idx: usize) -> Option<u32> {
        self.world.mask(idx)
    }

    fn set_sensor(&mut self, idx: usize, sensor: bool) {
        self.world.set_sensor(idx, sensor);
    }

    fn is_sensor(&self, idx: usize) -> bool {
        self.world.is_sensor(idx)
    }

    fn count(&self) -> usize {
        self.world.count()
    }

    fn len(&self) -> usize {
        self.world.len()
    }

    fn set_offset(&mut self, idx: usize, ox: f64, oy: f64) {
        self.world.set_offset(idx, ox, oy);
    }

    fn overlaps(&self, idx: usize) -> Vec<usize> {
        self.world.overlaps(idx)
    }

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
