# Windows 使用说明

从 [Releases](https://github.com/StevenArai/msutop/releases/latest) 下载
`msutop-v0.1.0-windows-x86_64.zip`，完整解压后双击 `00-PREVIEW.cmd`，或在该目录运行：

```powershell
.\msutop.exe
```

发布包包含 `msutop.exe`、`SDL2.dll`、启动脚本、说明和许可证，不需要安装 Rust、
Python 或 Visual C++ 运行库。请将 `SDL2.dll` 留在 EXE 旁边。构建使用 Rust 1.88
的 x86_64 Windows GNU 工具链和官方 MinGW SDL2。

Windows 目前使用 mock 监控数据，用于调试完整 160×80 界面；真实 `/proc`、`/sys`
采集在 Linux 上由 `--live` 开启。Windows 可以将 mock 画面通过同一串口协议发给屏幕。

## 预览参数

```powershell
.\msutop.exe --scale 6 --fps 30 --mock-threads 4
.\msutop.exe --scene cpu --theme vivid
.\msutop.exe --network nocarrier
.\msutop.exe --no-aa
.\msutop.exe --help
```

`--scene` 可选 `balanced`、`idle`、`cpu`、`io`、`memory`、`offline`。
`--mock-threads N` 可预览 1..512 个逻辑线程的响应式 CPU 柱布局。
`--scale` 是 1..16 的整数最近邻放大。默认数据更新 30Hz，窗口跟随显示器 VSync
呈现（如 60Hz）；所有监控区始终同时可见。

| 按键 | 操作 |
|---|---|
| 空格 | 暂停／继续 |
| 1..6 | 手动选择 mock 工作负载 |
| N | 切换网卡状态 |
| T | 切换默认／鲜亮 CPU 配色 |
| A | 切换曲线、柱和环的抗锯齿 |
| Q | 比较 RGB565／RGB888 |
| + / - | 改变整数放大倍数 |
| Esc | 关闭窗口 |

## 接 MSU 屏幕

确认设备的实际 COM 号，屏幕须支持 R / RGB565 v1 协议，并处于终端模式：

```powershell
.\msutop.exe --msu COM5 --fps 30 --seconds 30
```

上面只用 `COM5` 举例，需要替换成你的端口。续传到仍处于已确认 R 模式的同一设备：

```powershell
.\msutop.exe --msu COM5 --msu-ready --seconds 30
```

传输使用 RAW FULL + pipeline，每帧覆盖 160×80，保留 RGB565 颜色与 AA。
每帧十条带和 COMMIT 的 11 个 ACK 必须全部匹配。退出保留最后画面；错误或切换
二进制模式后须复位回终端重新握手。程序不会自动烧录固件。

`--seconds N` 或 `--frames N` 可限制运行时间；没有这两个参数时串口模式只发一帧。
具体线格式见 [rgb565.md](rgb565.md)。

## 输出帧

```powershell
.\msutop.exe --headless --rgb565 frames.rgb565 --fps 30 --seconds 10
.\msutop.exe --export output
```

每帧是 25,600 字节的行优先、大端 RGB565；导出同时生成 BMP。
运行性能可用 `--profile performance.json --seconds 10` 保存。

## 从源码构建

安装 Git 和 Rustup 后，在独立仓库根目录运行：

```powershell
.\scripts\preview.ps1 -BuildOnly -Check
.\target\release\msutop.exe
```

脚本安装 GNU Rust 工具链、下载校验过的 MinGW SDL2，并启用 `sdl,serial`。
完整发布包可运行 `scripts/package-windows.ps1` 重建。

缺少 `SDL2.dll` 时重新完整解压下载包；串口握手超时时确认 COM 号、设备状态，
并在设备复位回终端后省略 `--msu-ready`。`--live` 在 Windows 不可用。
