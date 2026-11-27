//! 重叠查询和形状测试。

use std::cell::RefCell;

use rayon::prelude::*;

use super::grid::{
    cell_of, cell_of_f32, cell_range, cell_span_f32, relink_planned, GridPtrs, ProxyAlloc,
};
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

    fn take_pool(&mut self) -> Option<rayon::ThreadPool> {
        if self.thread_limit == 0 {
            return None;
        }
        let mut slot = self.thread_pool.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = rayon::ThreadPoolBuilder::new().num_threads(self.thread_limit).build().ok();
        }
        slot.take()
    }

    fn put_pool(&mut self, pool: Option<rayon::ThreadPool>) {
        if self.thread_limit == 0 {
            return;
        }
        *self.thread_pool.lock().unwrap_or_else(|e| e.into_inner()) = pool;
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
        let (x, y) = self.slide_pose(slot, dx, dy);
        let (cx, cy) = self.pose_at(slot);
        if x != cx || y != cy {
            self.set_pose(self.bodies[slot].id, x, y);
        }
        self.pose_at(slot)
    }

    /// 同一帧移动一批。扫描、写坐标、重挂格子都并行。
    /// 每个人只看见这一帧开始时的位置，所以互不遮挡的墙和远处物体跟逐个 `move_by` 一样。
    /// 两个都在动、并且路径叠在一起的，这一帧可能都走进去。编号可以缺，重复编号会改走逐个移动。
    /// 少于 192 个或线程上限是 1 时，直接逐个 `move_by`。
    pub fn move_many(&mut self, ids: &[usize], dx: &[f64], dy: &[f64]) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        self.move_many_into(ids, dx, dy, &mut out);
        out
    }

    /// 和 [`Foil::move_many`] 相同，结果写进调用方的缓冲。
    pub fn move_many_into(&mut self, ids: &[usize], dx: &[f64], dy: &[f64], out: &mut Vec<(f64, f64)>) {
        let n = ids.len().min(dx.len()).min(dy.len());
        out.clear();
        if n == 0 {
            return;
        }
        let threads = self.batch_threads();
        if threads == 1 || n < QUERY_PARALLEL_AT {
            out.reserve(n);
            for i in 0..n {
                out.push(self.move_by(ids[i], dx[i], dy[i]));
            }
            return;
        }
        let mut scratch = std::mem::take(&mut self.move_scratch);
        if !self.plan_batch(&mut scratch, ids, dx, dy, n) || !self.grow_plans(&scratch.plans) {
            self.move_scratch = scratch;
            out.reserve(n);
            for i in 0..n {
                out.push(self.move_by(ids[i], dx[i], dy[i]));
            }
            return;
        }
        self.publish_batch(&scratch, threads);
        out.extend(scratch.plans.iter().map(|p| (p.x, p.y)));
        self.move_scratch = scratch;
    }

    fn batch_threads(&self) -> usize {
        if self.thread_limit == 1 {
            1
        } else if self.thread_limit == 0 {
            rayon::current_num_threads().max(1)
        } else {
            self.thread_limit
        }
    }

    fn slide_pose(&self, slot: usize, dx: f64, dy: f64) -> (f64, f64) {
        if !dx.is_finite() || !dy.is_finite() {
            return self.pose_at(slot);
        }
        if self.wide {
            let w = self.wide_pose[slot];
            let b = &self.bodies[slot];
            let x1 = w.x + dx;
            let y1 = w.y + dy;
            if b.sensor != 0 {
                return (x1, y1);
            }
            let x = sweep_axis(self, slot, true, w.x, x1, w.y, w.ox, w.oy, w.hw, w.hh, b.kind);
            let y = sweep_axis(self, slot, false, w.y, y1, x, w.ox, w.oy, w.hw, w.hh, b.kind);
            (x, y)
        } else {
            let b = &self.bodies[slot];
            let x0 = b.x;
            let y0 = b.y;
            let x1 = self.fit(x0 as f64 + dx) as f32;
            let y1 = self.fit(y0 as f64 + dy) as f32;
            if b.sensor != 0 {
                return (x1 as f64, y1 as f64);
            }
            let x = sweep_axis(self, slot, true, x0, x1, y0, b.ox, b.oy, b.hw, b.hh, b.kind);
            let y = sweep_axis(self, slot, false, y0, y1, x, b.ox, b.oy, b.hw, b.hh, b.kind);
            (x as f64, y as f64)
        }
    }

    /// 规划失败（重复编号，或格子扩不动）时返回 false，调用方改走逐个移动。
    fn plan_batch(&self, scratch: &mut super::types::MoveScratch, ids: &[usize], dx: &[f64], dy: &[f64], n: usize) -> bool {
        scratch.plans.clear();
        scratch.plans.reserve(n);
        if scratch.seen.len() < self.bodies.len() {
            scratch.seen.resize(self.bodies.len(), 0);
        }
        scratch.seen_gen = scratch.seen_gen.wrapping_add(1);
        if scratch.seen_gen == 0 {
            scratch.seen.fill(0);
            scratch.seen_gen = 1;
        }
        let gen = scratch.seen_gen;
        for i in 0..n {
            let Some(slot) = self.slot(ids[i]) else {
                scratch.plans.push(super::types::Planned {
                    slot: NONE,
                    x: 0.0,
                    y: 0.0,
                    c0: 0,
                    r0: 0,
                    c1: 0,
                    r1: 0,
                    kind: 0,
                });
                continue;
            };
            if self.bodies[slot].alive == 0 {
                scratch.plans.push(super::types::Planned {
                    slot: NONE,
                    x: 0.0,
                    y: 0.0,
                    c0: 0,
                    r0: 0,
                    c1: 0,
                    r1: 0,
                    kind: 0,
                });
                continue;
            }
            if scratch.seen[slot] == gen {
                return false;
            }
            scratch.seen[slot] = gen;
            scratch.plans.push(super::types::Planned {
                slot: slot as u32,
                x: 0.0,
                y: 0.0,
                c0: 0,
                r0: 0,
                c1: 0,
                r1: 0,
                kind: 0,
            });
        }
        let world = self;
        let ptr = SendMut(scratch.plans.as_mut_ptr() as usize);
        let dx = &dx[..n];
        let dy = &dy[..n];
        self.with_threads(|| {
            (0..n).into_par_iter().for_each(move |i| {
                let plan = unsafe { &mut *(ptr.0 as *mut Planned).add(i) };
                if plan.slot == NONE {
                    return;
                }
                let slot = plan.slot as usize;
                let (x, y) = world.slide_pose(slot, dx[i], dy[i]);
                let (cx, cy) = world.pose_at(slot);
                plan.x = x;
                plan.y = y;
                if x == cx && y == cy {
                    plan.kind = 0;
                    return;
                }
                let span = world.span_of_pose(slot, x, y);
                let body = &world.bodies[slot];
                let same = body.linked != 0 && body.c0 == span.0 && body.r0 == span.1 && body.c1 == span.2 && body.r1 == span.3;
                plan.c0 = span.0;
                plan.r0 = span.1;
                plan.c1 = span.2;
                plan.r1 = span.3;
                plan.kind = if same { 1 } else { 2 };
            });
        });
        true
    }

    fn grow_plans(&mut self, plans: &[super::types::Planned]) -> bool {
        let mut c0 = i32::MAX;
        let mut r0 = i32::MAX;
        let mut c1 = i32::MIN;
        let mut r1 = i32::MIN;
        let mut any = false;
        for plan in plans {
            if plan.kind != 2 || plan.slot == NONE {
                continue;
            }
            any = true;
            c0 = c0.min(plan.c0);
            r0 = r0.min(plan.r0);
            c1 = c1.max(plan.c1);
            r1 = r1.max(plan.r1);
            let body = &self.bodies[plan.slot as usize];
            if body.linked != 0 {
                c0 = c0.min(body.c0);
                r0 = r0.min(body.r0);
                c1 = c1.max(body.c1);
                r1 = r1.max(body.r1);
            }
        }
        if !any {
            return true;
        }
        self.try_grow(c0, r0, c1, r1) && !self.extra_blocks(c0, r0, c1, r1)
    }

    fn span_of_pose(&self, slot: usize, x: f64, y: f64) -> (i32, i32, i32, i32) {
        let body = &self.bodies[slot];
        if self.wide {
            let w = &self.wide_pose[slot];
            cell_range(x, y, w.ox, w.oy, w.hw, w.hh, body.kind, true)
        } else {
            cell_span_f32(x as f32, y as f32, body.ox, body.oy, body.hw, body.hh, body.kind)
        }
    }

    fn extra_blocks(&self, c0: i32, r0: i32, c1: i32, r1: i32) -> bool {
        self.extra.keys().any(|&(cx, cy)| cx >= c0 && cx <= c1 && cy >= r0 && cy <= r1)
    }

    fn publish_batch(&mut self, scratch: &super::types::MoveScratch, threads: usize) {
        let mut wrote = false;
        let mut relinked = false;
        let g = GridPtrs {
            bodies: self.bodies.as_mut_ptr(),
            proxies: self.proxies.as_mut_ptr(),
            heads: self.heads.as_mut_ptr(),
            ox: self.ox,
            oy: self.oy,
            gw: self.gw,
            gh: self.gh,
        };
        let wide_ptr = self.wide_pose.as_mut_ptr();
        let wide = self.wide;
        let plans = scratch.plans.as_ptr();
        let n = scratch.plans.len();
        let pool = self.take_pool();
        let install = |job: Box<dyn FnOnce() + Send + '_>| {
            if let Some(pool) = &pool {
                pool.install(|| job());
            } else {
                job();
            }
        };
        let pose_job = JobPtrs {
            bodies: g.bodies as usize,
            proxies: g.proxies as usize,
            heads: g.heads as usize,
            ox: g.ox,
            oy: g.oy,
            gw: g.gw,
            gh: g.gh,
            wide: wide_ptr as usize,
            plans: plans as usize,
            allocs: 0,
            frees: 0,
            stripes: 0,
        };
        install(Box::new(move || {
            (0..n).into_par_iter().for_each(move |i| {
                let plan = unsafe { &*(pose_job.plans as *const Planned).add(i) };
                if plan.kind == 0 || plan.slot == NONE {
                    return;
                }
                unsafe {
                    let slot = plan.slot as usize;
                    if wide {
                        let w = &mut *(pose_job.wide as *mut Wide).add(slot);
                        w.x = plan.x;
                        w.y = plan.y;
                    }
                    let body = &mut *(pose_job.bodies as *mut Body).add(slot);
                    body.x = plan.x as f32;
                    body.y = plan.y as f32;
                }
            });
        }));
        for plan in &scratch.plans {
            if plan.kind != 0 {
                wrote = true;
                break;
            }
        }
        let mut demand = vec![0u32; threads];
        let mut owned: Vec<Vec<usize>> = (0..threads).map(|_| Vec::new()).collect();
        let mut boundary: Vec<Vec<usize>> = (0..threads).map(|_| Vec::new()).collect();
        for (i, plan) in scratch.plans.iter().enumerate() {
            if plan.kind != 2 {
                continue;
            }
            relinked = true;
            let cells = span_cells(plan.c0, plan.r0, plan.c1, plan.r1);
            let body = &self.bodies[plan.slot as usize];
            let old_single = body.linked == 0 || body.c0 == body.c1;
            let same_col = body.linked == 0 || body.c0 == plan.c0;
            if plan.c0 == plan.c1 && old_single && same_col {
                let owner = plan.c0.rem_euclid(threads as i32) as usize;
                demand[owner] += cells;
                owned[owner].push(i);
            } else {
                let owner = i % threads;
                demand[owner] += cells;
                boundary[owner].push(i);
            }
        }
        if relinked {
            let mut total = 0u32;
            for d in &demand {
                total += *d;
            }
            let base = self.proxies.len() as u32;
            self.proxies.resize(self.proxies.len() + total as usize, empty_proxy());
            let proxies = self.proxies.as_mut_ptr();
            let mut cursor = base;
            let mut ranges = Vec::with_capacity(threads);
            for d in &demand {
                let start = cursor;
                cursor += *d;
                ranges.push((start, cursor));
            }
            let g = GridPtrs {
                bodies: self.bodies.as_mut_ptr(),
                proxies,
                heads: self.heads.as_mut_ptr(),
                ox: self.ox,
                oy: self.oy,
                gw: self.gw,
                gh: self.gh,
            };
            let stripes = self.move_stripes.as_ptr();
            let plans = scratch.plans.as_ptr();
            let mut allocs: Vec<ProxyAlloc> = ranges
                .iter()
                .map(|&(start, end)| ProxyAlloc { next: start, end, local_free: NONE })
                .collect();
            let alloc_ptr = allocs.as_mut_ptr();
            let mut free_heads = vec![NONE; threads];
            let free_ptr = free_heads.as_mut_ptr();
            let link_job = JobPtrs {
                bodies: g.bodies as usize,
                proxies: g.proxies as usize,
                heads: g.heads as usize,
                ox: g.ox,
                oy: g.oy,
                gw: g.gw,
                gh: g.gh,
                wide: 0,
                plans: plans as usize,
                allocs: alloc_ptr as usize,
                frees: free_ptr as usize,
                stripes: stripes as usize,
            };
            install(Box::new(move || {
                (0..threads).into_par_iter().for_each(move |t| {
                    let alloc = unsafe { &mut *(link_job.allocs as *mut ProxyAlloc).add(t) };
                    let grid = link_job.grid();
                    for &i in &owned[t] {
                        let plan = unsafe { &*(link_job.plans as *const Planned).add(i) };
                        unsafe {
                            relink_planned(grid, alloc, plan.slot as usize, (plan.c0, plan.r0, plan.c1, plan.r1));
                        }
                    }
                });
                (0..threads).into_par_iter().for_each(move |t| {
                    let alloc = unsafe { &mut *(link_job.allocs as *mut ProxyAlloc).add(t) };
                    let grid = link_job.grid();
                    for &i in &boundary[t] {
                        let plan = unsafe { &*(link_job.plans as *const Planned).add(i) };
                        let body = unsafe { &*(link_job.bodies as *const Body).add(plan.slot as usize) };
                        let mut ids = [0u8; 64];
                        let mut count = 0usize;
                        push_stripes(&mut ids, &mut count, plan.c0, plan.c1);
                        if body.linked != 0 {
                            push_stripes(&mut ids, &mut count, body.c0, body.c1);
                        }
                        ids[..count].sort_unstable();
                        let mut guards = Vec::with_capacity(count);
                        for s in &ids[..count] {
                            let stripe = unsafe { &*(link_job.stripes as *const std::sync::Mutex<()>).add(*s as usize) };
                            guards.push(stripe.lock().unwrap_or_else(|e| e.into_inner()));
                        }
                        unsafe {
                            relink_planned(grid, alloc, plan.slot as usize, (plan.c0, plan.r0, plan.c1, plan.r1));
                        }
                        drop(guards);
                    }
                    let mut tail = alloc.local_free;
                    while alloc.next < alloc.end {
                        unsafe {
                            (*(link_job.proxies as *mut Proxy).add(alloc.next as usize)).bucket_next = tail;
                        }
                        tail = alloc.next;
                        alloc.next += 1;
                    }
                    unsafe {
                        *(link_job.frees as *mut u32).add(t) = tail;
                    }
                });
            }));
            for head in free_heads {
                self.splice_free(head);
            }
        }
        if wrote {
            self.touch_pose();
        }
        if relinked {
            self.touch_grid();
        }
        self.put_pool(pool);
    }

    fn splice_free(&mut self, head: u32) {
        if head == NONE {
            return;
        }
        let mut tail = head;
        let guard = self.proxies.len() as u32 + 1;
        let mut n = 0u32;
        loop {
            n += 1;
            if n > guard {
                return;
            }
            let next = self.proxies[tail as usize].bucket_next;
            if next == NONE {
                break;
            }
            tail = next;
        }
        self.proxies[tail as usize].bucket_next = self.free_proxy;
        self.free_proxy = head;
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
fn clamp_travel<T: SlideNum>(start: T, goal: T, lo: T, hi: T) -> T {
    if goal >= start {
        if start > lo && start < hi {
            return start;
        }
        if goal > lo && start <= lo {
            return lo;
        }
        goal
    } else {
        if start > lo && start < hi {
            return start;
        }
        if goal < hi && start >= hi {
            return hi;
        }
        goal
    }
}

#[derive(Clone, Copy)]
struct SendMut(usize);

#[derive(Clone, Copy)]
struct JobPtrs {
    bodies: usize,
    proxies: usize,
    heads: usize,
    ox: i32,
    oy: i32,
    gw: i32,
    gh: i32,
    wide: usize,
    plans: usize,
    allocs: usize,
    frees: usize,
    stripes: usize,
}

impl JobPtrs {
    fn grid(self) -> GridPtrs {
        GridPtrs {
            bodies: self.bodies as *mut Body,
            proxies: self.proxies as *mut Proxy,
            heads: self.heads as *mut CellHeads,
            ox: self.ox,
            oy: self.oy,
            gw: self.gw,
            gh: self.gh,
        }
    }
}

fn empty_proxy() -> Proxy {
    Proxy {
        slot: 0,
        cx: 0,
        cy: 0,
        bucket_next: NONE,
        owned_next: NONE,
        list: 0,
    }
}

fn span_cells(c0: i32, r0: i32, c1: i32, r1: i32) -> u32 {
    let w = (c1 as i64 - c0 as i64 + 1).max(0);
    let h = (r1 as i64 - r0 as i64 + 1).max(0);
    (w * h) as u32
}

fn push_stripes(ids: &mut [u8; 64], count: &mut usize, c0: i32, c1: i32) {
    let mut cx = c0;
    loop {
        let s = cx.rem_euclid(64) as u8;
        if !ids[..*count].contains(&s) && *count < 64 {
            ids[*count] = s;
            *count += 1;
        }
        if cx == c1 {
            break;
        }
        cx += 1;
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
) -> T {
    if start == target {
        return start;
    }
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
                            center_goal = clamp_travel(a_center_start, center_goal, lo, hi);
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
