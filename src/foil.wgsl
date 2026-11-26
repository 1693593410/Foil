// Foil 显卡碰撞。32px 密铺格子，一格两条链（0 实体，1 传感器）。
// 贴墙和重叠检测都在这里。传感器只走实体链，形状测试前跳过传感器对传感器。

const NONE: u32 = 0xFFFFFFFFu;
const CELL: f32 = 32.0;

struct Grid {
    ox: i32,
    oy: i32,
    gw: i32,
    gh: i32,
    n_body: u32,
    n_proxy: u32,
    n_in: u32,
    hit_cap: u32,
}

struct Body {
    x: f32,
    y: f32,
    ox: f32,
    oy: f32,
    hw: f32,
    hh: f32,
    group: u32,
    mask: u32,
    kind: u32,
    sensor: u32,
    id: u32,
    alive: u32,
}

struct Proxy {
    slot: u32,
    next: u32,
    _p0: u32,
    _p1: u32,
}

struct Mover {
    slot: u32,
    dx: f32,
    dy: f32,
    _pad: u32,
}

struct SlideOut {
    x: f32,
    y: f32,
    wall: u32,
    _pad: u32,
}

struct Hit {
    me: u32,
    other: u32,
    _p0: u32,
    _p1: u32,
}

@group(0) @binding(0) var<uniform> grid: Grid;
@group(0) @binding(1) var<storage, read_write> bodies: array<Body>;
@group(0) @binding(2) var<storage, read> proxies: array<Proxy>;
@group(0) @binding(3) var<storage, read> heads: array<u32>;
@group(0) @binding(4) var<storage, read> inputs: array<Mover>;
@group(0) @binding(5) var<storage, read_write> slide_outs: array<SlideOut>;
@group(0) @binding(6) var<storage, read_write> hit_count: atomic<u32>;
@group(0) @binding(7) var<storage, read_write> hit_outs: array<Hit>;

fn cell_of(v: f32) -> i32 {
    return i32(floor(v * (1.0 / CELL)));
}

fn head_at(cx: i32, cy: i32, list: u32) -> u32 {
    let x = cx - grid.ox;
    let y = cy - grid.oy;
    if (x < 0 || y < 0 || x >= grid.gw || y >= grid.gh) {
        return NONE;
    }
    let i = (u32(y) * u32(grid.gw) + u32(x)) * 2u + list;
    return heads[i];
}

fn groups_ok(a: Body, b: Body) -> bool {
    let a_ok = a.mask == 0u || b.group == 0u || ((a.mask & b.group) != 0u);
    let b_ok = b.mask == 0u || a.group == 0u || ((b.mask & a.group) != 0u);
    return a_ok && b_ok;
}

fn aabb_circle(rx: f32, ry: f32, hw: f32, hh: f32, cx: f32, cy: f32, radius: f32) -> bool {
    let px = clamp(cx, rx - hw, rx + hw);
    let py = clamp(cy, ry - hh, ry + hh);
    let dx = cx - px;
    let dy = cy - py;
    return dx * dx + dy * dy < radius * radius;
}

fn shapes(a: Body, ax: f32, ay: f32, b: Body, bx: f32, by: f32) -> bool {
    let acx = ax + a.ox;
    let acy = ay + a.oy;
    let bcx = bx + b.ox;
    let bcy = by + b.oy;
    if (a.kind == 0u && b.kind == 0u) {
        return (acx - a.hw) < (bcx + b.hw)
            && (acx + a.hw) > (bcx - b.hw)
            && (acy - a.hh) < (bcy + b.hh)
            && (acy + a.hh) > (bcy - b.hh);
    }
    if (a.kind == 1u && b.kind == 1u) {
        let dx = acx - bcx;
        let dy = acy - bcy;
        let r = a.hw + b.hw;
        return dx * dx + dy * dy < r * r;
    }
    if (a.kind == 0u && b.kind == 1u) {
        return aabb_circle(acx, acy, a.hw, a.hh, bcx, bcy, b.hw);
    }
    return aabb_circle(bcx, bcy, b.hw, b.hh, acx, acy, a.hw);
}

fn overlaps_solid(slot: u32, x: f32, y: f32) -> bool {
    let me = bodies[slot];
    if (me.sensor != 0u || me.alive == 0u) {
        return false;
    }
    let hx = me.hw;
    let hy = select(me.hw, me.hh, me.kind == 0u);
    let cx = x + me.ox;
    let cy = y + me.oy;
    let c0 = cell_of(cx - hx);
    let r0 = cell_of(cy - hy);
    let c1 = cell_of(cx + hx);
    let r1 = cell_of(cy + hy);
    let guard = grid.n_proxy + 1u;
    for (var cyi = r0; cyi <= r1; cyi++) {
        for (var cxi = c0; cxi <= c1; cxi++) {
            var p = head_at(cxi, cyi, 0u);
            var n = 0u;
            loop {
                if (p == NONE || n > guard || p >= grid.n_proxy) {
                    break;
                }
                n++;
                let proxy = proxies[p];
                let next = proxy.next;
                let other_slot = proxy.slot;
                if (other_slot != slot && other_slot < grid.n_body) {
                    let other = bodies[other_slot];
                    if (other.alive != 0u && other.sensor == 0u && groups_ok(me, other) && shapes(me, x, y, other, other.x, other.y)) {
                        return true;
                    }
                }
                p = next;
            }
        }
    }
    return false;
}

@compute @workgroup_size(64)
fn slide(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= grid.n_in) {
        return;
    }
    let job = inputs[id.x];
    let slot = job.slot;
    if (slot >= grid.n_body) {
        return;
    }
    let me = bodies[slot];
    var x = me.x;
    var y = me.y;
    var wall = 0u;
    let limit = max(min(me.hw, me.hh), 1.0);
    let dist = max(abs(job.dx), abs(job.dy));
    var steps = i32(ceil(dist / limit));
    steps = clamp(steps, 1, 8);
    let sx = job.dx / f32(steps);
    let sy = job.dy / f32(steps);
    for (var s = 0; s < steps; s++) {
        if (abs(sx) > 1e-8) {
            let tx = x + sx;
            if (overlaps_solid(slot, tx, y)) {
                wall = wall | select(2u, 1u, sx < 0.0);
            } else {
                x = tx;
            }
        }
        if (abs(sy) > 1e-8) {
            let ty = y + sy;
            if (overlaps_solid(slot, x, ty)) {
                wall = wall | select(8u, 4u, sy < 0.0);
            } else {
                y = ty;
            }
        }
    }
    slide_outs[id.x] = SlideOut(x, y, wall, 0u);
}

@compute @workgroup_size(64)
fn store(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= grid.n_in) {
        return;
    }
    let slot = inputs[id.x].slot;
    if (slot >= grid.n_body) {
        return;
    }
    let o = slide_outs[id.x];
    bodies[slot].x = o.x;
    bodies[slot].y = o.y;
}

fn push_hit(me: u32, other: u32) {
    let i = atomicAdd(&hit_count, 1u);
    if (i < grid.hit_cap) {
        hit_outs[i] = Hit(me, other, 0u, 0u);
    }
}

@compute @workgroup_size(64)
fn query(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= grid.n_in) {
        return;
    }
    let slot = inputs[id.x].slot;
    if (slot >= grid.n_body) {
        return;
    }
    let me = bodies[slot];
    if (me.alive == 0u) {
        return;
    }
    let lists = select(2u, 1u, me.sensor != 0u);
    let hx = me.hw;
    let hy = select(me.hw, me.hh, me.kind == 0u);
    let cx = me.x + me.ox;
    let cy = me.y + me.oy;
    let c0 = cell_of(cx - hx);
    let r0 = cell_of(cy - hy);
    let c1 = cell_of(cx + hx);
    let r1 = cell_of(cy + hy);
    let guard = grid.n_proxy + 1u;
    for (var list = 0u; list < lists; list++) {
        for (var cyi = r0; cyi <= r1; cyi++) {
            for (var cxi = c0; cxi <= c1; cxi++) {
                var p = head_at(cxi, cyi, list);
                var n = 0u;
                loop {
                    if (p == NONE || n > guard || p >= grid.n_proxy) {
                        break;
                    }
                    n++;
                    let proxy = proxies[p];
                    let next = proxy.next;
                    let other_slot = proxy.slot;
                    if (other_slot != slot && other_slot < grid.n_body) {
                        let other = bodies[other_slot];
                        let own = owner_cell(me, other);
                        let skip = other.alive == 0u || (me.sensor != 0u && other.sensor != 0u);
                        if (cxi == own.x && cyi == own.y && !skip && groups_ok(me, other) && shapes(me, me.x, me.y, other, other.x, other.y)) {
                            push_hit(me.id, other.id);
                        }
                    }
                    p = next;
                }
            }
        }
    }
}

fn half_of(b: Body) -> vec2<f32> {
    return vec2<f32>(b.hw, select(b.hw, b.hh, b.kind == 0u));
}

fn owner_cell(me: Body, other: Body) -> vec2<i32> {
    let mh = half_of(me);
    let oh = half_of(other);
    let mc = vec2<f32>(me.x + me.ox, me.y + me.oy);
    let oc = vec2<f32>(other.x + other.ox, other.y + other.oy);
    return vec2<i32>(
        max(cell_of(mc.x - mh.x), cell_of(oc.x - oh.x)),
        max(cell_of(mc.y - mh.y), cell_of(oc.y - oh.y)),
    );
}
