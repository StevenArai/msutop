import AppKit
import CoreVideo

let animationEpoch = ProcessInfo.processInfo.systemUptime
func animationTime() -> Double { 70 + ProcessInfo.processInfo.systemUptime - animationEpoch }

func nativeProcesses(limit: Int = 32) -> [MonitorProcess] {
    var values = [MonitorProcess](repeating: MonitorProcess(), count: limit)
    let count = monitor_processes(&values, Int32(limit))
    return Array(values.prefix(Int(count)))
}

final class OriginalRenderer {
    private let handle = msutop_renderer_create()!
    private var rgba = [UInt8](repeating: 0, count: 160 * 80 * 4)
    private var rightRGBA = [UInt8](repeating: 0, count: 96 * 80 * 4)
    private let colorSpace = CGColorSpaceCreateDeviceRGB()
    var vivid = false { didSet { applyOptions() } }
    var antialias = true { didSet { applyOptions() } }
    var demo = false { didSet { applyOptions() } }
    deinit { msutop_renderer_destroy(handle) }
    private func applyOptions() { msutop_renderer_options(handle, vivid ? 1 : 0, antialias ? 1 : 0, demo ? 1 : 0) }
    func setVolumes(_ disks: [Volume]) {
        var values = disks.prefix(4).map { disk -> MonitorVolume in
            var result = MonitorVolume(); result.used = disk.used; result.total = disk.total
            let bytes = Array(disk.name.utf8.prefix(127))
            withUnsafeMutableBytes(of: &result.name) { $0.copyBytes(from: bytes) }
            return result
        }
        msutop_renderer_volumes(handle, &values, Int32(values.count))
    }
    func observe(_ sample: MonitorSample, at time: Double, disks: [Volume], processes: [MonitorProcess]) {
        var sample = sample
        processes.withUnsafeBufferPointer { rows in
            msutop_renderer_observe(handle, &sample, time, disks.first?.used ?? 0, disks.first?.total ?? 0, rows.baseAddress, Int32(rows.count))
        }
    }
    func frame(at time: Double) -> CGImage? {
        let size = rgba.count
        let count = rgba.withUnsafeMutableBufferPointer { msutop_renderer_render(handle, time, $0.baseAddress, Int32(size)) }
        guard count == size else { return nil }
        return image(rgba)
    }
    private func image(_ bytes: [UInt8], width: Int = 160) -> CGImage? {
        guard let provider = CGDataProvider(data: Data(bytes) as CFData) else { return nil }
        return CGImage(width: width, height: 80, bitsPerComponent: 8, bitsPerPixel: 32,
                       bytesPerRow: width * 4, space: colorSpace,
                       bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }
    func panels(at time: Double) -> (CGImage, CGImage)? {
        let size = rgba.count
        let count = rgba.withUnsafeMutableBufferPointer { left in
            rightRGBA.withUnsafeMutableBufferPointer { right in
                msutop_renderer_render_panels(handle, time, left.baseAddress, right.baseAddress, Int32(size))
            }
        }
        guard count == size, let left = image(rgba), let right = image(rightRGBA, width: 96) else { return nil }
        return (left, right)
    }
}

/// Display refresh is independent of telemetry. Never queue a backlog of obsolete frames.
final class DisplayClock {
    private var link: CVDisplayLink?
    private let lock = NSLock()
    private var pending = false
    var frame: (() -> Void)?
    init() {
        var created: CVDisplayLink?
        guard CVDisplayLinkCreateWithActiveCGDisplays(&created) == kCVReturnSuccess, let created else { return }
        link = created
        CVDisplayLinkSetOutputCallback(created, { _, _, _, _, _, context in
            guard let context else { return kCVReturnError }
            Unmanaged<DisplayClock>.fromOpaque(context).takeUnretainedValue().schedule()
            return kCVReturnSuccess
        }, Unmanaged.passUnretained(self).toOpaque())
    }
    private func schedule() {
        lock.lock()
        if pending { lock.unlock(); return }
        pending = true; lock.unlock()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.lock.lock(); self.pending = false; self.lock.unlock()
            self.frame?()
        }
    }
    func start() { if let link { CVDisplayLinkStart(link) } }
    func stop() { if let link { CVDisplayLinkStop(link) } }
    deinit { stop() }
}

struct TimedValue { let seconds: Double; let value: Double }
