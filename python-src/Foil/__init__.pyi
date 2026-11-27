class Foil:
    """二维重叠检测。

    调用方用 `set_pose` 摆位置，用 `overlaps` 或 `query` 取重叠结果。
    边贴着不算重叠。`mask == 0` 会碰上所有人，对方 `group == 0` 也会碰上。
    0 不是「谁都不碰」。
    """

    def __init__(self, backend: str = "cpu", real: str = "f32", threads: int = 16) -> None:
        """创建世界。

        backend 可以是 cpu、dx12（也认 d3d12、directx12）、vulkan、metal、
        gl（也认 opengl、gles、angle），以及 wgpu、webgpu、gpu。
        real 写 f32 或 f64。f64 只能配 cpu。
        threads 是批量查询的线程数。写多少用多少。0 表示用进程里现成的线程池。
        被查物体少于 192 个时始终单线程。默认 16。
        显卡查询是实验功能，传输开销目前大于收益。
        """

    def backend(self) -> str:
        """创建时选定的后端名字。某次查询改走 CPU 时，这个字符串不变。"""

    def real(self) -> str:
        """返回 \"f32\" 或 \"f64\"。"""

    def threads(self) -> int:
        """当前线程上限。0 表示用进程里的线程池。少于 192 个物体时查询仍是单线程。"""

    def set_threads(self, n: int) -> None:
        """改线程上限。1 始终单线程。下次批量查询才按新的数量建池。"""

    def set_timing(self, on: bool) -> None:
        """打开后，query 才累计显卡检测耗时。CPU 后端没有这个数。"""

    def detect_ns(self) -> int:
        """历次显卡检测耗时合计，单位纳秒。"""

    def add(
        self,
        idx: int,
        x: float,
        y: float,
        w: float,
        h: float,
        shape: str = "rect",
        sensor: bool = False,
        group: int = 0,
        mask: int = 0,
    ) -> None:
        """放进一个物体。

        idx 由调用方指定。x、y 是中心，w、h 是宽高。
        shape 只接受 rect 和 circle。圆的半径是宽高里较大的一半。
        形状写错，或编号放不进 32 位，抛出 ValueError，物体不会放进去。
        sensor 为真时只参与检测。mask 为 0 碰上所有人，对方 group 为 0 也碰上。
        """

    def remove(self, idx: int) -> None:
        """按编号删掉。编号不在世界里时什么也不做。"""

    def clear(self) -> None:
        """清空全部物体和格子。"""

    def set_pose(self, idx: int, x: float, y: float) -> None:
        """改中心。不存在的编号会被忽略。"""

    def move_by(self, idx: int, dx: float, dy: float) -> tuple[float, float]:
        """沿位移贴着别的实体滑，并写进新中心。先水平，再竖直。

        空地上每轴只扫一次格子。边贴着可以停。传感器不挡人，自己也不会被挡住。
        编号不存在时返回 (0, 0)，世界不变。
        """

    def pose(self, idx: int) -> tuple[float, float] | None:
        """读中心。编号不存在时返回 None。"""

    def set_size(self, idx: int, w: float, h: float, shape: str = "") -> None:
        """改宽高。shape 为空字符串时沿用现在的形状。形状名写错才抛出 ValueError。"""

    def size(self, idx: int) -> tuple[float, float] | None:
        """读宽高。编号不存在时返回 None。"""

    def shape(self, idx: int) -> str | None:
        """返回 \"rect\" 或 \"circle\"。编号不存在时返回 None。"""

    def set_group(self, idx: int, group: int) -> None:
        """改自己的分组位。不存在的编号会被忽略。"""

    def group(self, idx: int) -> int | None:
        """读分组位。编号不存在时返回 None。"""

    def set_mask(self, idx: int, mask: int) -> None:
        """改掩码。0 表示碰上所有人。不存在的编号会被忽略。"""

    def mask(self, idx: int) -> int | None:
        """读掩码。编号不存在时返回 None。"""

    def set_sensor(self, idx: int, sensor: bool) -> None:
        """改成传感器或实体。传感器互相看不见，实体仍能看见传感器。"""

    def is_sensor(self, idx: int) -> bool:
        """是否为传感器。编号不存在时返回 False。"""

    def count(self) -> int:
        """还在世界里的物体个数，含传感器。"""

    def len(self) -> int:
        """实体个数。传感器不算。"""

    def set_offset(self, idx: int, ox: float, oy: float) -> None:
        """形状中心相对 set_pose 的偏移。不存在的编号会被忽略。"""

    def overlaps(self, idx: int) -> list[int]:
        """用存着的位置，返回重叠到的编号。边贴着不算。传感器不会列出别的传感器。"""

    def query(self, indices: list[int]) -> list[tuple[int, int]]:
        """一次查很多个编号。

        每一对是 (被查询的编号, 对方)。两边都在名单里时，两个方向都留。
        选了显卡且精度是 f32 时先走显卡，查不了再整次改走 CPU。
        """
