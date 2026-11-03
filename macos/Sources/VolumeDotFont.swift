import Foundation

/// Hand-designed 6x8 bitmap glyphs; never rasterize an outline font into a tiny cell.
enum VolumeDotFont {
    static let width = 6
    static let height = 8
    private static let japanese: [UInt32: UInt64] = {
        let executable = URL(fileURLWithPath: CommandLine.arguments[0]).deletingLastPathComponent().deletingLastPathComponent()
        let candidates = [Bundle.main.resourceURL?.appendingPathComponent("fonts/k6x8-gothic.bin"),
                          executable.appendingPathComponent("assets/fonts/k6x8/k6x8-gothic.bin")].compactMap { $0 }
        guard let data = candidates.compactMap({ try? Data(contentsOf: $0) }).first else { fatalError("Missing bundled k6x8 bitmap font") }
        let bytes = [UInt8](data)
        precondition(bytes.count >= 8 && Array(bytes.prefix(4)) == Array("KF68".utf8))
        func integer(_ offset: Int) -> UInt32 {
            UInt32(bytes[offset]) | UInt32(bytes[offset + 1]) << 8 | UInt32(bytes[offset + 2]) << 16 | UInt32(bytes[offset + 3]) << 24
        }
        let count = Int(integer(4))
        precondition(bytes.count == 8 + count * 12)
        var glyphs: [UInt32: UInt64] = [:]; glyphs.reserveCapacity(count)
        for i in 0..<count {
            let offset = 8 + i * 12
            var rows: UInt64 = 0
            for row in 0..<8 { rows |= UInt64(bytes[offset + 4 + row]) << (row * 8) }
            glyphs[integer(offset)] = rows
        }
        return glyphs
    }()
    static func glyph(_ scalar: UnicodeScalar) -> UInt64 {
        if (32...126).contains(scalar.value) {
            let offset = Int(scalar.value - 32) * 8
            var rows: UInt64 = 0
            for row in 0..<8 { rows |= UInt64(pixelGlyphs[offset + row]) << (row * 8) }
            return rows
        }
        return japanese[scalar.value] ?? japanese[0x25a1] ?? glyph("?")
    }
}
