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
let documentIconset = temp.appendingPathComponent("PowermoveDocument.iconset")
try fm.createDirectory(at: documentIconset, withIntermediateDirectories: true)
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

        let document = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: document)
        NSGraphicsContext.current?.imageInterpolation = .high
        let canvas = Double(pixels)
        let page = NSRect(x: canvas * 0.14, y: canvas * 0.05,
                          width: canvas * 0.72, height: canvas * 0.90)
        let radius = canvas * 0.035
        let fold = canvas * 0.15
        let pagePath = NSBezierPath()
        pagePath.move(to: NSPoint(x: page.minX + radius, y: page.minY))
        pagePath.line(to: NSPoint(x: page.maxX - radius, y: page.minY))
        pagePath.curve(to: NSPoint(x: page.maxX, y: page.minY + radius),
                       controlPoint1: NSPoint(x: page.maxX - radius * 0.45, y: page.minY),
                       controlPoint2: NSPoint(x: page.maxX, y: page.minY + radius * 0.45))
        pagePath.line(to: NSPoint(x: page.maxX, y: page.maxY - fold))
        pagePath.line(to: NSPoint(x: page.maxX - fold, y: page.maxY))
        pagePath.line(to: NSPoint(x: page.minX + radius, y: page.maxY))
        pagePath.curve(to: NSPoint(x: page.minX, y: page.maxY - radius),
                       controlPoint1: NSPoint(x: page.minX + radius * 0.45, y: page.maxY),
                       controlPoint2: NSPoint(x: page.minX, y: page.maxY - radius * 0.45))
        pagePath.line(to: NSPoint(x: page.minX, y: page.minY + radius))
        pagePath.curve(to: NSPoint(x: page.minX + radius, y: page.minY),
                       controlPoint1: NSPoint(x: page.minX, y: page.minY + radius * 0.45),
                       controlPoint2: NSPoint(x: page.minX + radius * 0.45, y: page.minY))
        pagePath.close()
        NSColor(calibratedWhite: 0.985, alpha: 1).setFill()
        pagePath.fill()
        NSColor(calibratedWhite: 0.72, alpha: 1).setStroke()
        pagePath.lineWidth = max(1, canvas * 0.012)
        pagePath.stroke()

        let foldPath = NSBezierPath()
        foldPath.move(to: NSPoint(x: page.maxX - fold, y: page.maxY))
        foldPath.line(to: NSPoint(x: page.maxX - fold, y: page.maxY - fold))
        foldPath.line(to: NSPoint(x: page.maxX, y: page.maxY - fold))
        foldPath.close()
        NSColor(calibratedWhite: 0.88, alpha: 1).setFill()
        foldPath.fill()
        NSColor(calibratedWhite: 0.72, alpha: 1).setStroke()
        foldPath.lineWidth = max(1, canvas * 0.009)
        foldPath.stroke()

        let markSide = canvas * 0.45
        image.draw(in: NSRect(x: (canvas - markSide) / 2, y: canvas * 0.17,
                              width: markSide, height: markSide),
                   from: .zero, operation: .sourceOver, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()
        try document.representation(using: .png, properties: [:])!.write(
            to: documentIconset.appendingPathComponent("icon_\(points)x\(points)\(suffix).png"))
    }
}
let output = temp.appendingPathComponent("icon.icns")
try run("/usr/bin/iconutil", ["-c", "icns", iconset.path, "-o", output.path])
try Data(contentsOf: output).write(to: root.appendingPathComponent("resources/icon.icns"), options: .atomic)
let documentOutput = temp.appendingPathComponent("document.icns")
try run("/usr/bin/iconutil", ["-c", "icns", documentIconset.path, "-o", documentOutput.path])
try Data(contentsOf: documentOutput).write(to: root.appendingPathComponent("resources/document.icns"), options: .atomic)
print("Updated resources/icon.icns and resources/document.icns (10 sizes each, 16–1024 pixels).")
