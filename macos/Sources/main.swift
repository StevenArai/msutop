import AppKit
import Darwin

let appName = "msutop Mac"
let preferences = UserDefaults.standard
let canvasSize = NSSize(width: 320, height: 160)
let gib = 1_073_741_824.0

func cString<T>(_ value: T) -> String {
    withUnsafeBytes(of: value) { bytes in
        String(decoding: bytes.prefix(while: { $0 != 0 }), as: UTF8.self)
    }
}
func percent(_ value: Double) -> String { String(format: "%.1f%%", value) }
func capacity(_ value: UInt64) -> String { String(format: "%.1f", Double(value) / gib) }
func rate(_ value: Double) -> String {
    let units = ["B/s", "KiB/s", "MiB/s", "GiB/s"]
    var v = max(0, value), i = 0
    while v >= 1024 && i < 3 { v /= 1024; i += 1 }
    return String(format: v < 10 && i > 0 ? "%.1f%@" : "%.0f%@", v, units[i])
}
func uptime(_ value: Double) -> String {
    let hours = max(0, Int(value)) / 3600
    return hours >= 24 ? String(format: "%02dd %02dh", hours / 24, hours % 24) : String(format: "%02dh %02dm", hours, Int(value) / 60 % 60)
}

struct Volume: Codable {
    var name: String
    var used: UInt64
    var total: UInt64
    var path: String
    var ratio: Double { total > 0 ? Double(used) / Double(total) : 0 }
}
func volumes() -> [Volume] {
    let keys: Set<URLResourceKey> = [.volumeNameKey, .volumeTotalCapacityKey, .volumeAvailableCapacityKey, .volumeIsLocalKey]
    let root = URL(fileURLWithPath: "/")
    let mounted = FileManager.default.mountedVolumeURLs(includingResourceValuesForKeys: Array(keys), options: [.skipHiddenVolumes]) ?? []
    var seen = Set<String>(), result: [Volume] = []
    for url in [root] + mounted {
        guard url.path == "/" || url.path.hasPrefix("/Volumes/") else { continue }
        guard seen.insert(url.path).inserted,
              let info = try? url.resourceValues(forKeys: keys), info.volumeIsLocal == true,
              let total = info.volumeTotalCapacity, total > 0,
              let available = info.volumeAvailableCapacity else { continue }
        result.append(Volume(name: info.volumeName ?? url.lastPathComponent,
                             used: UInt64(max(0, total - available)), total: UInt64(total), path: url.path))
    }
    return result
}
struct ProcessRow: Codable {
    var name: String
    var cpu: Double
    var memory: UInt64
    var pid: Int32
}
struct Reading: Codable {
    var cpuUser: Double, cpuSystem: Double, cores: [Double]
    var cpuAvailable: Bool
    var memoryTotal: UInt64, memoryUsed: UInt64, cached: UInt64, wired: UInt64, compressed: UInt64
    var memoryAvailable: Bool, swapAvailable: Bool
    var swapUsed: UInt64, swapTotal: UInt64
    var gpu: Double, gpuAvailable: Bool
    var download: Double, upload: Double, networkAvailable: Bool, interfaces: String
    var diskRead: Double, diskWrite: Double, diskIOAvailable: Bool, uptime: Double
    var cpuPowerWatts: Double?, cpuTemperatureC: Double?, cpuFrequencyMHz: Double?
    var cpu: Double { cpuUser + cpuSystem }
    var memoryRatio: Double { memoryTotal > 0 ? Double(memoryUsed) / Double(memoryTotal) : 0 }
    init() {
        self.init(sample: nativeSample())
    }
    init(sample s: MonitorSample) {
        cpuUser = s.cpu_user; cpuSystem = s.cpu_system; cpuAvailable = s.cpu_available != 0
        cores = withUnsafeBytes(of: s.cores) { Array($0.bindMemory(to: Double.self).prefix(Int(s.core_count))) }
        memoryTotal = s.memory_total; memoryUsed = s.memory_used; cached = s.memory_cached
        wired = s.memory_wired; compressed = s.memory_compressed; memoryAvailable = s.memory_available != 0
        swapUsed = s.swap_used; swapTotal = s.swap_total; swapAvailable = s.swap_available != 0
        gpu = s.gpu_usage; gpuAvailable = s.gpu_available != 0
        download = s.download; upload = s.upload; networkAvailable = s.network_available != 0; interfaces = cString(s.interfaces)
        diskRead = s.disk_read; diskWrite = s.disk_write; diskIOAvailable = s.disk_io_available != 0; uptime = s.uptime
        cpuPowerWatts = s.cpu_sensor_flags & 1 != 0 ? s.cpu_power_watts : nil
        cpuTemperatureC = s.cpu_sensor_flags & 2 != 0 ? s.cpu_temperature_c : nil
        cpuFrequencyMHz = s.cpu_sensor_flags & 4 != 0 ? s.cpu_frequency_mhz : nil
    }
}
func processes() -> [ProcessRow] {
    nativeProcesses(limit: 2).map { ProcessRow(name: cString($0.name), cpu: $0.cpu, memory: $0.memory, pid: $0.pid) }
}
final class Model {
    let original = OriginalRenderer()
    var reading: Reading?
    var disks: [Volume] = [] { didSet { original.setVolumes(disks) } }
    func update(_ raw: Reading, at time: Double) { reading = raw }
    func clearHistory() {}
}

enum Palette {
    static let background = NSColor(hex: 0x080b0b)
    static let text = NSColor(hex: 0xcccccc), title = NSColor(hex: 0xeeeeee)
    static let muted = NSColor(hex: 0x7b8582), grid = NSColor(hex: 0x28322e), border = NSColor(hex: 0x435b4b)
    static let green = NSColor(hex: 0x77ca9b), cyan = NSColor(hex: 0x74e6fc)
    static let yellow = NSColor(hex: 0xffd77a), purple = NSColor(hex: 0xb0a9de)
    static let pink = NSColor(hex: 0xdcafde), red = NSColor(hex: 0xff4769)
}
extension NSColor {
    convenience init(hex: UInt32) { self.init(srgbRed: CGFloat((hex >> 16) & 255) / 255, green: CGFloat((hex >> 8) & 255) / 255, blue: CGFloat(hex & 255) / 255, alpha: 1) }
}
final class MonitorView: NSView {
    let model: Model
    var onMenu: ((NSEvent) -> Void)?
    var locked = false, showExtensions = false
    var drawCount = 0
    private var labelImage: NSImage?
    private var labelNames: [String] = []
    private var labelSize = NSSize.zero
    private var labelBacking: CGFloat = 0
    override var isFlipped: Bool { true }
    override var mouseDownCanMoveWindow: Bool { !locked }
    init(model: Model) {
        self.model = model
        super.init(frame: NSRect(origin: .zero, size: canvasSize))
        setAccessibilityElement(true); setAccessibilityRole(.group); setAccessibilityLabel("msutop Mac 实时系统监控")
    }
    required init?(coder: NSCoder) { fatalError() }
    override func rightMouseDown(with event: NSEvent) { onMenu?(event) }
    override func mouseDown(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        if p.x > bounds.width - 34 && p.y < 35 { onMenu?(event) }
        else if !locked { window?.performDrag(with: event) }
    }
    func paint(_ image: CGImage, in rect: NSRect, context: CGContext) {
        context.saveGState(); context.interpolationQuality = .none
        context.translateBy(x: rect.minX, y: rect.maxY); context.scaleBy(x: 1, y: -1)
        context.draw(image, in: NSRect(origin: .zero, size: rect.size)); context.restoreGState()
    }
    func volumeLabelOverlay() -> NSImage {
        let names = model.disks.prefix(4).map(\.name)
        let backing = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
        if let labelImage, names == labelNames, labelSize == bounds.size, labelBacking == backing { return labelImage }
        let image = NSImage(size: bounds.size)
        image.lockFocusFlipped(true)
        let context = NSGraphicsContext.current!.cgContext
        context.clear(NSRect(origin: .zero, size: bounds.size))
        let unit = bounds.height / 80
        for (index, name) in names.enumerated() {
            let area = NSRect(x: 163 * unit, y: CGFloat(15 + index * 16) * unit, width: 90 * unit, height: 8 * unit)
            context.saveGState(); context.clip(to: area)
            context.setShouldAntialias(false); context.setFillColor(Palette.text.cgColor)
            // Smaller than the telemetry pixel grid, with dots snapped to physical pixels.
            let dot = max(1, (unit * (2.0 / 3.0) * backing).rounded()) / backing
            let origin = NSPoint(x: (area.minX * backing).rounded() / backing,
                                 y: ((area.midY - 4 * dot) * backing).rounded() / backing)
            let limit = max(1, Int(area.width / (6 * dot)))
            var scalars = Array(name.precomposedStringWithCanonicalMapping.unicodeScalars)
            if scalars.count > limit { scalars = Array(scalars.prefix(limit - 1)) + [UnicodeScalar(126)!] }
            for (index, scalar) in scalars.enumerated() {
                let glyph = VolumeDotFont.glyph(scalar)
                for row in 0..<8 {
                    let bits = UInt8(truncatingIfNeeded: glyph >> (row * 8))
                    for col in 0..<6 where bits & (0x80 >> col) != 0 {
                        context.fill(NSRect(x: origin.x + CGFloat(index * 6 + col) * dot,
                                            y: origin.y + CGFloat(row) * dot, width: dot, height: dot))
                    }
                }
            }
            context.restoreGState()
        }
        image.unlockFocus()
        labelImage = image; labelNames = names; labelSize = bounds.size; labelBacking = backing
        return image
    }
    override func draw(_ dirtyRect: NSRect) {
        drawCount += 1
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        let time = animationTime()
        if !showExtensions {
            if let frame = model.original.frame(at: time) { paint(frame, in: bounds, context: context) }
        } else if let (left, right) = model.original.panels(at: time) {
            let unit = bounds.height / 80
            paint(left, in: NSRect(x: 0, y: 0, width: 160 * unit, height: bounds.height), context: context)
            paint(right, in: NSRect(x: 160 * unit, y: 0, width: 96 * unit, height: bounds.height), context: context)
            volumeLabelOverlay().draw(in: bounds, from: .zero, operation: .sourceOver, fraction: 1,
                                      respectFlipped: true, hints: [.interpolation: NSImageInterpolation.none])
        }
    }
    func savePNG(to path: String) throws {
        guard let bitmap = bitmapImageRepForCachingDisplay(in: bounds),
              let png = { () -> Data? in cacheDisplay(in: bounds, to: bitmap); return bitmap.representation(using: .png, properties: [:]) }()
        else { throw NSError(domain: appName, code: 1) }
        try png.write(to: URL(fileURLWithPath: path))
    }
}

final class MonitorWindow: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    let model = Model()
    var panel: MonitorWindow!, view: MonitorView!, status: NSStatusItem!, timer: Timer?
    let queue = DispatchQueue(label: "local.msutop.mac.metrics", qos: .utility)
    let displayClock = DisplayClock()
    var busy = false, samples = 0
    var latestProcesses: [MonitorProcess] = []
    var lastVolumes = -Double.infinity, lastProcesses = -Double.infinity, lastFrame = -Double.infinity
    var statsTime = animationTime(), statsSamples = 0, statsDraws = 0
    var animationFPS = preferences.integer(forKey: "animationFPS") == 30 ? 30 : 60
    // Extensions are opt-in, including upgrades from the former extended default.
    var showExtensions = preferences.bool(forKey: "showExtensions")
    var desktop = preferences.object(forKey: "desktopMode") == nil ? false : preferences.bool(forKey: "desktopMode")
    var locked = preferences.bool(forKey: "locked")
    var scale = preferences.double(forKey: "scale") == 0 ? 2 : preferences.double(forKey: "scale")
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        panel = MonitorWindow(contentRect: NSRect(x: 0, y: 0, width: (showExtensions ? 512 : 320) * scale, height: 160 * scale), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.title = appName; panel.backgroundColor = Palette.background; panel.isOpaque = true
        panel.hasShadow = true; panel.isMovableByWindowBackground = true
        panel.hidesOnDeactivate = false; panel.isFloatingPanel = true
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        panel.delegate = self; panel.isReleasedWhenClosed = false
        view = MonitorView(model: model); view.frame = NSRect(origin: .zero, size: panel.frame.size); view.autoresizingMask = [.width, .height]
        view.locked = locked; view.showExtensions = showExtensions
        model.original.vivid = preferences.bool(forKey: "vivid")
        view.onMenu = { [weak self] event in guard let self else { return }; NSMenu.popUpContextMenu(self.menu(), with: event, for: self.view) }
        panel.contentView = view
        updateLevel()
        restorePosition()
        panel.orderFrontRegardless()
        status = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        status.button?.image = NSImage(systemSymbolName: "chart.bar.xaxis", accessibilityDescription: appName)
        status.button?.toolTip = "msutop Mac · 系统监控"
        status.button?.target = self; status.button?.action = #selector(statusClicked)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(woke), name: NSWorkspace.didWakeNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(screensChanged), name: NSApplication.didChangeScreenParametersNotification, object: nil)
        startTimer(); sample()
        displayClock.frame = { [weak self] in self?.presentFrame() }
        displayClock.start()
    }
    func updateLevel() { panel.level = desktop ? NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopIconWindow)) + 1) : .floating }
    func preferredScreen() -> NSScreen { NSScreen.main ?? NSScreen.screens[0] }
    func fit(_ frame: NSRect, on screen: NSScreen) -> NSRect {
        let area = screen.visibleFrame; var f = frame
        f.size.width = min(f.width, area.width - 24); f.size.height = f.width / (showExtensions ? 3.2 : 2)
        f.origin.x = min(max(f.minX, area.minX + 12), area.maxX - f.width - 12)
        f.origin.y = min(max(f.minY, area.minY + 12), area.maxY - f.height - 12)
        return f
    }
    func restorePosition() {
        var f = panel.frame
        if let saved = preferences.string(forKey: "frame") {
            let old = NSRectFromString(saved)
            if let screen = NSScreen.screens.first(where: { $0.visibleFrame.intersects(old) }) { f.origin = old.origin; panel.setFrame(fit(f, on: screen), display: true); return }
        }
        let screen = preferredScreen(), area = screen.visibleFrame
        f.origin = NSPoint(x: area.midX - f.width / 2, y: area.maxY - f.height - 16)
        panel.setFrame(fit(f, on: screen), display: true)
    }
    func windowDidMove(_ notification: Notification) { if let panel { preferences.set(NSStringFromRect(panel.frame), forKey: "frame") } }
    @objc func screensChanged() { let screen = panel.screen ?? preferredScreen(); panel.setFrame(fit(panel.frame, on: screen), display: true) }
    func startTimer() {
        timer?.invalidate()
        timer = Timer(timeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in self?.sample() }
        timer?.tolerance = 0.001
        RunLoop.main.add(timer!, forMode: .common)
    }
    func presentFrame() {
        let now = animationTime()
        guard now - lastFrame >= 0.85 / Double(animationFPS) else { return }
        lastFrame = now
        if panel.isVisible && panel.occlusionState.contains(.visible) {
            view.needsDisplay = true; view.displayIfNeeded()
        }
        if now - statsTime >= 2 {
            let elapsed = now - statsTime
            let data: [String: Any] = ["version": "0.8.0", "telemetry_hz": Double(samples - statsSamples) / elapsed,
                "presentation_hz": Double(view.drawCount - statsDraws) / elapsed, "target_fps": animationFPS,
                "demo": model.original.demo, "original_renderer": true, "show_extensions": showExtensions]
            if let encoded = try? JSONSerialization.data(withJSONObject: data, options: [.sortedKeys]),
               let directory = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?.appendingPathComponent("msutop-mac") {
                try? encoded.write(to: directory.appendingPathComponent("runtime.json"), options: .atomic)
            }
            statsTime = now; statsSamples = samples; statsDraws = view.drawCount
        }
    }
    func sample() {
        guard !busy else { return }; busy = true
        let time = animationTime()
        let refreshVolumes = time - lastVolumes >= 15, refreshProcesses = time - lastProcesses >= 0.5
        if refreshVolumes { lastVolumes = time }
        if refreshProcesses { lastProcesses = time }
        samples += 1
        queue.async { [weak self] in
            let native = nativeSample()
            let reading = Reading(sample: native), diskList = refreshVolumes ? volumes() : nil
            let top = refreshProcesses ? nativeProcesses() : nil
            DispatchQueue.main.async {
                guard let self else { return }
                if let diskList { self.model.disks = diskList }
                if let top { self.latestProcesses = top }
                self.model.update(reading, at: time)
                self.model.original.observe(native, at: time, disks: self.model.disks, processes: self.latestProcesses)
                if self.samples % 15 == 0 {
                    let power = reading.cpuPowerWatts.map { String(format: "%.1f W", $0) } ?? "--"
                    let temperature = reading.cpuTemperatureC.map { String(format: "%.1f °C", $0) } ?? "--"
                    let frequency = reading.cpuFrequencyMHz.map { String(format: "%.2f GHz", $0 / 1000) } ?? "--"
                    self.view.setAccessibilityValue("\(self.model.original.demo ? "原版模拟数据演示" : "实时监控"), 原版动画 \(self.animationFPS) FPS, CPU \(percent(reading.cpu)), 功率 \(power), 温度 \(temperature), 主频 \(frequency), GPU \(reading.gpuAvailable ? percent(reading.gpu) : "不可用"), 内存 \(capacity(reading.memoryUsed)) GiB")
                }
                self.busy = false
            }
        }
    }
    @objc func woke() { model.clearHistory(); sample() }
    func item(_ title: String, _ action: Selector, checked: Bool = false, tag: Int = 0) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: action, keyEquivalent: ""); i.target = self; i.state = checked ? .on : .off; i.tag = tag; return i
    }
    func menu() -> NSMenu {
        let menu = NSMenu(); let title = NSMenuItem(title: "msutop Mac · 实时系统监控", action: nil, keyEquivalent: ""); title.isEnabled = false; menu.addItem(title)
        menu.addItem(.separator())
        menu.addItem(item(panel.isVisible ? "隐藏监控条" : "显示监控条", #selector(toggleVisible)))
        menu.addItem(item("始终置顶", #selector(setFloating), checked: !desktop))
        menu.addItem(item("贴在桌面", #selector(setDesktop), checked: desktop))
        menu.addItem(item("锁定位置", #selector(toggleLock), checked: locked))
        menu.addItem(item("显示 GPU 与多硬盘扩展", #selector(toggleExtensions), checked: showExtensions))
        menu.addItem(item("鲜亮配色", #selector(toggleVivid), checked: model.original.vivid))
        let animation = NSMenuItem(title: "动画", action: nil, keyEquivalent: ""), frames = NSMenu()
        frames.addItem(item("流畅 · 60 FPS", #selector(changeAnimationFPS(_:)), checked: animationFPS == 60, tag: 60))
        frames.addItem(item("省电 · 30 FPS", #selector(changeAnimationFPS(_:)), checked: animationFPS == 30, tag: 30))
        animation.submenu = frames; menu.addItem(animation)
        menu.addItem(item("原版演示（模拟数据）", #selector(toggleDemo), checked: model.original.demo))
        let size = NSMenuItem(title: "大小", action: nil, keyEquivalent: ""), sizes = NSMenu()
        for (label, value) in [("小", 15), ("标准", 20), ("大", 25)] {
            let width = (showExtensions ? 512 : 320) * value / 10, height = 160 * value / 10
            sizes.addItem(item("\(label) · \(width) × \(height)", #selector(changeSize(_:)), checked: abs(scale - Double(value) / 10) < 0.01, tag: value))
        }
        size.submenu = sizes; menu.addItem(size)
        let displays = NSMenuItem(title: "移动到显示器", action: nil, keyEquivalent: ""), screens = NSMenu()
        for (i, screen) in NSScreen.screens.enumerated() { screens.addItem(item(screen.localizedName, #selector(moveToScreen(_:)), tag: i)) }
        displays.submenu = screens; menu.addItem(displays)
        menu.addItem(item("重置到屏幕顶部", #selector(resetPosition)))
        menu.addItem(.separator())
        menu.addItem(item("打开活动监视器", #selector(openActivityMonitor)))
        menu.addItem(item("关于 msutop Mac", #selector(about)))
        menu.addItem(item("退出", #selector(quit)))
        return menu
    }
    @objc func statusClicked() { status.menu = menu(); status.button?.performClick(nil); status.menu = nil }
    @objc func toggleVisible() { if panel.isVisible { panel.orderOut(nil) } else { panel.orderFrontRegardless() } }
    @objc func setFloating() { desktop = false; preferences.set(false, forKey: "desktopMode"); updateLevel(); panel.orderFrontRegardless() }
    @objc func setDesktop() { desktop = true; preferences.set(true, forKey: "desktopMode"); updateLevel(); panel.orderFrontRegardless() }
    @objc func toggleLock() { locked.toggle(); preferences.set(locked, forKey: "locked"); view.locked = locked; panel.isMovableByWindowBackground = !locked; view.needsDisplay = true }
    @objc func toggleExtensions() {
        let left = panel.frame.minX, top = panel.frame.maxY
        showExtensions.toggle(); view.showExtensions = showExtensions
        preferences.set(showExtensions, forKey: "showExtensions")
        var f = panel.frame; f.size.width = (showExtensions ? 512 : 320) * scale; f.size.height = 160 * scale
        f.origin = NSPoint(x: left, y: top - f.height)
        panel.setFrame(fit(f, on: panel.screen ?? preferredScreen()), display: true)
        preferences.set(NSStringFromRect(panel.frame), forKey: "frame")
        view.needsDisplay = true
    }
    @objc func toggleVivid() { model.original.vivid.toggle(); preferences.set(model.original.vivid, forKey: "vivid") }
    @objc func toggleDemo() { model.original.demo.toggle() }
    @objc func changeAnimationFPS(_ sender: NSMenuItem) { animationFPS = sender.tag; preferences.set(animationFPS, forKey: "animationFPS") }
    @objc func changeSize(_ sender: NSMenuItem) {
        scale = Double(sender.tag) / 10; preferences.set(scale, forKey: "scale")
        var f = panel.frame; let top = f.maxY; f.size = NSSize(width: (showExtensions ? 512 : 320) * scale, height: 160 * scale); f.origin.y = top - f.height
        panel.setFrame(fit(f, on: panel.screen ?? preferredScreen()), display: true); preferences.set(NSStringFromRect(panel.frame), forKey: "frame")
    }
    @objc func moveToScreen(_ sender: NSMenuItem) {
        guard NSScreen.screens.indices.contains(sender.tag) else { return }
        let screen = NSScreen.screens[sender.tag], area = screen.visibleFrame
        var f = panel.frame; f.origin = NSPoint(x: area.midX - f.width / 2, y: area.maxY - f.height - 16)
        panel.setFrame(fit(f, on: screen), display: true); panel.orderFrontRegardless()
    }
    @objc func resetPosition() {
        let area = (panel.screen ?? preferredScreen()).visibleFrame
        var f = panel.frame; f.origin = NSPoint(x: area.midX - f.width / 2, y: area.maxY - f.height - 16)
        panel.setFrame(fit(f, on: panel.screen ?? preferredScreen()), display: true); panel.orderFrontRegardless()
    }
    @objc func openActivityMonitor() { NSWorkspace.shared.open(URL(fileURLWithPath: "/System/Applications/Utilities/Activity Monitor.app")) }
    @objc func about() {
        let alert = NSAlert(); alert.messageText = appName
        alert.informativeText = "原版 msutop 动画 + macOS 实时监控\n\n默认显示原版 160×80 完整面板，保留原版字体、布局与动画。菜单中勾选「显示 GPU 与多硬盘扩展」后，在右侧增加 96×80 面板，显示 GPU 曲线与四块磁盘；取消勾选即可收起。选择会保存。扩展与主面板保持相同像素倍率，开关不会改变主面板的字体或布局。原版进程字体为 6×8，其余紧凑数值为 3×5。CPU 右上角依次为利用率/功率、温度/主频。卷标在画布外叠加更小的 6×8 点阵字形，英文为 Adafruit，日文为 k6x8 Gothic。点阵对齐屏幕物理像素，不进行字体平滑。亚像素滚动、渐变、双环、活动灯和平滑算法直接复用原版代码。30 Hz 采集，60 FPS 显示。卷标文字层会缓存，不随动画重复栅格化。\n\n磁盘容量显示四个本地卷；读写曲线合计所有物理磁盘。CPU 功率、温度和主频由只读 macmon 采集器提供，无需管理员权限。功率为 IOReport 估算 CPU 功耗；温度为有效 CPU 传感器平均值；主频为最高性能核心组的平均活动频率，空闲时采用最低档位。采集不可用时显示 --。\n演示模式使用原版模拟数据，左上角有 DEMO 标记；默认始终使用本机真实数据。\n\n授权：msutop MIT / Adafruit BSD / btop Apache-2.0 / k6x8 Num Kadoma 自由许可。"
        alert.runModal()
    }
    @objc func quit() { NSApp.terminate(nil) }
    func applicationWillTerminate(_ notification: Notification) { SensorFeed.shared.stop() }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
}

func iconPNG(_ path: String) throws {
    let image = NSImage(size: NSSize(width: 512, height: 512)); image.lockFocus()
    Palette.background.setFill(); NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: 512, height: 512), xRadius: 100, yRadius: 100).fill()
    for (i, height) in [120, 220, 160, 300, 240].enumerated() { Palette.green.setFill(); NSRect(x: 76 + i * 76, y: 98, width: 44, height: height).fill() }
    Palette.cyan.setFill(); NSRect(x: 76, y: 70, width: 348, height: 8).fill(); image.unlockFocus()
    let rep = NSBitmapImageRep(data: image.tiffRepresentation!)!
    try rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: path))
}
let args = CommandLine.arguments
if let index = args.firstIndex(of: "--probe") {
    defer { SensorFeed.shared.stop() }
    let count = args.indices.contains(index + 1) ? Int(args[index + 1]) ?? 3 : 3
    let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
    for i in 0..<max(1, count) {
        let reading = Reading(); let procs = processes()
        struct Probe: Encodable { let reading: Reading; let volumes: [Volume]; let processes: [ProcessRow] }
        let data = try encoder.encode(Probe(reading: reading, volumes: volumes(), processes: procs))
        print(String(decoding: data, as: UTF8.self)); fflush(stdout)
        if i < count - 1 { Thread.sleep(forTimeInterval: 1) }
    }
} else if let index = args.firstIndex(of: "--render"), args.indices.contains(index + 1) {
    defer { SensorFeed.shared.stop() }
    _ = NSApplication.shared
    let model = Model(); model.disks = volumes()
    for _ in 0..<120 {
        let native = nativeSample()
        let time = animationTime()
        model.update(Reading(sample: native), at: time)
        model.original.observe(native, at: time, disks: model.disks, processes: nativeProcesses())
        Thread.sleep(forTimeInterval: 1.0 / 30.0)
    }
    let view = MonitorView(model: model); view.showExtensions = args.contains("--extensions")
    view.frame.size = NSSize(width: view.showExtensions ? 1024 : 640, height: 320)
    try view.savePNG(to: args[index + 1])
} else if let index = args.firstIndex(of: "--icon"), args.indices.contains(index + 1) {
    _ = NSApplication.shared; try iconPNG(args[index + 1])
} else {
    let app = NSApplication.shared
    if let id = Bundle.main.bundleIdentifier, NSRunningApplication.runningApplications(withBundleIdentifier: id).contains(where: { $0.processIdentifier != getpid() }) { exit(0) }
    let delegate = AppDelegate(); app.delegate = delegate; app.run()
}
