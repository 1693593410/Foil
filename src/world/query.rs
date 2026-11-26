//! 重叠查询和形状测试。

use std::cell::RefCell;

use rayon::prelude::*;

use super::grid::{cell_range, cell_span_f32};
use super::QUERY_PARALLEL_AT;
use super::types::*;
use super::Foil;

impl Foil {
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

    /// 这一帧的 (被查询的物体, 对方)。两边都查时两对都留。
    /// `open` 选了显卡时先走显卡，查不了再走 CPU。
    pub fn query(&mut self, indices: &[usize]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        if self.wide || !self.query_gpu(indices, &mut out) {
            out.clear();
            self.query_many(indices, &mut out);
        }
        out
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
