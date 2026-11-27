// Explicit display pin for disposable gate batches; always restore original id.
import AppKit
import CoreGraphics
import Foundation
let args = CommandLine.arguments
let id = args[1] == "restore" ? UInt32(args[2])! : CGMainDisplayID()
guard let current = CGDisplayCopyDisplayMode(id),
      let modes = CGDisplayCopyAllDisplayModes(id, [kCGDisplayShowDuplicateLowResolutionModes:true] as CFDictionary) as? [CGDisplayMode] else { exit(2) }
let target: CGDisplayMode?
if args[1] == "restore" {
    target = modes.first { $0.ioDisplayModeID == UInt32(args[3])! }
} else {
    let hz = Double(args[1])!
    target = modes.first { $0.width == current.width && $0.height == current.height && $0.pixelWidth == current.pixelWidth && $0.pixelHeight == current.pixelHeight && abs($0.refreshRate - hz) < 0.5 }
}
guard let target, CGDisplaySetDisplayMode(id, target, nil) == .success,
      CGDisplayCopyDisplayMode(id)?.ioDisplayModeID == target.ioDisplayModeID else { exit(3) }
let report: [String: Any] = ["display":id, "original_mode":current.ioDisplayModeID, "hz":target.refreshRate, "width":target.pixelWidth, "height":target.pixelHeight]
print(String(data: try JSONSerialization.data(withJSONObject:report), encoding:.utf8)!)
