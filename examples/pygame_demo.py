"""pygame 只负责画。物体和查询都在 Foil.Foil() 里，走 CPU。Esc 退出。

    python foil/examples/pygame_demo.py
"""

import time

import Foil
import pygame

W, H = 1280, 720
BODY = 6.0
CIRCLE = 10.0
CIRCLES = 500
SENSORS = 36
COLS, ROWS = 50, 40
WALLS = (
    (8.0, H * 0.5, 16.0, float(H)),
    (W - 8.0, H * 0.5, 16.0, float(H)),
    (W * 0.5, 8.0, float(W), 16.0),
    (W * 0.5, H - 8.0, float(W), 16.0),
)


class Mover:
    def __init__(self, fid, x, y, vx, vy, shape, sensor, radius):
        self.fid = fid
        self.x = x
        self.y = y
        self.px = x
        self.py = y
        self.vx = vx
        self.vy = vy
        self.shape = shape
        self.sensor = sensor
        self.radius = radius
        self.hit = False


def build():
    world = Foil.Foil()
    for i, (x, y, w, h) in enumerate(WALLS):
        world.add(i, x, y, w, h, "rect", False, 1, 1)
    movers = []
    fid = len(WALLS)
    dx = (W - 80.0) / COLS
    dy = (H - 80.0) / ROWS
    for i in range(COLS * ROWS):
        col, row = i % COLS, i // COLS
        x = 40.0 + (col + 0.5) * dx
        y = 40.0 + (row + 0.5) * dy
        vx = ((i * 17) % 121) - 60.0
        vy = ((i * 31) % 121) - 60.0
        if abs(vx) < 24.0:
            vx = 70.0
        if abs(vy) < 24.0:
            vy = -70.0
        world.add(fid, x, y, BODY, BODY, "rect", False, 1, 1)
        movers.append(Mover(fid, x, y, float(vx), float(vy), "rect", False, BODY * 0.5))
        fid += 1
    for i in range(CIRCLES):
        col, row = i % 25, i // 25
        gap_c = col * 49 // 25
        gap_r = row * 39 // 20
        x = 40.0 + (gap_c + 1) * dx
        y = 40.0 + (gap_r + 1) * dy
        vx = 80.0 if i % 2 == 0 else -65.0
        vy = -75.0 if i % 3 == 0 else 55.0
        world.add(fid, x, y, CIRCLE, CIRCLE, "circle", False, 1, 1)
        movers.append(Mover(fid, x, y, vx, vy, "circle", False, CIRCLE * 0.5))
        fid += 1
    for i in range(SENSORS):
        col = i % 9
        row = i // 9
        x = 180.0 + col * 110.0
        y = 140.0 + row * 120.0
        vx = 90.0 if i % 2 == 0 else -90.0
        vy = 60.0 if i % 3 else -80.0
        world.add(fid, x, y, 18.0, 18.0, "circle", True, 1, 1)
        movers.append(Mover(fid, x, y, vx, vy, "circle", True, 9.0))
        fid += 1
    return world, movers


def step(world, movers, dt):
    for m in movers:
        m.px, m.py = m.x, m.y
        m.x = min(W - 20.0, max(20.0, m.x + m.vx * dt))
        m.y = min(H - 20.0, max(20.0, m.y + m.vy * dt))
        if m.x <= 20.0 or m.x >= W - 20.0:
            m.vx = -m.vx
        if m.y <= 20.0 or m.y >= H - 20.0:
            m.vy = -m.vy
        m.hit = False
        world.set_pose(m.fid, m.x, m.y)
    by_id = {m.fid: m for m in movers}
    t0 = time.perf_counter()
    pairs = world.query([m.fid for m in movers])
    ms = (time.perf_counter() - t0) * 1000.0
    blocked = set()
    for a, b in pairs:
        ma = by_id.get(int(a))
        mb = by_id.get(int(b))
        if ma is not None:
            ma.hit = True
        if mb is not None:
            mb.hit = True
        if ma is not None and ma.sensor:
            continue
        if mb is not None and mb.sensor:
            continue
        blocked.add(int(a))
    for fid in blocked:
        m = by_id.get(fid)
        if m is None:
            continue
        m.x, m.y = m.px, m.py
        m.vx, m.vy = -m.vx, -m.vy
        world.set_pose(m.fid, m.x, m.y)
    return ms, len(pairs)


def draw(screen, font, movers, ms, pairs, backend):
    screen.fill((18, 20, 26))
    for x, y, w, h in WALLS:
        pygame.draw.rect(screen, (42, 51, 64), (x - w * 0.5, y - h * 0.5, w, h))
    half = BODY * 0.5
    for m in movers:
        if m.shape == "circle":
            if m.sensor:
                color = (232, 168, 96) if m.hit else (120, 176, 168)
            else:
                color = (186, 214, 255) if m.hit else (78, 132, 210)
            pygame.draw.circle(screen, color, (int(m.x), int(m.y)), int(m.radius))
        else:
            color = (214, 92, 78) if m.hit else (215, 196, 163)
            pygame.draw.rect(screen, color, (m.x - half, m.y - half, BODY, BODY))
    label = font.render(
        f"{backend}  {COLS * ROWS} 矩形 + {CIRCLES} 圆实体 + {SENSORS} 传感器   查询 {ms:.2f} ms   重叠 {pairs}",
        True,
        (230, 230, 230),
    )
    screen.blit(label, (16, 18))


def main():
    pygame.init()
    screen = pygame.display.set_mode((W, H))
    pygame.display.set_caption("foil pygame")
    font = pygame.font.SysFont("microsoftyahei", 18)
    clock = pygame.time.Clock()
    world, movers = build()
    running = True
    while running:
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                running = False
            elif event.type == pygame.KEYDOWN and event.key == pygame.K_ESCAPE:
                running = False
        dt = min(clock.tick(60) / 1000.0, 0.05)
        ms, pairs = step(world, movers, dt)
        draw(screen, font, movers, ms, pairs, world.backend())
        pygame.display.flip()
    pygame.quit()


if __name__ == "__main__":
    main()
