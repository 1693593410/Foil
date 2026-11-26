# Foil

Foil is a lightweight 2D game physics engine. It aims for speed, a small footprint, and playability, and it does not try to be a complex physics system. It is meant for most 2D games.

It currently supports overlap tests for axis-aligned rectangles and circles, plus sensors, groups, and masks.

API reference: [docs/groups.html](https://gitee.com/longyateng/dargon/blob/foil/foil/docs/groups.html).

Notes

1. The project is still in development. Bug reports and issues are welcome.
2. GPU acceleration is experimental. The transfer cost is much larger than the speedup, so it is not recommended for now.

# 中文

Foil 是一个轻量级 2D 游戏物理引擎，以速度、轻量、可玩性为目标，不追求复杂的物理系统，主要面向大多数 2D 游戏场景。

目前完成了轴对齐矩形和圆的重叠检测、传感器、分组和掩码。

接口说明见 [docs/groups.html](https://gitee.com/longyateng/dargon/blob/foil/foil/docs/groups.html)。

注

1. 项目正处在开发阶段，如遇到 Bug，欢迎提交反馈 / Issue。
2. GPU 加速正属于实验性功能，传输开销远大于带来的性能提升，暂时不推荐使用。
