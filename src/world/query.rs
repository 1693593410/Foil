//! 重叠查询和形状测试。

use std::cell::RefCell;

use rayon::prelude::*;

use super::grid::{cell_of, cell_of_f32, cell_range, cell_span_f32};
use super::QUERY_PARALLEL_AT;
use super::types::*;
use super::Foil;

impl Foil {
    /// 用存着的位置，返回重叠到的编号。边贴着不算。传感器不会列出别的传感器。
    pub fn overlaps(&self, idx: usize) -> Vec<usize> {
        let Some((x, y)) = self.pose(idx) else {
            return Vec::new();
        };
        self.overlaps_at(idx, x, y)
    }

    /// 把 `idx` 临时放在 (x, y) 再查，不改它存着的坐标。
    pub fn overlaps_at(&self, idx: usize, x: f64, y: f64) -> Vec<usize> {
        let mut out = Vec::new();
        let Some(slot) = self.slot(idx) else {
            return out;
        };
        if self.bodies[slot].alive == 0 {
            return out;
        }
        self.visit(slot, self.fit(x), self.fit(y), false, Some(&mut out));
        out
    }

    /// 一次查很多个下标。物体够多时按段并行，每段自己一块代数表。
    /// 传感器在形状测试之前就跳过别的传感器。格子只读，贴墙改链仍是单线程。
    pub fn query_many(&self, indices: &[usize], out: &mut Vec<(usize, usize)>) {
        if indices.is_empty() || self.bodies.is_empty() {
            return;
        }
        if indices.len() < QUERY_PARALLEL_AT || self.thread_limit == 1 {
            self.query_many_serial(indices, out);
            return;
        }
        self.with_threads(|| self.query_parallel(indices, out));
    }

    pub(super) fn query_parallel(&self, indices: &[usize], out: &mut Vec<(usize, usize)>) {
        let threads = rayon::current_num_threads().max(1);
        let chunk = (indices.len() / threads).max(64);
        let mut parts: Vec<Vec<(usize, usize)>> = Vec::new();
        indices
            .par_chunks(chunk)
            .map(|chunk| {
                let mut local = Vec::new();
                self.query_many_serial(chunk, &mut local);
                local
            })
            .collect_into_vec(&mut parts);
        for part in parts {
            out.extend(part);
        }
    }


    pub(super) fn with_threads(&self, f: impl FnOnce() + Send) {
        if self.thread_limit == 0 {
            f();
            return;
        }
        let mut slot = self.thread_pool.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            match rayon::ThreadPoolBuilder::new().num_threads(self.thread_limit).build() {
                Ok(pool) => *slot = Some(pool),
                Err(_) => {
                    drop(slot);
                    f();
                    return;
                }
            }
        }
        slot.as_ref().unwrap().install(f);
    }

    pub(super) fn query_many_serial(&self, indices: &[usize], out: &mut Vec<(usize, usize)>) {
        if indices.is_empty() || self.bodies.is_empty() {
            return;
        }
        if self.wide {
            self.query_serial_wide(indices, out);
        } else {
            self.query_serial_f32(indices, out);
        }
    }

    pub(super) fn with_stamp(&self, mut body: impl FnMut(&mut [u32], &mut u32)) {
        let n = self.bodies.len();
        STAMP.with(|cell| {
            let st = &mut *cell.borrow_mut();
            if st.data.len() < n {
                st.data.resize(n, 0);
            }
            body(&mut st.data, &mut st.gen);
        });
    }

    pub(super) fn query_serial_f32(&self, indices: &[usize], out: &mut Vec<(usize, usize)>) {
        self.with_stamp(|stamp, gen_slot| {
            let mut gen = *gen_slot;
            let guard = self.proxies.len() as u32 + 1;
            for &idx in indices {
                let Some(slot) = self.slot(idx) else {
                    continue;
                };
                let me = &self.bodies[slot];
                if me.alive == 0 {
                    continue;
                }
                if gen == 0 {
                    stamp.fill(0);
                    gen = 1;
                }
                let me_sensor = me.sensor != 0;
                let (c0, r0, c1, r1) = cell_span_f32(me.x, me.y, me.ox, me.oy, me.hw, me.hh, me.kind);
                let lists = if me_sensor { 1 } else { 2 };
                for list in 0..lists {
                    let mut cy = r0;
                    loop {
                        let mut cx = c0;
                        loop {
                            let mut p = self.head_of(cx, cy, list);
                            let mut n = 0u32;
                            while p != NONE {
                                n += 1;
                                if n > guard {
                                    break;
                                }
                                let proxy = &self.proxies[p as usize];
                                let next = proxy.bucket_next;
                                let other_slot = proxy.slot as usize;
                                if other_slot != slot && other_slot < stamp.len() {
                                    let other = &self.bodies[other_slot];
                                    let skip = other.alive == 0
                                        || (me_sensor && other.sensor != 0)
                                        || stamp[other_slot] == gen;
                                    if !skip {
                                        stamp[other_slot] = gen;
                                        if groups_match(me, other) && shapes_f32(me, me.x, me.y, other, other.x, other.y) {
                                            out.push((me.id, other.id));
                                        }
                                    }
                                }
                                p = next;
                            }
                            if cx == c1 {
                                break;
                            }
                            cx += 1;
                        }
                        if cy == r1 {
                            break;
                        }
                        cy += 1;
                    }
                }
                gen = gen.wrapping_add(1);
            }
            *gen_slot = gen;
        });
    }

    pub(super) fn query_serial_wide(&self, indices: &[usize], out: &mut Vec<(usize, usize)>) {
        self.with_stamp(|stamp, gen_slot| {
            let mut gen = *gen_slot;
            let guard = self.proxies.len() as u32 + 1;
            for &idx in indices {
                let Some(slot) = self.slot(idx) else {
                    continue;
                };
                let me = &self.bodies[slot];
                if me.alive == 0 {
                    continue;
                }
                if gen == 0 {
                    stamp.fill(0);
                    gen = 1;
                }
                let mine = self.wide_pose[slot];
                let me_sensor = me.sensor != 0;
                let (c0, r0, c1, r1) = cell_range(mine.x, mine.y, mine.ox, mine.oy, mine.hw, mine.hh, me.kind, true);
                let lists = if me_sensor { 1 } else { 2 };
                for list in 0..lists {
                    let mut cy = r0;
                    loop {
                        let mut cx = c0;
                        loop {
                            let mut p = self.head_of(cx, cy, list);
                            let mut n = 0u32;
                            while p != NONE {
                                n += 1;
                                if n > guard {
                                    break;
                                }
                                let proxy = &self.proxies[p as usize];
                                let next = proxy.bucket_next;
                                let other_slot = proxy.slot as usize;
                                if other_slot != slot && other_slot < stamp.len() {
                                    let other = &self.bodies[other_slot];
                                    let skip = other.alive == 0
                                        || (me_sensor && other.sensor != 0)
                                        || stamp[other_slot] == gen;
                                    if !skip {
                                        stamp[other_slot] = gen;
                                        let theirs = self.wide_pose[other_slot];
                                        if groups_match(me, other)
                                            && shapes_pair(
                                                mine.x + mine.ox,
                                                mine.y + mine.oy,
                                                mine.hw,
                                                mine.hh,
                                                me.kind,
                                                theirs.x + theirs.ox,
                                                theirs.y + theirs.oy,
                                                theirs.hw,
                                                theirs.hh,
                                                other.kind,
                                            )
                                        {
                                            out.push((me.id, other.id));
                                        }
                                    }
                                }
                                p = next;
                            }
                            if cx == c1 {
                                break;
                            }
                            cx += 1;
                        }
                        if cy == r1 {
                            break;
                        }
                        cy += 1;
                    }
                }
                gen = gen.wrapping_add(1);
            }
            *gen_slot = gen;
        });
    }

    pub(super) fn visit(
        &self,
        slot: usize,
        x: f64,
        y: f64,
        stop: bool,
        mut out: Option<&mut Vec<usize>>,
    ) -> bool {
        let me = &self.bodies[slot];
        let (c0, r0, c1, r1) = if self.wide {
            let w = &self.wide_pose[slot];
            cell_range(x, y, w.ox, w.oy, w.hw, w.hh, me.kind, true)
        } else {
            cell_span_f32(x as f32, y as f32, me.ox, me.oy, me.hw, me.hh, me.kind)
        };
        let guard = self.proxies.len() as u32 + 1;
        // 挡住和 sensor 只走实体链。实体查询两条链都走。
        let lists = if stop || me.sensor != 0 { 1 } else { 2 };
        for list in 0..lists {
            let mut cy = r0;
            loop {
                let mut cx = c0;
                loop {
                    let mut p = self.head_of(cx, cy, list);
                    let mut n = 0u32;
                    while p != NONE {
                        n += 1;
                        if n > guard {
                            break;
                        }
                        let proxy = &self.proxies[p as usize];
                        let next = proxy.bucket_next;
                        let other_slot = proxy.slot as usize;
                        if other_slot != slot {
                            let other = &self.bodies[other_slot];
                            let overlapped = if self.wide {
                                let mine = self.wide_pose[slot];
                                let theirs = self.wide_pose[other_slot];
                                shapes_pair(
                                    x + mine.ox,
                                    y + mine.oy,
                                    mine.hw,
                                    mine.hh,
                                    me.kind,
                                    theirs.x + theirs.ox,
                                    theirs.y + theirs.oy,
                                    theirs.hw,
                                    theirs.hh,
                                    other.kind,
                                )
                            } else {
                                shapes_f32(me, x as f32, y as f32, other, other.x, other.y)
                            };
                            if other.alive != 0
                                && (stop == false || other.sensor == 0)
                                && groups_match(me, other)
                                && overlapped
                            {
                                if stop {
                                    return true;
                                }
                                if let Some(buf) = out.as_mut() {
                                    let id = other.id;
                                    if !buf.contains(&id) {
                                        buf.push(id);
                                    }
                                }
                            }
                        }
                        p = next;
                    }
                    if cx == c1 {
                        break;
                    }
                    cx += 1;
                }
                if cy == r1 {
                    break;
                }
                cy += 1;
            }
        }
        false
    }

    /// 问这两个编号在给定中心上是否重叠。分组和形状都算。边贴着不算。
    pub fn hit(&self, a: usize, ax: f64, ay: f64, b: usize, bx: f64, by: f64) -> bool {
        if a == b {
            return false;
        }
        let Some(sa) = self.slot(a) else {
            return false;
        };
        let Some(sb) = self.slot(b) else {
            return false;
        };
        let ba = &self.bodies[sa];
        let bb = &self.bodies[sb];
        if ba.alive == 0 || bb.alive == 0 || !groups_match(ba, bb) {
            return false;
        }
        let ax = self.fit(ax);
        let ay = self.fit(ay);
        let bx = self.fit(bx);
        let by = self.fit(by);
        if self.wide {
            let wa = self.wide_pose[sa];
            let wb = self.wide_pose[sb];
            shapes_pair(ax + wa.ox, ay + wa.oy, wa.hw, wa.hh, ba.kind, bx + wb.ox, by + wb.oy, wb.hw, wb.hh, bb.kind)
        } else {
            shapes_f32(ba, ax as f32, ay as f32, bb, bx as f32, by as f32)
        }
    }

    /// 一次查很多个编号。结果是 `(被查询的编号, 对方)`。两边都在名单里时，两个方向都留。
    /// 选了显卡且精度是 f32 时先走显卡，查不了再整次改走 CPU。显卡路径是实验功能。
    pub fn query(&mut self, indices: &[usize]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        if self.wide || !self.query_gpu(indices, &mut out) {
            out.clear();
            self.query_many(indices, &mut out);
        }
        out
    }

    /// 沿位移贴着别的实体滑，并把新中心写进去。先水平，再竖直。
    /// 空地上每轴只扫一次格子。边贴着可以停。传感器不挡人，自己也不会被挡住。
    /// 编号不存在时返回 `(0, 0)`，世界不变。
    pub fn move_by(&mut self, idx: usize, dx: f64, dy: f64) -> (f64, f64) {
        let Some(slot) = self.slot(idx) else {
            return (0.0, 0.0);
        };
        if self.bodies[slot].alive == 0 {
            return (0.0, 0.0);
        }
        let (x, y) = self.slide_pose(slot, dx, dy, true);
        self.store_slide(slot, x, y)
    }

    /// 优化前的逐个滑动：起点已经在别人身体里时停住，不往外推。
    #[doc(hidden)]
    pub fn move_by_held(&mut self, idx: usize, dx: f64, dy: f64) -> (f64, f64) {
        let Some(slot) = self.slot(idx) else {
            return (0.0, 0.0);
        };
        if self.bodies[slot].alive == 0 {
            return (0.0, 0.0);
        }
        let (x, y) = self.slide_pose(slot, dx, dy, false);
        self.store_slide(slot, x, y)
    }

    fn store_slide(&mut self, slot: usize, x: f64, y: f64) -> (f64, f64) {
        let x = self.fit(x);
        let y = self.fit(y);
        let (cx, cy) = self.pose_at(slot);
        if x == cx && y == cy {
            return (cx, cy);
        }
        if !self.wide {
            let b = &self.bodies[slot];
            let xf = x as f32;
            let yf = y as f32;
            let ox = b.ox;
            let oy = b.oy;
            let (c0, r0, c1, r1) = cell_span_f32(xf, yf, ox, oy, b.hw, b.hh, b.kind);
            if b.linked != 0 && b.c0 == c0 && b.r0 == r0 && b.c1 == c1 && b.r1 == r1 {
                self.bodies[slot].x = xf;
                self.bodies[slot].y = yf;
                self.touch_pose();
                self.nudge_solid_center(slot, xf + ox, yf + oy);
                return (x, y);
            }
        }
        let id = self.bodies[slot].id;
        self.set_pose(id, x, y);
        self.pose_at(slot)
    }

    fn slide_pose(&self, slot: usize, dx: f64, dy: f64, release: bool) -> (f64, f64) {
        let (x, y) = self.pose_at(slot);
        self.slide_from(slot, x, y, dx, dy, release)
    }

    fn slide_from(&self, slot: usize, x0: f64, y0: f64, dx: f64, dy: f64, release: bool) -> (f64, f64) {
        if !dx.is_finite() || !dy.is_finite() {
            return self.pose_at(slot);
        }
        if self.wide {
            let w = self.wide_pose[slot];
            let b = &self.bodies[slot];
            let x1 = x0 + dx;
            let y1 = y0 + dy;
            if b.sensor != 0 {
                return (x1, y1);
            }
            let x = sweep_axis(self, slot, true, x0, x1, y0, w.ox, w.oy, w.hw, w.hh, b.kind, release);
            let y = sweep_axis(self, slot, false, y0, y1, x, w.ox, w.oy, w.hw, w.hh, b.kind, release);
            (x, y)
        } else {
            let b = &self.bodies[slot];
            let ox = b.ox;
            let oy = b.oy;
            let hw = b.hw;
            let hh = b.hh;
            let kind = b.kind;
            let sensor = b.sensor;
            let x0 = x0 as f32;
            let y0 = y0 as f32;
            let x1 = self.fit(x0 as f64 + dx) as f32;
            let y1 = self.fit(y0 as f64 + dy) as f32;
            if sensor != 0 {
                return (x1 as f64, y1 as f64);
            }
            let (x, y) = if kind == KIND_RECT {
                let x = sweep_rect_f32(self, slot, true, x0, x1, y0, ox, oy, hw, hh, release);
                let y = sweep_rect_f32(self, slot, false, y0, y1, x, ox, oy, hw, hh, release);
                (x, y)
            } else {
                let x = sweep_axis(self, slot, true, x0, x1, y0, ox, oy, hw, hh, kind, release);
                let y = sweep_axis(self, slot, false, y0, y1, x, ox, oy, hw, hh, kind, release);
                (x, y)
            };
            (x as f64, y as f64)
        }
    }

    pub(super) fn query_gpu(&mut self, indices: &[usize], out: &mut Vec<(usize, usize)>) -> bool {
        #[cfg(feature = "gpu")]
        {
            let Some(mut gpu) = self.gpu.take() else {
                return false;
            };
            let ok = gpu.query(self, indices, out);
            self.gpu = Some(gpu);
            return ok;
        }
        #[cfg(not(feature = "gpu"))]
        {
            let _ = (indices, out);
            false
        }
    }
}

trait SlideNum: Copy + PartialOrd + std::ops::Add<Output = Self> + std::ops::Sub<Output = Self> + std::ops::Mul<Output = Self> {
    fn from_f32(v: f32) -> Self;
    fn from_f64(v: f64) -> Self;
    fn sqrt(self) -> Self;
    fn abs(self) -> Self;
    fn min_num(self, other: Self) -> Self;
    fn max_num(self, other: Self) -> Self;
    fn cell(self, wide: bool) -> i32;
}

impl SlideNum for f32 {
    #[inline]
    fn from_f32(v: f32) -> Self { v }
    #[inline]
    fn from_f64(v: f64) -> Self { v as f32 }
    #[inline]
    fn sqrt(self) -> Self { f32::sqrt(self) }
    #[inline]
    fn abs(self) -> Self { f32::abs(self) }
    #[inline]
    fn min_num(self, other: Self) -> Self { f32::min(self, other) }
    #[inline]
    fn max_num(self, other: Self) -> Self { f32::max(self, other) }
    #[inline]
    fn cell(self, _: bool) -> i32 { cell_of_f32(self) }
}

impl SlideNum for f64 {
    #[inline]
    fn from_f32(v: f32) -> Self { v as f64 }
    #[inline]
    fn from_f64(v: f64) -> Self { v }
    #[inline]
    fn sqrt(self) -> Self { f64::sqrt(self) }
    #[inline]
    fn abs(self) -> Self { f64::abs(self) }
    #[inline]
    fn min_num(self, other: Self) -> Self { f64::min(self, other) }
    #[inline]
    fn max_num(self, other: Self) -> Self { f64::max(self, other) }
    #[inline]
    fn cell(self, wide: bool) -> i32 { cell_of(self, wide) }
}

#[inline]
fn slide_clamp<T: SlideNum>(v: T, lo: T, hi: T) -> T {
    if v < lo { lo } else if v > hi { hi } else { v }
}

/// 形状中心在运动轴上落进 `(lo, hi)` 才重叠。刚好贴边可以停。
#[inline]
fn block_interval<T: SlideNum>(
    ak: u8,
    a_half_along: T,
    a_half_perp: T,
    a_perp: T,
    bk: u8,
    b_along: T,
    b_half_along: T,
    b_perp: T,
    b_half_perp: T,
) -> Option<(T, T)> {
    let gap = match (ak, bk) {
        (KIND_RECT, KIND_RECT) => {
            if (a_perp - b_perp).abs() >= a_half_perp + b_half_perp {
                return None;
            }
            a_half_along + b_half_along
        }
        (KIND_CIRCLE, KIND_CIRCLE) => {
            let dy = a_perp - b_perp;
            let r = a_half_along + b_half_along;
            if dy * dy >= r * r {
                return None;
            }
            (r * r - dy * dy).sqrt()
        }
        (KIND_RECT, KIND_CIRCLE) => {
            let py = slide_clamp(b_perp, a_perp - a_half_perp, a_perp + a_half_perp);
            let dy = b_perp - py;
            let r = b_half_along;
            if dy * dy >= r * r {
                return None;
            }
            a_half_along + (r * r - dy * dy).sqrt()
        }
        _ => {
            let py = slide_clamp(a_perp, b_perp - b_half_perp, b_perp + b_half_perp);
            let dy = a_perp - py;
            let r = a_half_along;
            if dy * dy >= r * r {
                return None;
            }
            b_half_along + (r * r - dy * dy).sqrt()
        }
    };
    Some((b_along - gap, b_along + gap))
}

#[inline]
fn clamp_travel<T: SlideNum>(start: T, goal: T, lo: T, hi: T, release: bool) -> T {
    if start > lo && start < hi {
        if !release {
            return start;
        }
        let exit_lo = start - lo <= hi - start;
        if exit_lo {
            if goal <= lo { goal } else { lo }
        } else if goal >= hi {
            goal
        } else {
            hi
        }
    } else if goal >= start {
        if goal > lo && start <= lo { lo } else { goal }
    } else if goal < hi && start >= hi {
        hi
    } else {
        goal
    }
}

fn sweep_axis<T: SlideNum>(
    world: &Foil,
    slot: usize,
    along_x: bool,
    start: T,
    target: T,
    perp: T,
    ox: T,
    oy: T,
    hw: T,
    hh: T,
    kind: u8,
    release: bool,
) -> T {
    let (hx, hy) = if kind == KIND_CIRCLE { (hw, hw) } else { (hw, hh) };
    let (a_half_along, a_half_perp, a_perp) = if along_x {
        (hx, hy, perp + oy)
    } else {
        (hy, hx, perp + ox)
    };
    let a_center_start = if along_x { start + ox } else { start + oy };
    let (c0, r0, c1, r1) = swept_cells(along_x, start, target, perp, ox, oy, hx, hy, world.wide);
    let (my_group, my_mask) = {
        let me = &world.bodies[slot];
        (me.group, me.mask)
    };
    let guard = world.proxies.len() as u32 + 1;
    let mut center_goal = if along_x { target + ox } else { target + oy };
    let mut cy = r0;
    loop {
        let mut cx = c0;
        loop {
            let mut p = world.head_of(cx, cy, 0);
            let mut n = 0u32;
            if world.wide {
            while p != NONE {
                n += 1;
                if n > guard {
                    break;
                }
                let proxy = &world.proxies[p as usize];
                let next = proxy.bucket_next;
                let other_slot = proxy.slot as usize;
                if other_slot != slot {
                    let (alive, sensor, other_kind, group, mask) = {
                        let other = &world.bodies[other_slot];
                        (other.alive, other.sensor, other.kind, other.group, other.mask)
                    };
                    let hit = alive != 0
                        && sensor == 0
                        && (my_mask == 0 || group == 0 || (my_mask & group) != 0)
                        && (mask == 0 || my_group == 0 || (mask & my_group) != 0);
                    if hit {
                        let (bx, by, bw, bh) = other_shape::<T>(world, other_slot);
                        let (b_along, b_perp, b_half_along, b_half_perp) = if along_x {
                            (bx, by, bw, bh)
                        } else {
                            (by, bx, bh, bw)
                        };
                        if let Some((lo, hi)) = block_interval(
                            kind,
                            a_half_along,
                            a_half_perp,
                            a_perp,
                            other_kind,
                            b_along,
                            b_half_along,
                            b_perp,
                            b_half_perp,
                        ) {
                            center_goal = clamp_travel(a_center_start, center_goal, lo, hi, release);
                            if !release && center_goal == a_center_start {
                                return if along_x { center_goal - ox } else { center_goal - oy };
                            }
                        }
                    }
                }
                p = next;
            }
            } else {
                for hit in world.solid_of(cx, cy) {
                    let other_slot = hit.slot as usize;
                    if other_slot == slot {
                        continue;
                    }
                    let blocks = (my_mask == 0 || hit.group == 0 || (my_mask & hit.group) != 0)
                        && (hit.mask == 0 || my_group == 0 || (hit.mask & my_group) != 0);
                    if !blocks {
                        continue;
                    }
                    let (bw, bh) = if hit.kind == KIND_CIRCLE {
                        (T::from_f32(hit.hw), T::from_f32(hit.hw))
                    } else {
                        (T::from_f32(hit.hw), T::from_f32(hit.hh))
                    };
                    let bx = T::from_f32(hit.sx);
                    let by = T::from_f32(hit.sy);
                    let (b_along, b_perp, b_half_along, b_half_perp) = if along_x {
                        (bx, by, bw, bh)
                    } else {
                        (by, bx, bh, bw)
                    };
                    if let Some((lo, hi)) = block_interval(
                        kind,
                        a_half_along,
                        a_half_perp,
                        a_perp,
                        hit.kind,
                        b_along,
                        b_half_along,
                        b_perp,
                        b_half_perp,
                    ) {
                        center_goal = clamp_travel(a_center_start, center_goal, lo, hi, release);
                        if !release && center_goal == a_center_start {
                            return if along_x { center_goal - ox } else { center_goal - oy };
                        }
                    }
                }
            }
            if cx == c1 {
                break;
            }
            cx += 1;
        }
        if cy == r1 {
            break;
        }
        cy += 1;
    }
    if along_x { center_goal - ox } else { center_goal - oy }
}

fn sweep_rect_f32(
    world: &Foil,
    slot: usize,
    along_x: bool,
    start: f32,
    target: f32,
    perp: f32,
    ox: f32,
    oy: f32,
    hw: f32,
    hh: f32,
    release: bool,
) -> f32 {
    let (c0, r0, c1, r1) = swept_cells(along_x, start, target, perp, ox, oy, hw, hh, false);
    let (my_group, my_mask) = {
        let me = &world.bodies[slot];
        (me.group, me.mask)
    };
    let (a_half_along, a_half_perp, a_perp, a_center_start, mut center_goal) = if along_x {
        (hw, hh, perp + oy, start + ox, target + ox)
    } else {
        (hh, hw, perp + ox, start + oy, target + oy)
    };
    let mut cy = r0;
    loop {
        let mut cx = c0;
        loop {
            for hit in world.solid_of(cx, cy) {
                if hit.slot as usize == slot {
                    continue;
                }
                if !((my_mask == 0 || hit.group == 0 || (my_mask & hit.group) != 0)
                    && (hit.mask == 0 || my_group == 0 || (hit.mask & my_group) != 0))
                {
                    continue;
                }
                if hit.kind != KIND_RECT {
                    let (bw, bh) = if hit.kind == KIND_CIRCLE {
                        (hit.hw, hit.hw)
                    } else {
                        (hit.hw, hit.hh)
                    };
                    let (b_along, b_perp, b_half_along, b_half_perp) = if along_x {
                        (hit.sx, hit.sy, bw, bh)
                    } else {
                        (hit.sy, hit.sx, bh, bw)
                    };
                    if let Some((lo, hi)) = block_interval(
                        KIND_RECT,
                        a_half_along,
                        a_half_perp,
                        a_perp,
                        hit.kind,
                        b_along,
                        b_half_along,
                        b_perp,
                        b_half_perp,
                    ) {
                        center_goal = clamp_travel(a_center_start, center_goal, lo, hi, release);
                        if !release && center_goal == a_center_start {
                            return if along_x { center_goal - ox } else { center_goal - oy };
                        }
                    }
                    continue;
                }
                let (b_along, b_perp, b_half_along, b_half_perp) = if along_x {
                    (hit.sx, hit.sy, hit.hw, hit.hh)
                } else {
                    (hit.sy, hit.sx, hit.hh, hit.hw)
                };
                if (a_perp - b_perp).abs() >= a_half_perp + b_half_perp {
                    continue;
                }
                let gap = a_half_along + b_half_along;
                center_goal = clamp_travel(a_center_start, center_goal, b_along - gap, b_along + gap, release);
                if !release && center_goal == a_center_start {
                    return if along_x { center_goal - ox } else { center_goal - oy };
                }
            }
            if cx == c1 {
                break;
            }
            cx += 1;
        }
        if cy == r1 {
            break;
        }
        cy += 1;
    }
    if along_x { center_goal - ox } else { center_goal - oy }
}

fn other_shape<T: SlideNum>(world: &Foil, slot: usize) -> (T, T, T, T) {
    let kind = world.bodies[slot].kind;
    let (x, y, ox, oy, hw, hh) = if world.wide {
        let w = &world.wide_pose[slot];
        (T::from_f64(w.x), T::from_f64(w.y), T::from_f64(w.ox), T::from_f64(w.oy), T::from_f64(w.hw), T::from_f64(w.hh))
    } else {
        let body = &world.bodies[slot];
        (T::from_f32(body.x), T::from_f32(body.y), T::from_f32(body.ox), T::from_f32(body.oy), T::from_f32(body.hw), T::from_f32(body.hh))
    };
    let (hx, hy) = if kind == KIND_CIRCLE { (hw, hw) } else { (hw, hh) };
    (x + ox, y + oy, hx, hy)
}

fn swept_cells<T: SlideNum>(
    along_x: bool,
    start: T,
    target: T,
    perp: T,
    ox: T,
    oy: T,
    hx: T,
    hy: T,
    wide: bool,
) -> (i32, i32, i32, i32) {
    let (x0, x1, y0, y1) = if along_x {
        (start.min_num(target) + ox - hx, start.max_num(target) + ox + hx, perp + oy - hy, perp + oy + hy)
    } else {
        (perp + ox - hx, perp + ox + hx, start.min_num(target) + oy - hy, start.max_num(target) + oy + hy)
    };
    (x0.cell(wide), y0.cell(wide), x1.cell(wide), y1.cell(wide))
}

/// `mask == 0` 碰上所有人。对方 `group == 0` 也碰上。两边都要过。
#[inline]
fn groups_match(a: &Body, b: &Body) -> bool {
    let a_ok = a.mask == 0 || b.group == 0 || (a.mask & b.group) != 0;
    let b_ok = b.mask == 0 || a.group == 0 || (b.mask & a.group) != 0;
    a_ok && b_ok
}

#[inline]
fn shapes_f32(a: &Body, ax: f32, ay: f32, b: &Body, bx: f32, by: f32) -> bool {
    shapes_pair(ax + a.ox, ay + a.oy, a.hw, a.hh, a.kind, bx + b.ox, by + b.oy, b.hw, b.hh, b.kind)
}

#[inline]
fn shapes_pair<T>(acx: T, acy: T, ahw: T, ahh: T, ak: u8, bcx: T, bcy: T, bhw: T, bhh: T, bk: u8) -> bool
where
    T: Copy + PartialOrd + std::ops::Add<Output = T> + std::ops::Sub<Output = T> + std::ops::Mul<Output = T>,
{
    match (ak, bk) {
        (KIND_RECT, KIND_RECT) => {
            acx - ahw < bcx + bhw && acx + ahw > bcx - bhw && acy - ahh < bcy + bhh && acy + ahh > bcy - bhh
        }
        (KIND_CIRCLE, KIND_CIRCLE) => {
            let dx = acx - bcx;
            let dy = acy - bcy;
            let r = ahw + bhw;
            dx * dx + dy * dy < r * r
        }
        (KIND_RECT, KIND_CIRCLE) => aabb_circle(acx, acy, ahw, ahh, bcx, bcy, bhw),
        _ => aabb_circle(bcx, bcy, bhw, bhh, acx, acy, ahw),
    }
}

#[inline]
fn aabb_circle<T>(rx: T, ry: T, hw: T, hh: T, cx: T, cy: T, radius: T) -> bool
where
    T: Copy + PartialOrd + std::ops::Add<Output = T> + std::ops::Sub<Output = T> + std::ops::Mul<Output = T>,
{
    let px = clamp_num(cx, rx - hw, rx + hw);
    let py = clamp_num(cy, ry - hh, ry + hh);
    let dx = cx - px;
    let dy = cy - py;
    dx * dx + dy * dy < radius * radius
}

#[inline]
fn clamp_num<T: Copy + PartialOrd>(v: T, lo: T, hi: T) -> T {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}


struct StampState {
    gen: u32,
    data: Vec<u32>,
}

thread_local! {
    static STAMP: RefCell<StampState> = RefCell::new(StampState { gen: 1, data: Vec::new() });
}
