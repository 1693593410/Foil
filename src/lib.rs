#![allow(non_snake_case)]

//! 轻量级 2D 游戏物理引擎，以速度、轻量、可玩性为目标，不追求复杂的物理系统。
//! 目前完成了轴对齐矩形和圆的重叠检测、传感器、分组和掩码。
//! 物体放好就停在那里。调用方用 `set_pose` 改位置，再用重叠查询做检测。
//!
//! `mask == 0` 或对方 `group == 0` 都会碰上。0 不是「谁都不碰」。
//!
//! 箱子名是 `Foil`，类型也是 `Foil`：`use Foil::Foil`。
//! `Foil::new` 是 CPU、f32。`Foil::open("dx12")` 把 `query` 放到显卡上。`gpu` 是默认功能。
//! GPU 还在试验，传输开销大于收益，暂时不推荐。
//! `Foil::open_with("cpu", "f64")` 用双精度，只走 CPU。
//! `set_threads` 限制批量查询的线程数，默认 16，`0` 不封顶。少于 192 个物体仍是单线程。

mod world;

pub use world::{BodyView, Foil, SpecError};
