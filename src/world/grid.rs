//! 密铺格子、编号槽和链表。

use super::types::*;
use super::Foil;

impl Foil {
    pub(super) fn touch_pose(&mut self) {
        self.pose_gen = self.pose_gen.wrapping_add(1);
    }

    pub(super) fn touch_grid(&mut self) {
        self.grid_gen = self.grid_gen.wrapping_add(1);
    }

    pub(super) fn dense_index(&self, cx: i32, cy: i32) -> Option<usize> {
        if self.gw <= 0 {
            return None;
        }
        let x = cx.wrapping_sub(self.ox);
        let y = cy.wrapping_sub(self.oy);
        if x < 0 || y < 0 || x >= self.gw || y >= self.gh {
            None
        } else {
            Some((y * self.gw + x) as usize)
        }
    }

    pub(super) fn try_grow(&mut self, c0: i32, r0: i32, c1: i32, r1: i32) -> bool {
        let pad = 8i64;
        let c0 = c0 as i64;
        let r0 = r0 as i64;
        let c1 = c1 as i64;
        let r1 = r1 as i64;
        let ox = self.ox as i64;
        let oy = self.oy as i64;
        let gw = self.gw as i64;
        let gh = self.gh as i64;
        let nx0 = if gw <= 0 { c0 - pad } else { ox.min(c0 - pad) };
        let ny0 = if gw <= 0 { r0 - pad } else { oy.min(r0 - pad) };
        let nx1 = if gw <= 0 { c1 + pad + 1 } else { (ox + gw).max(c1 + pad + 1) };
        let ny1 = if gw <= 0 { r1 + pad + 1 } else { (oy + gh).max(r1 + pad + 1) };
        let nw = nx1 - nx0;
        let nh = ny1 - ny0;
        // 着色器用 u32 下标，一格两条链，所以格子数 × 2 必须放得进 u32。
        let Some(area) = nw.checked_mul(nh) else {
            return false;
        };
        if nw <= 0
            || nh <= 0
            || nw > i32::MAX as i64
            || nh > i32::MAX as i64
            || nx0 < i32::MIN as i64
            || ny0 < i32::MIN as i64
            || nx0 > i32::MAX as i64
            || ny0 > i32::MAX as i64
            || area > (u32::MAX as i64) / 2
        {
            return false;
        }
        let nx0 = nx0 as i32;
        let ny0 = ny0 as i32;
        let nw = nw as i32;
        let nh = nh as i32;
        if self.gw > 0 && nx0 == self.ox && ny0 == self.oy && nw == self.gw && nh == self.gh {
            return true;
        }
        let mut next = Vec::new();
        if next.try_reserve_exact(area as usize).is_err() {
            return false;
        }
        next.resize(area as usize, CellHeads::EMPTY);
        if self.gw > 0 {
            for y in 0..self.gh {
                for x in 0..self.gw {
                    let old = self.heads[(y * self.gw + x) as usize];
                    let nx = (self.ox + x) - nx0;
                    let ny = (self.oy + y) - ny0;
                    next[(ny * nw + nx) as usize] = old;
                }
            }
        }
        self.heads = next;
        self.ox = nx0;
        self.oy = ny0;
        self.gw = nw;
        self.gh = nh;
        let extra = std::mem::take(&mut self.extra);
        for ((cx, cy), heads) in extra {
            if let Some(i) = self.dense_index(cx, cy) {
                self.heads[i] = heads;
            } else {
                self.extra.insert((cx, cy), heads);
            }
        }
        true
    }

    pub(super) fn head_of(&self, cx: i32, cy: i32, list: usize) -> u32 {
        if let Some(i) = self.dense_index(cx, cy) {
            self.heads[i].lists[list]
        } else {
            self.extra
                .get(&(cx, cy))
                .map(|c| c.lists[list])
                .unwrap_or(NONE)
        }
    }

    pub(super) fn set_head(&mut self, cx: i32, cy: i32, list: usize, v: u32) {
        if let Some(i) = self.dense_index(cx, cy) {
            self.heads[i].lists[list] = v;
            return;
        }
        self.extra.entry((cx, cy)).or_insert(CellHeads::EMPTY).lists[list] = v;
    }

    pub(super) fn ensure_cell(&mut self, cx: i32, cy: i32) {
        if self.dense_index(cx, cy).is_some() {
            return;
        }
        self.try_grow(cx, cy, cx, cy);
    }

    pub(super) fn slot(&self, idx: usize) -> Option<usize> {
        if idx < self.index.len() {
            let v = self.index[idx];
            if v != 0 {
                return Some((v - 1) as usize);
            }
        }
        self.sparse.get(&idx).copied().map(|s| s as usize)
    }

    pub(super) fn remember(&mut self, idx: usize, slot: u32) {
        if idx < self.index.len() {
            self.index[idx] = slot + 1;
            self.sparse.remove(&idx);
            return;
        }
        let gap = idx - self.index.len();
        if gap > (1 << 20) {
            self.sparse.insert(idx, slot);
            return;
        }
        self.index.resize(idx + 1, 0);
        self.index[idx] = slot + 1;
    }

    pub(super) fn ensure_slot(&mut self, idx: usize) -> usize {
        if let Some(slot) = self.slot(idx) {
            return slot;
        }
        let slot = if let Some(s) = self.free.pop() {
            s as usize
        } else {
            let s = self.bodies.len();
            self.bodies.push(empty_body());
            if self.wide {
                self.wide_pose.push(Wide::ZERO);
            }
            s
        };
        self.remember(idx, slot as u32);
        slot
    }

    pub(super) fn unlink_bucket(&mut self, cx: i32, cy: i32, target: u32, list: usize) {
        let mut prev = NONE;
        let mut cur = self.head_of(cx, cy, list);
        let guard = self.proxies.len() as u32 + 1;
        let mut n = 0u32;
        while cur != NONE {
            n += 1;
            if n > guard {
                break;
            }
            let next = self.proxies[cur as usize].bucket_next;
            if cur == target {
                if prev == NONE {
                    self.set_head(cx, cy, list, next);
                } else {
                    self.proxies[prev as usize].bucket_next = next;
                }
                break;
            }
            prev = cur;
            cur = next;
        }
    }

    pub(super) fn unlink(&mut self, slot: usize) {
        let mut p = self.bodies[slot].proxy;
        let guard = self.proxies.len() as u32 + 1;
        let mut n = 0u32;
        while p != NONE {
            n += 1;
            if n > guard {
                break;
            }
            let proxy = &self.proxies[p as usize];
            let next = proxy.owned_next;
            let cx = proxy.cx;
            let cy = proxy.cy;
            let list = proxy.list as usize;
            self.unlink_bucket(cx, cy, p, list);
            self.proxies[p as usize].bucket_next = self.free_proxy;
            self.proxies[p as usize].owned_next = NONE;
            self.free_proxy = p;
            p = next;
        }
        let b = &mut self.bodies[slot];
        b.proxy = NONE;
        b.linked = 0;
        self.touch_grid();
    }

    pub(super) fn alloc_proxy(&mut self, slot: u32, cx: i32, cy: i32, owned_next: u32, list: u8) -> u32 {
        let p = if self.free_proxy != NONE {
            let p = self.free_proxy;
            self.free_proxy = self.proxies[p as usize].bucket_next;
            p
        } else {
            let p = self.proxies.len() as u32;
            self.proxies.push(Proxy {
                slot,
                cx,
                cy,
                bucket_next: NONE,
                owned_next,
                list,
            });
            return p;
        };
        self.proxies[p as usize] = Proxy {
            slot,
            cx,
            cy,
            bucket_next: NONE,
            owned_next,
            list,
        };
        p
    }

    pub(super) fn link(&mut self, slot: usize) {
        let list: u8 = if self.bodies[slot].sensor != 0 { 1 } else { 0 };
        let (c0, r0, c1, r1) = self.span(slot);
        let mut owned = NONE;
        let mut cy = r0;
        loop {
            let mut cx = c0;
            loop {
                let p = self.alloc_proxy(slot as u32, cx, cy, owned, list);
                let li = list as usize;
                self.ensure_cell(cx, cy);
                self.proxies[p as usize].bucket_next = self.head_of(cx, cy, li);
                self.set_head(cx, cy, li, p);
                owned = p;
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
        let b = &mut self.bodies[slot];
        b.proxy = owned;
        b.c0 = c0;
        b.r0 = r0;
        b.c1 = c1;
        b.r1 = r1;
        b.linked = 1;
        self.touch_grid();
    }

    pub(super) fn link_if_changed(&mut self, slot: usize) {
        let linked = self.bodies[slot].linked;
        let (oc0, or0, oc1, or1) = (self.bodies[slot].c0, self.bodies[slot].r0, self.bodies[slot].c1, self.bodies[slot].r1);
        let (c0, r0, c1, r1) = self.span(slot);
        if linked != 0 && oc0 == c0 && or0 == r0 && oc1 == c1 && or1 == r1 {
            return;
        }
        self.unlink(slot);
        self.link(slot);
    }

    pub(super) fn retire(&mut self, slot: usize) {
        let b = &self.bodies[slot];
        if b.alive == 0 {
            return;
        }
        if b.sensor == 0 {
            self.solid_count = self.solid_count.saturating_sub(1);
        }
        self.unlink(slot);
    }

    pub(crate) fn absorb_extra_for_gpu(&mut self) {
        if self.extra.is_empty() {
            return;
        }
        let keys: Vec<(i32, i32)> = self.extra.keys().copied().collect();
        if keys.is_empty() {
            return;
        }
        for (cx, cy) in keys {
            self.try_grow(cx, cy, cx, cy);
        }
        self.touch_grid();
    }
}

pub(super) fn empty_body() -> Body {
    Body {
        x: 0.0,
        y: 0.0,
        ox: 0.0,
        oy: 0.0,
        hw: 0.0,
        hh: 0.0,
        group: 0,
        mask: 0,
        kind: KIND_RECT,
        sensor: 0,
        linked: 0,
        alive: 0,
        c0: 0,
        r0: 0,
        c1: 0,
        r1: 0,
        proxy: NONE,
        id: 0,
    }
}

#[inline]
pub(super) fn cell_of(v: f64, wide: bool) -> i32 {
    if wide {
        (v * INV_CELL_F64).floor() as i32
    } else {
        ((v as f32) * INV_CELL).floor() as i32
    }
}

#[inline]
pub(super) fn cell_of_f32(v: f32) -> i32 {
    (v * INV_CELL).floor() as i32
}

#[inline]
pub(super) fn cell_span_f32(x: f32, y: f32, ox: f32, oy: f32, hw: f32, hh: f32, kind: u8) -> (i32, i32, i32, i32) {
    let cx = x + ox;
    let cy = y + oy;
    let (hx, hy) = if kind == KIND_CIRCLE { (hw, hw) } else { (hw, hh) };
    (
        cell_of_f32(cx - hx),
        cell_of_f32(cy - hy),
        cell_of_f32(cx + hx),
        cell_of_f32(cy + hy),
    )
}

#[inline]
pub(super) fn cell_range(x: f64, y: f64, ox: f64, oy: f64, hw: f64, hh: f64, kind: u8, wide: bool) -> (i32, i32, i32, i32) {
    let cx = x + ox;
    let cy = y + oy;
    let (hx, hy) = if kind == KIND_CIRCLE { (hw, hw) } else { (hw, hh) };
    (
        cell_of(cx - hx, wide),
        cell_of(cy - hy, wide),
        cell_of(cx + hx, wide),
        cell_of(cy + hy, wide),
    )
}

#[derive(Clone, Copy)]
pub(super) struct GridPtrs {
    pub(super) bodies: *mut Body,
    pub(super) proxies: *mut Proxy,
    pub(super) heads: *mut CellHeads,
    pub(super) ox: i32,
    pub(super) oy: i32,
    pub(super) gw: i32,
    pub(super) gh: i32,
}

unsafe impl Send for GridPtrs {}
unsafe impl Sync for GridPtrs {}

pub(super) struct ProxyAlloc {
    pub(super) next: u32,
    pub(super) end: u32,
    pub(super) local_free: u32,
}

#[inline]
fn dense_slot(g: GridPtrs, cx: i32, cy: i32) -> Option<usize> {
    if g.gw <= 0 {
        return None;
    }
    let x = cx.wrapping_sub(g.ox);
    let y = cy.wrapping_sub(g.oy);
    if x < 0 || y < 0 || x >= g.gw || y >= g.gh {
        None
    } else {
        Some((y * g.gw + x) as usize)
    }
}

/// 调用方保证：同一格同一时刻只有一个线程进来，代理下标不和别的线程重叠。
pub(super) unsafe fn relink_planned(g: GridPtrs, alloc: &mut ProxyAlloc, slot: usize, span: (i32, i32, i32, i32)) {
    unsafe {
        unlink_raw(g, alloc, slot);
        link_raw(g, alloc, slot, span);
    }
}

unsafe fn unlink_raw(g: GridPtrs, alloc: &mut ProxyAlloc, slot: usize) {
    unsafe {
        let body = &mut *g.bodies.add(slot);
        let mut p = body.proxy;
        let guard = isize::MAX as u32;
        let mut n = 0u32;
        while p != NONE {
            n += 1;
            if n > guard {
                break;
            }
            let proxy = &mut *g.proxies.add(p as usize);
            let next = proxy.owned_next;
            let cx = proxy.cx;
            let cy = proxy.cy;
            let list = proxy.list as usize;
            if let Some(i) = dense_slot(g, cx, cy) {
                let head = &mut (*g.heads.add(i)).lists[list];
                let mut prev = NONE;
                let mut cur = *head;
                let mut steps = 0u32;
                while cur != NONE {
                    steps += 1;
                    if steps > guard {
                        break;
                    }
                    let cur_p = &mut *g.proxies.add(cur as usize);
                    let nxt = cur_p.bucket_next;
                    if cur == p {
                        if prev == NONE {
                            *head = nxt;
                        } else {
                            (*g.proxies.add(prev as usize)).bucket_next = nxt;
                        }
                        break;
                    }
                    prev = cur;
                    cur = nxt;
                }
            }
            let freed = &mut *g.proxies.add(p as usize);
            freed.bucket_next = alloc.local_free;
            freed.owned_next = NONE;
            alloc.local_free = p;
            p = next;
        }
        let body = &mut *g.bodies.add(slot);
        body.proxy = NONE;
        body.linked = 0;
    }
}

unsafe fn link_raw(g: GridPtrs, alloc: &mut ProxyAlloc, slot: usize, span: (i32, i32, i32, i32)) {
    unsafe {
        let (c0, r0, c1, r1) = span;
        let list: u8 = if (*g.bodies.add(slot)).sensor != 0 { 1 } else { 0 };
        let mut owned = NONE;
        let mut cy = r0;
        loop {
            let mut cx = c0;
            loop {
                let p = alloc_raw(g, alloc, slot as u32, cx, cy, owned, list);
                let i = dense_slot(g, cx, cy).expect("格子应先扩好");
                let head = &mut (*g.heads.add(i)).lists[list as usize];
                (*g.proxies.add(p as usize)).bucket_next = *head;
                *head = p;
                owned = p;
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
        let body = &mut *g.bodies.add(slot);
        body.proxy = owned;
        body.c0 = c0;
        body.r0 = r0;
        body.c1 = c1;
        body.r1 = r1;
        body.linked = 1;
    }
}

unsafe fn alloc_raw(g: GridPtrs, alloc: &mut ProxyAlloc, slot: u32, cx: i32, cy: i32, owned_next: u32, list: u8) -> u32 {
    unsafe {
        let p = if alloc.local_free != NONE {
            let p = alloc.local_free;
            alloc.local_free = (*g.proxies.add(p as usize)).bucket_next;
            p
        } else {
            let p = alloc.next;
            alloc.next += 1;
            debug_assert!(p < alloc.end);
            p
        };
        *g.proxies.add(p as usize) = Proxy {
            slot,
            cx,
            cy,
            bucket_next: NONE,
            owned_next,
            list,
        };
        p
    }
}
