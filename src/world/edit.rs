//! 放入、挪走、改尺寸和分组。

use super::grid::{cell_range, cell_span_f32};
use super::types::*;
use super::Foil;

impl Foil {
    /// 改中心。不存在的编号会被忽略。
    pub fn set_pose(&mut self, idx: usize, x: f64, y: f64) {
        let Some(slot) = self.slot(idx) else {
            return;
        };
        let x = self.fit(x);
        let y = self.fit(y);
        if self.wide {
            let w = &mut self.wide_pose[slot];
            w.x = x;
            w.y = y;
        }
        let b = &mut self.bodies[slot];
        b.x = x as f32;
        b.y = y as f32;
        self.touch_pose();
        self.link_if_changed(slot);
        self.refresh_solid(slot);
    }

    /// 读中心。编号不存在时返回 `None`。
    pub fn pose(&self, idx: usize) -> Option<(f64, f64)> {
        self.slot(idx).map(|s| self.pose_at(s))
    }

    pub(super) fn pose_at(&self, slot: usize) -> (f64, f64) {
        if self.wide {
            let w = &self.wide_pose[slot];
            (w.x, w.y)
        } else {
            let b = &self.bodies[slot];
            (b.x as f64, b.y as f64)
        }
    }

    pub(super) fn span(&self, slot: usize) -> (i32, i32, i32, i32) {
        let b = &self.bodies[slot];
        if self.wide {
            let w = &self.wide_pose[slot];
            cell_range(w.x, w.y, w.ox, w.oy, w.hw, w.hh, b.kind, true)
        } else {
            cell_span_f32(b.x, b.y, b.ox, b.oy, b.hw, b.hh, b.kind)
        }
    }

    pub(super) fn wide_at(&self, slot: usize) -> Wide {
        if self.wide {
            self.wide_pose[slot]
        } else {
            let b = &self.bodies[slot];
            Wide {
                x: b.x as f64,
                y: b.y as f64,
                ox: b.ox as f64,
                oy: b.oy as f64,
                hw: b.hw as f64,
                hh: b.hh as f64,
            }
        }
    }

    pub(super) fn place(
        &mut self,
        idx: usize,
        x: f64,
        y: f64,
        hw: f64,
        hh: f64,
        kind: u8,
        sensor: bool,
        group: u32,
        mask: u32,
        ox: f64,
        oy: f64,
    ) {
        let slot = self.ensure_slot(idx);
        self.retire(slot);
        let sensor: u8 = if sensor { 1 } else { 0 };
        let x = self.fit(x);
        let y = self.fit(y);
        let ox = self.fit(ox);
        let oy = self.fit(oy);
        let hw = self.fit(hw);
        let hh = self.fit(hh);
        if self.wide {
            self.wide_pose[slot] = Wide { x, y, ox, oy, hw, hh };
        }
        self.bodies[slot] = Body {
            x: x as f32,
            y: y as f32,
            ox: ox as f32,
            oy: oy as f32,
            hw: hw as f32,
            hh: hh as f32,
            group,
            mask,
            kind,
            sensor,
            linked: 0,
            alive: 1,
            c0: 0,
            r0: 0,
            c1: 0,
            r1: 0,
            proxy: NONE,
            id: idx,
        };
        if sensor == 0 {
            self.solid_count += 1;
        }
        self.link(slot);
        self.touch_pose();
    }

    /// 放进一个物体。编号由调用方指定。`x` `y` 是中心，`w` `h` 是宽高。
    ///
    /// `shape` 只接受 `rect` 和 `circle`。圆的半径是宽高里较大的一半。
    /// 形状写错，或编号放不进 32 位，返回 [`SpecError`]，物体不会放进去。
    /// `sensor` 为真时只参与检测。`mask == 0` 碰上所有人，对方 `group == 0` 也碰上。
    /// 0 不是「谁都不碰」。物体放好就停在那里，库不推动它。
    pub fn add(
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
    ) -> Result<(), SpecError> {
        if idx > u32::MAX as usize {
            return Err(SpecError::Id);
        }
        let kind = parse_shape(shape)?;
        let (hw, hh) = match kind {
            KIND_CIRCLE => {
                let r = w.max(h) * 0.5;
                (r, r)
            }
            _ => (w * 0.5, h * 0.5),
        };
        self.place(idx, x, y, hw, hh, kind, sensor, group, mask, 0.0, 0.0);
        Ok(())
    }

    /// 按编号删掉。编号不在世界里时什么也不做。
    pub fn remove(&mut self, idx: usize) {
        let Some(slot) = self.slot(idx) else {
            return;
        };
        self.retire(slot);
        self.bodies[slot].alive = 0;
        self.touch_pose();
        if idx < self.index.len() {
            self.index[idx] = 0;
        }
        self.sparse.remove(&idx);
        self.free.push(slot as u32);
    }

    /// 清空全部物体和格子。
    pub fn clear(&mut self) {
        self.bodies.clear();
        self.index.clear();
        self.sparse.clear();
        self.wide_pose.clear();
        self.free.clear();
        self.proxies.clear();
        self.free_proxy = NONE;
        self.ox = 0;
        self.oy = 0;
        self.gw = 0;
        self.gh = 0;
        self.heads.clear();
        self.extra.clear();
        self.solid.clear();
        self.extra_solid.clear();
        self.solid_count = 0;
        self.touch_pose();
        self.touch_grid();
    }

    /// `shape` 为空则保持原形状。非空时只接受 `rect` 和 `circle`。
    pub fn set_size(&mut self, idx: usize, shape: &str, w: f64, h: f64) -> Result<(), SpecError> {
        let kind = if shape.is_empty() {
            None
        } else {
            Some(parse_shape(shape)?)
        };
        let Some(slot) = self.slot(idx) else {
            return Ok(());
        };
        let kind = kind.unwrap_or(self.bodies[slot].kind);
        let (hw, hh) = if kind == KIND_CIRCLE {
            let r = self.fit(w.max(h) * 0.5);
            (r, r)
        } else {
            (self.fit(w * 0.5), self.fit(h * 0.5))
        };
        if self.wide {
            let pose = &mut self.wide_pose[slot];
            pose.hw = hw;
            pose.hh = hh;
        }
        let b = &mut self.bodies[slot];
        b.kind = kind;
        b.hw = hw as f32;
        b.hh = hh as f32;
        self.touch_pose();
        self.link_if_changed(slot);
        self.refresh_solid(slot);
        Ok(())
    }

    /// 读宽高。编号不存在时返回 `None`。
    pub fn size(&self, idx: usize) -> Option<(f64, f64)> {
        self.slot(idx).map(|s| {
            let w = self.wide_at(s);
            (w.hw * 2.0, w.hh * 2.0)
        })
    }

    /// 返回 `"rect"` 或 `"circle"`。编号不存在时返回 `None`。
    pub fn shape(&self, idx: usize) -> Option<&'static str> {
        self.slot(idx).map(|s| {
            if self.bodies[s].kind == KIND_CIRCLE {
                "circle"
            } else {
                "rect"
            }
        })
    }

    /// 改自己的分组位。不存在的编号会被忽略。
    pub fn set_group(&mut self, idx: usize, group: u32) {
        if let Some(slot) = self.slot(idx) {
            self.bodies[slot].group = group;
            self.touch_pose();
            self.refresh_solid(slot);
        }
    }

    /// 没有这个编号时是 `None`。0 是真正的分组，不是「不存在」。
    pub fn group(&self, idx: usize) -> Option<u32> {
        self.slot(idx).map(|s| self.bodies[s].group)
    }

    /// 改掩码。0 表示碰上所有人。
    pub fn set_mask(&mut self, idx: usize, mask: u32) {
        if let Some(slot) = self.slot(idx) {
            self.bodies[slot].mask = mask;
            self.touch_pose();
            self.refresh_solid(slot);
        }
    }

    /// 没有这个编号时是 `None`。`0` 表示碰上所有人。
    pub fn mask(&self, idx: usize) -> Option<u32> {
        self.slot(idx).map(|s| self.bodies[s].mask)
    }

    /// 改成传感器或实体。传感器互相看不见，实体仍能看见传感器。
    pub fn set_sensor(&mut self, idx: usize, sensor: bool) {
        let Some(slot) = self.slot(idx) else {
            return;
        };
        let next: u8 = if sensor { 1 } else { 0 };
        if self.bodies[slot].sensor == next {
            return;
        }
        if self.bodies[slot].sensor == 0 {
            self.solid_count = self.solid_count.saturating_sub(1);
        }
        self.unlink(slot);
        self.bodies[slot].sensor = next;
        if next == 0 {
            self.solid_count += 1;
        }
        self.link(slot);
        self.touch_pose();
    }

    /// 形状中心相对 `set_pose` 的偏移。
    pub fn set_offset(&mut self, idx: usize, ox: f64, oy: f64) {
        let Some(slot) = self.slot(idx) else {
            return;
        };
        let ox = self.fit(ox);
        let oy = self.fit(oy);
        if self.wide {
            let w = &mut self.wide_pose[slot];
            w.ox = ox;
            w.oy = oy;
        }
        let b = &mut self.bodies[slot];
        b.ox = ox as f32;
        b.oy = oy as f32;
        self.touch_pose();
        self.link_if_changed(slot);
        self.refresh_solid(slot);
    }

    /// 实体个数。传感器不算。要连传感器一起数，用 [`Foil::count`]。
    pub fn len(&self) -> usize {
        self.solid_count
    }

    /// 还在世界里的物体，含传感器。
    pub fn count(&self) -> usize {
        self.bodies.iter().filter(|b| b.alive != 0).count()
    }

    /// 这个编号是否还在。
    pub fn contains(&self, idx: usize) -> bool {
        self.slot(idx).is_some()
    }

    /// 是否为传感器。编号不存在时返回 `false`。
    pub fn is_sensor(&self, idx: usize) -> bool {
        self.slot(idx).is_some_and(|s| self.bodies[s].sensor != 0)
    }

    /// 逐个交给回调。`BodyView` 有 `id`、`x`、`y`、`is_sensor`。
    pub fn each(&self, mut f: impl FnMut(BodyView)) {
        for (slot, b) in self.bodies.iter().enumerate() {
            if b.alive == 0 {
                continue;
            }
            let (x, y) = if self.wide {
                (self.wide_pose[slot].x, self.wide_pose[slot].y)
            } else {
                (b.x as f64, b.y as f64)
            };
            f(BodyView {
                id: b.id,
                x,
                y,
                is_sensor: b.sensor != 0,
            });
        }
    }

}

pub(super) fn parse_shape(shape: &str) -> Result<u8, SpecError> {
    match shape {
        "rect" => Ok(KIND_RECT),
        "circle" => Ok(KIND_CIRCLE),
        _ => Err(SpecError::Shape),
    }
}
