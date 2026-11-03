# msutop Mac

原版渲染核心 + macOS 原生数据采集 + 桌面悬浮窗口。此目录是独立的 macOS 构建入口，不加入根 Cargo 包的依赖或功能，也不改变 Linux/Windows 的采集、渲染和构建方式。

默认显示原版 **160×80** 完整面板，宽高比 2:1。菜单中的「显示 GPU 与多硬盘扩展」默认关闭；开启后在右侧追加 **96×80** 面板。两侧采用同一像素倍率，扩展后的画面为 256×80，宽高比 3.2:1。开关只增减右侧区域，原版主面板的字体、布局和动画保持一致。

- 原版主面板：CPU 曲线、每核柱、RAM/Swap 双环、磁盘 IO、网络和两个主要进程。
- 可选右侧扩展：一行紧凑的 GPU 数值/曲线，以及四个本地卷的名称、使用比例、容量和进度条。
- 主面板保留原版字体与布局：进程区为 **6×8**，其余紧凑数值为 **3×5**。开启扩展也不会切换主面板字体。CPU 右上角为利用率/功率、温度/主频；传感器尚未就绪或采集失败时显示 `--W`、`--C`、`--G`。卷标绕过低分辨率画布独立叠加，点阵像素采用主画面像素倍率的 2/3：英文使用 Adafruit **6×8** 点阵，日文使用 [k6x8 Gothic](https://littlelimit.net/k6x8.htm) 原生 **6×8** 点阵。所有字形按屏幕物理像素对齐，不使用系统平滑字体或轮廓缩图。文字层按名称/大小/屏幕倍率缓存。

30 Hz 采集与 60 FPS 显示分开运行。曲线使用原版固定时间桶、亚像素位移、抗锯齿和渐变；核心柱、双环、活动灯和进程稳定选择也复用原算法。默认曲线覆盖约 4.23 秒，启动后从右侧逐渐积累真实历史。`Sources/canvases.rs` 直接包含根目录的 `src/ui.rs`，在它旁边新增可选扩展；原版渲染源文件保持原样。

## 使用

打开 `~/Applications/msutop Mac.app`，或本目录下 `build/msutop Mac.app`。

- 拖动移动；右键、右上角或菜单栏图标打开设置。
- **始终置顶 / 贴在桌面**：切换窗口层级。
- 支持位置锁定、隐藏、大小调整、移动显示器、鲜亮配色和 30/60 FPS。
- **显示 GPU 与多硬盘扩展**：勾选后向右展开 GPU/多卷信息，取消后恢复原版 160×80 面板；默认关闭，选择会保存。
- **原版演示（模拟数据）**：使用上游演示负载并明确标记 `DEMO`；GPU 和本地卷容量仍来自 Mac。默认使用真实监控，演示模式不持久保存。
- 位置、大小、模式和动画帧率会保存；从菜单退出。不自动添加登录项。

## 数据口径

| 指标 | 来源 |
|---|---|
| CPU/每核 | Mach `host_processor_info` 的区间差值；总 CPU 为全部核心平均占用 |
| CPU 功率 | [macmon](https://github.com/vladkens/macmon) 的 IOReport/CLPC 能量差分估算，单位 W；macOS 27 使用严格 CLPC 读取，避免把冻结的旧能量计数当作 0 W |
| CPU 温度 | macmon 通过 AppleSMC 读取有效 CPU 温度传感器的平均值，单位 °C |
| CPU 主频 | 最高性能 CPU 核心组的平均活动频率，单位 MHz，界面以 GHz 显示；空闲时采用该组最低档位。三层核心的机型采用最高性能组 |
| GPU | IOKit `IOAccelerator/PerformanceStatistics`；驱动不支持时显示 `--` |
| RAM | Mach `host_statistics64`，App（internal − purgeable）+ wired + compressor；GiB |
| Swap | `sysctl vm.swapusage`；GiB |
| 网络 | `NET_RT_IFLIST2` 的 64 位计数；优先物理默认路由 `en*`，无物理默认路由时合计活跃 `en*`，不重复统计 VPN/桥接接口 |
| 磁盘容量 | Foundation，total − available，显示前四个本地卷；十进制 GB。APFS 按共享容器可用空间计算 |
| 磁盘读写 | IOKit `IOBlockStorageDriver`，按设备 ID 做区间差值再合计；热插拔不引入历史流量尖峰 |
| 进程 | `proc_pid_rusage` 的 Mach 时间差；100% 表示一个核心，RSS 按二进制单位；每 0.5 秒扫描候选，显示值平滑到 30 Hz，并按原版策略避免名称频繁交换 |

沿用上游紧凑速率格式：`K/M/G` 按 1024 进位，网络/磁盘表示每秒字节数；进程末列为 RSS。IOKit/GPU 和 Swap 每 0.25 秒更新，磁盘容量每 15 秒更新。CPU 功率/温度/频率每 0.5 秒更新，并使用原版平滑显示。不需要管理员权限，不发送监控数据。

硬件传感器通过随附的 `msutop-sensors` 只读子进程采集。它与主界面隔离，退出应用时一并退出，不安装服务或登录项。单项缺失显示 `--`；数据超过 3 秒未更新会失效。功耗采集要求有效的 CLPC 通道；旧系统或硬件不支持时传感器辅助进程会失败并重试，主界面照常工作。子进程异常会自动重试，跨睡眠的计数差分会丢弃并重新建立基线。macmon 的库代码固定在 `vendor/macmon/UPSTREAM.txt` 记录的提交，仅编译采集库，不包含 TUI、HTTP 服务或写控制功能。

## 构建与验证

macOS 13+、Apple Command Line Tools、Rust 1.90+。功率、温度、主频采集针对 Apple Silicon；Intel Mac 和虚拟机可运行主界面，不支持的传感器显示 `--`。构建脚本会从随项目附带的 BDF 点阵源打包 k6x8 字库；不安装系统字体。Rust 只用于构建，应用静态包含渲染器，运行不需要 Rust/SDL/浏览器。构建脚本优先使用项目缓存的工具链（如果存在），否则使用 PATH 中的 Rust。

```sh
cd macos
./scripts/build-renderer.sh --test
./scripts/build.sh
python3 scripts/smoke-test.py
./scripts/install.sh
build/msutop-mac --probe 4
build/msutop-mac --render "$PWD/output/preview.png"
build/msutop-mac --render "$PWD/output/preview-extended.png" --extensions
```

测试包含 600 帧原版像素对照（默认原版模式与可选扩展模式的主面板均逐帧一致，涵盖字体、CPU 四项、进程曲线和边框）、没有新采样时两画布仍持续滚动、不等宽 FFI 输出边界检查、上游抗锯齿/历史/量程/圆环测试，以及真实单核负载和传感器范围校验。CPU 三项已在 Apple Silicon 真机上以非管理员权限实测，另已验证采集中断后数据失效及自动恢复。`runtime.json` 位于 `~/Library/Caches/msutop-mac/`，记录实际采样/绘制频率。

应用本地 ad-hoc 签名。为避开 iCloud/File Provider 的 FinderInfo 属性，签名包生成在 `~/Library/Caches/msutop-mac/`，`build/msutop Mac.app` 是符号链接。安装脚本复制到 `~/Applications/`；更新前先退出正在运行的版本。

源代码见 `Sources/`；授权与上游字体来源说明见 `NOTICE.txt` 和 `licenses/`。

## 平台隔离

根目录的 `Cargo.toml`、`Cargo.lock`、`build.rs`、`src/` 和原有平台脚本不需要修改。Mac 专用 Swift/AppKit、C/IOKit、Rust FFI、传感器依赖、日文字库和许可证全部在 `macos/` 内。`Sensors/Cargo.toml` 是独立 Cargo 包，其依赖进一步限定在 `cfg(target_os = "macos")`。构建脚本在非 Darwin 主机直接退出。

CI 在 Linux 和 Windows 上使用根项目及其原有特性构建、测试；Mac 作业单独构建 `.app` 并比较原版像素。Mac 的 Rust 1.90 要求不会提高根项目的 Rust 1.88 要求。
