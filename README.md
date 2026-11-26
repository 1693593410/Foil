# Foil

Foil is a lightweight 2D game physics engine. It aims for performance, a small footprint, simplicity, and playability. It does not try to be a complex physics system, and it is meant for most 2D games.

So far it only has overlap tests for axis-aligned rectangles and circles, plus sensors, groups, and masks.

This project is released under the MIT license. You can use, modify, and redistribute it.

## Install

Rust:

```toml
rustfoil2d = "0.1.0"
```

```rust
use Foil::Foil;

let world = Foil::new();
```

Python 3.13 or newer. The published wheel is for 64-bit Windows and CPython 3.13. Other systems build from the source package and need Rust.

```
pip install pyfoil2d
```

```python
import Foil

world = Foil.Foil()
```

API reference: [docs/groups.html](docs/groups.html).

Notes

1. The project is still in development. Bug reports and issues are welcome.
2. GPU acceleration is experimental. The transfer cost is much larger than the speedup, so it is not recommended for now.

# 中文

Foil 是一个轻量级 2D 游戏物理引擎，以性能、轻量、简单、可玩性为目标，不追求复杂的物理系统，主要面向大多数 2D 游戏场景。

目前仅完成轴对齐矩形和圆的重叠检测、传感器、分组和掩码。

本项目采用 MIT 协议，可以自由使用、修改和再发布。

## 安装

Rust：

```toml
rustfoil2d = "0.1.0"
```

```rust
use Foil::Foil;

let world = Foil::new();
```

Python 需要 3.13 或更高版本。已经发布的轮子是 64 位 Windows、CPython 3.13。其他系统会用源码包编译，需要本机有 Rust。

```
pip install pyfoil2d
```

```python
import Foil

world = Foil.Foil()
```

接口说明见 [docs/groups.html](docs/groups.html)。

注

1. 项目正处在开发阶段，如遇到 Bug，欢迎提交反馈 / Issue。
2. GPU 加速正属于实验性功能，传输开销远大于带来的性能提升，暂时不推荐使用。
