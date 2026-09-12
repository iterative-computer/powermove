// Regenerate the packaged macOS icon: swift scripts/export-app-icon.swift
import AppKit
import Foundation

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
let fm = FileManager.default
let candidates = [
    ProcessInfo.processInfo.environment["ICTOOL_PATH"],
    NSHomeDirectory() + "/Applications/Icon Composer.app/Contents/Executables/ictool",
    "/Applications/Icon Composer.app/Contents/Executables/ictool",
    "/Applications/Xcode.app/Contents/Applications/Icon Composer.app/Contents/Executables/ictool"
].compactMap { $0 }
guard let ictool = candidates.first(where: { fm.isExecutableFile(atPath: $0) }) else {
    fatalError("Install Apple Icon Composer or set ICTOOL_PATH to its ictool executable.")
}
func run(_ executable: String, _ arguments: [String]) throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: executable)
    process.arguments = arguments
    try process.run()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
        throw NSError(domain: "IconExport", code: Int(process.terminationStatus),
                      userInfo: [NSLocalizedDescriptionKey: "Failed: \(executable)"])
    }
}
let temp = fm.temporaryDirectory.appendingPathComponent("powermove-icon-\(UUID().uuidString)")
try fm.createDirectory(at: temp, withIntermediateDirectories: true)
defer { try? fm.removeItem(at: temp) }
let rendered = temp.appendingPathComponent("render.png")
try run(ictool, [root.appendingPathComponent("resources/icon/Powermove.icon").path,
    "--export-image", "--output-file", rendered.path, "--platform", "macOS",
    "--rendition", "Default", "--width", "1024", "--height", "1024", "--scale", "1"])
guard let image = NSImage(contentsOf: rendered) else { fatalError("Icon Composer produced no image") }
let iconset = temp.appendingPathComponent("Powermove.iconset")
try fm.createDirectory(at: iconset, withIntermediateDirectories: true)
for points in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let pixels = points * scale
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
        NSGraphicsContext.current?.imageInterpolation = .high
        // Composer exports the tile edge-to-edge. Legacy macOS ICNS needs
        // transparent margins so the tile matches neighboring Dock icons.
        let side = Double(pixels) * 824.0 / 1024.0
        let inset = (Double(pixels) - side) / 2.0
        image.draw(in: NSRect(x: inset, y: inset, width: side, height: side),
                   from: .zero, operation: .copy, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()
        let suffix = scale == 2 ? "@2x" : ""
        try bitmap.representation(using: .png, properties: [:])!.write(
            to: iconset.appendingPathComponent("icon_\(points)x\(points)\(suffix).png"))
    }
}
let output = temp.appendingPathComponent("icon.icns")
try run("/usr/bin/iconutil", ["-c", "icns", iconset.path, "-o", output.path])
try Data(contentsOf: output).write(to: root.appendingPathComponent("resources/icon.icns"), options: .atomic)
print("Updated resources/icon.icns from resources/icon/Powermove.icon (10 sizes, 16–1024 pixels).")
