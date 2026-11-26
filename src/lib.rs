#![allow(non_snake_case)]

//! 轻量级 2D 游戏物理引擎，以性能、轻量、简单、可玩性为目标，不追求复杂的物理系统。
//! 目前仅完成轴对齐矩形和圆的重叠检测、传感器、分组和掩码。采用 MIT 协议。
//!
//! 物体放好就停在那里。调用方用 [`Foil::set_pose`] 摆位置，用 [`Foil::overlaps`] 或 [`Foil::query`] 取重叠结果。
//! 边贴着不算重叠。`mask == 0` 会碰上所有人，对方 `group == 0` 也会碰上。0 不是「谁都不碰」。
//!
//! ```toml
//! rustfoil2d = "0.1.0"
//! ```
//!
//! ```
//! use Foil::Foil;
//! let world = Foil::new();
//! ```
//!
//! [`Foil::new`] 是 CPU、f32，批量查询线程上限 16。
//! [`Foil::open`] 可以选 `dx12`（也认 `d3d12`、`directx12`）、`vulkan`、`metal`、`gl`（也认 `opengl`、`gles`、`angle`），以及 `wgpu`、`webgpu`、`gpu`。
//! 显卡查询是实验功能，传输开销目前大于收益。
//! [`Foil::open_with`] 的精度写 `f32` 或 `f64`。`f64` 只能配 `cpu`。
//! [`Foil::set_threads`] 写多少用多少。`0` 表示用进程里现成的线程池。被查物体少于 192 个时始终单线程。
//!
//! 依赖名是 `rustfoil2d`。箱子名是 `Foil`，类型也是 `Foil`：`use Foil::Foil`。

mod world;

pub use world::{BodyView, Foil, SpecError};
