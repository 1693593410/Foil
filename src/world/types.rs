//! 物体、代理、格子头和错误。

pub(super) const CELL: f32 = 32.0;
pub(super) const INV_CELL: f32 = 1.0 / CELL;
pub(super) const INV_CELL_F64: f64 = 1.0 / 32.0;
pub(super) const NONE: u32 = u32::MAX;

pub(super) const KIND_RECT: u8 = 0;
pub(super) const KIND_CIRCLE: u8 = 1;

#[derive(Clone, Copy)]
pub(super) struct Body {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) ox: f32,
    pub(super) oy: f32,
    pub(super) hw: f32,
    pub(super) hh: f32,
    pub(super) group: u32,
    pub(super) mask: u32,
    pub(super) kind: u8,
    pub(super) sensor: u8,
    pub(super) linked: u8,
    /// 0 表示这个槽已经空出。编号本身可以是任何 32 位值，包括 `u32::MAX`。
    pub(super) alive: u8,
    pub(super) c0: i32,
    pub(super) r0: i32,
    pub(super) c1: i32,
    pub(super) r1: i32,
    pub(super) proxy: u32,
    pub(super) id: usize,
}

#[derive(Clone, Copy)]
pub(super) struct Wide {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) ox: f64,
    pub(super) oy: f64,
    pub(super) hw: f64,
    pub(super) hh: f64,
}

impl Wide {
    pub(super) const ZERO: Self = Self { x: 0.0, y: 0.0, ox: 0.0, oy: 0.0, hw: 0.0, hh: 0.0 };
}

#[derive(Clone, Copy)]
pub(super) struct Proxy {
    pub(super) slot: u32,
    pub(super) cx: i32,
    pub(super) cy: i32,
    pub(super) bucket_next: u32,
    pub(super) owned_next: u32,
    /// 0 实体链，1 sensor 链。卸下时要知道从哪条链摘。
    pub(super) list: u8,
}

#[derive(Clone, Copy)]
pub(super) struct Planned {
    pub(super) slot: u32,
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) c0: i32,
    pub(super) r0: i32,
    pub(super) c1: i32,
    pub(super) r1: i32,
    /// 0 坐标没变，1 只改坐标，2 格子也要重挂。
    pub(super) kind: u8,
}

pub(super) struct MoveScratch {
    pub(super) plans: Vec<Planned>,
    pub(super) seen: Vec<u32>,
    pub(super) seen_gen: u32,
}

impl Default for MoveScratch {
    fn default() -> Self {
        Self {
            plans: Vec::new(),
            seen: Vec::new(),
            seen_gen: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct CellHeads {
    pub(super) lists: [u32; 2],
}

impl CellHeads {
    pub(super) const EMPTY: Self = Self {
        lists: [NONE, NONE],
    };
}

/// 形状或体型不是库认的名字。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecError {
    /// 只接受 `rect` 和 `circle`。
    Shape,
    /// 编号要能放进 32 位。`4294967295` 可以用。
    Id,
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpecError::Shape => f.write_str("形状只接受 rect 或 circle"),
            SpecError::Id => f.write_str("编号要能放进 32 位，4294967295 可以用"),
        }
    }
}

impl std::error::Error for SpecError {}

/// 一次遍历里看到的物体。
#[derive(Clone, Copy, Debug)]
pub struct BodyView {
    pub id: usize,
    pub x: f64,
    pub y: f64,
    pub is_sensor: bool,
}
