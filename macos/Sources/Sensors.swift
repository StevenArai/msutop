import Foundation

struct CPUSensorReading {
    var power: Double?, temperature: Double?, frequency: Double?
    var tier = ""
    var received = ProcessInfo.processInfo.systemUptime
}

/// A separate bundled helper isolates private hardware APIs from the UI process.
final class SensorFeed {
    static let shared = SensorFeed()
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "local.msutop.mac.sensors", qos: .utility)
    private var reading: CPUSensorReading?
    private var process: Process?
    private var stopped = false

    private init() { queue.async { [weak self] in self?.run() } }
    var latest: CPUSensorReading? {
        lock.lock(); defer { lock.unlock() }
        guard let reading, ProcessInfo.processInfo.systemUptime - reading.received < 3 else { return nil }
        return reading
    }
    private var shouldStop: Bool { lock.lock(); defer { lock.unlock() }; return stopped }
    func stop() {
        lock.lock(); stopped = true; let child = process; lock.unlock()
        if let child, child.isRunning { child.terminate() }
    }
    private func consume(_ line: Data) {
        guard let object = try? JSONSerialization.jsonObject(with: line) as? [String: Any], object["type"] as? String == "sample" else { return }
        func number(_ key: String, _ range: ClosedRange<Double>) -> Double? {
            guard let value = object[key] as? Double, value.isFinite, range.contains(value) else { return nil }
            return value
        }
        let sample = CPUSensorReading(power: number("cpu_power_watts", 0...1000),
            temperature: number("cpu_temperature_c", 0.1...150), frequency: number("cpu_frequency_mhz", 100...10_000),
            tier: object["frequency_tier"] as? String ?? "")
        lock.lock(); reading = sample; lock.unlock()
    }
    private func run() {
        let executable = Bundle.main.executableURL ?? URL(fileURLWithPath: CommandLine.arguments[0])
        let helper = executable.deletingLastPathComponent().appendingPathComponent("msutop-sensors")
        guard FileManager.default.isExecutableFile(atPath: helper.path) else { return }
        while !shouldStop {
            let child = Process(), output = Pipe()
            child.executableURL = helper; child.standardOutput = output; child.standardError = FileHandle.nullDevice
            do {
                try child.run()
                lock.lock(); process = child; let cancelled = stopped; lock.unlock()
                if cancelled { child.terminate() }
                var pending = Data()
                while !shouldStop {
                    let chunk = autoreleasepool { output.fileHandleForReading.availableData }
                    if chunk.isEmpty { break }
                    pending.append(chunk)
                    while let newline = pending.firstIndex(of: 10) {
                        autoreleasepool { consume(Data(pending.prefix(upTo: newline))) }
                        pending = Data(pending.suffix(from: pending.index(after: newline)))
                    }
                    if pending.count > 64 * 1024 { pending.removeAll() }
                }
                if child.isRunning { child.terminate() }
                child.waitUntilExit()
                try? output.fileHandleForReading.close()
            } catch { /* Missing/unavailable sensors remain --; retry without disrupting the UI. */ }
            lock.lock(); process = nil; reading = nil; lock.unlock()
            if !shouldStop { Thread.sleep(forTimeInterval: 2) }
        }
    }
}

func nativeSample() -> MonitorSample {
    var sample = MonitorSample(); monitor_sample(&sample)
    if let sensor = SensorFeed.shared.latest {
        if let value = sensor.power { sample.cpu_power_watts = value; sample.cpu_sensor_flags |= 1 }
        if let value = sensor.temperature { sample.cpu_temperature_c = value; sample.cpu_sensor_flags |= 2 }
        if let value = sensor.frequency { sample.cpu_frequency_mhz = value; sample.cpu_sensor_flags |= 4 }
    }
    return sample
}
