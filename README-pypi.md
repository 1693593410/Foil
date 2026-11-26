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

API reference: [GitHub](https://github.com/1693593410/Foil).

Notes

1. The project is still in development. Bug reports and issues are welcome.
2. GPU acceleration is experimental. The transfer cost is much larger than the speedup, so it is not recommended for now.
