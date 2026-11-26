//! 一万个矩形。库只负责检测，位置在这个例子里改。Esc 退出。
//!
//! cargo run --release --manifest-path foil/Cargo.toml --example crowd

use std::time::{Duration, Instant};

use Foil::Foil;
use minifb::{Key, Window, WindowOptions};

const W: usize = 1280;
const H: usize = 720;
const MOVERS: usize = 10_000;
const BODY: f32 = 4.0;
const FIRST: usize = 4;

const WALLS: [(f32, f32, f32, f32); 4] = [
    (8.0, 360.0, 16.0, 720.0),
    (1272.0, 360.0, 16.0, 720.0),
    (640.0, 8.0, 1280.0, 16.0),
    (640.0, 712.0, 1280.0, 16.0),
];

fn main() {
    let mut world = Foil::new();
    for (i, &(x, y, w, h)) in WALLS.iter().enumerate() {
        world
            .add(i, x as f64, y as f64, w as f64, h as f64, "rect", false, 1, 1)
            .expect("墙");
    }
    let mut pos = vec![(0.0f32, 0.0f32); MOVERS];
    let mut vel = vec![(0.0f32, 0.0f32); MOVERS];
    let dx = (W as f32 - 56.0) / 100.0;
    let dy = (H as f32 - 56.0) / 100.0;
    for i in 0..MOVERS {
        let col = (i % 100) as f32;
        let row = (i / 100) as f32;
        let x = 28.0 + (col + 0.5) * dx;
        let y = 28.0 + (row + 0.5) * dy;
        let vx = ((i * 17) % 121) as f32 - 60.0;
        let vy = ((i * 31) % 121) as f32 - 60.0;
        let vx = if vx.abs() < 24.0 { 48.0 } else { vx };
        let vy = if vy.abs() < 24.0 { -48.0 } else { vy };
        world
            .add(FIRST + i, x as f64, y as f64, BODY as f64, BODY as f64, "rect", false, 1, 1)
            .expect("物体");
        pos[i] = (x, y);
        vel[i] = (vx, vy);
    }

    let mut window = Window::new("foil  10000", W, H, WindowOptions::default()).expect("窗口");
    #[allow(deprecated)]
    window.limit_update_rate(Some(Duration::from_micros(16_600)));
    let mut buf = vec![0u32; W * H];

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let t = Instant::now();
        let dt = 1.0 / 60.0;
        for i in 0..MOVERS {
            let id = FIRST + i;
            let (x, y) = pos[i];
            let (vx, vy) = vel[i];
            let nx = (x + vx * dt).clamp(20.0, 1260.0);
            let ny = (y + vy * dt).clamp(20.0, 700.0);
            world.set_pose(id, nx as f64, ny as f64);
            let hit_wall = world.overlaps(id).iter().any(|o| *o < FIRST);
            if hit_wall {
                world.set_pose(id, x as f64, y as f64);
                vel[i] = (-vx, -vy);
            } else {
                pos[i] = (nx, ny);
                if nx <= 20.0 || nx >= 1260.0 {
                    vel[i].0 = -vx;
                }
                if ny <= 20.0 || ny >= 700.0 {
                    vel[i].1 = -vy;
                }
            }
        }
        let step_ms = t.elapsed().as_secs_f64() * 1000.0;

        buf.fill(0x0012141a);
        for &(x, y, w, h) in &WALLS {
            fill(&mut buf, x, y, w, h, 0x002a3340);
        }
        for i in 0..MOVERS {
            let (x, y) = pos[i];
            fill(&mut buf, x, y, BODY, BODY, 0x00d7c4a3);
        }
        window.set_title(&format!("foil  {step_ms:.2} ms  {MOVERS}"));
        window.update_with_buffer(&buf, W, H).expect("画到窗口");
    }
}

fn fill(buf: &mut [u32], cx: f32, cy: f32, w: f32, h: f32, color: u32) {
    let x0 = ((cx - w * 0.5).floor() as i32).max(0) as usize;
    let y0 = ((cy - h * 0.5).floor() as i32).max(0) as usize;
    let x1 = ((cx + w * 0.5).ceil() as i32).clamp(0, W as i32) as usize;
    let y1 = ((cy + h * 0.5).ceil() as i32).clamp(0, H as i32) as usize;
    for y in y0..y1 {
        let row = y * W;
        for x in x0..x1 {
            buf[row + x] = color;
        }
    }
}
