// Read-only environment probe. No window, display mode or user setting changes.
import AppKit
import CoreGraphics
import Foundation

let thermal: String
switch ProcessInfo.processInfo.thermalState {
case .nominal: thermal = "nominal"
case .fair: thermal = "fair"
case .serious: thermal = "serious"
case .critical: thermal = "critical"
@unknown default: thermal = "unknown"
}
var ids = [CGDirectDisplayID](repeating: 0, count: 32)
var count: UInt32 = 0
guard CGGetActiveDisplayList(UInt32(ids.count), &ids, &count) == .success,
      Int(count) < ids.count else { exit(2) }
var displays: [[String: Any]] = []
for id in ids.prefix(Int(count)) {
    guard let mode = CGDisplayCopyDisplayMode(id) else { exit(3) }
    displays.append(["id": id, "mode": mode.ioDisplayModeID,
                     "width": mode.pixelWidth, "height": mode.pixelHeight,
                     "hz": mode.refreshRate, "builtin": CGDisplayIsBuiltin(id) != 0])
}
let apps: [[String: Any]] = NSWorkspace.shared.runningApplications.map {
    ["id": $0.bundleIdentifier ?? "unknown", "pid": $0.processIdentifier,
     "executable": $0.executableURL?.path ?? "unknown"]
}
let data: [String: Any] = ["thermal": thermal, "displays": displays, "apps": apps]
print(String(data: try JSONSerialization.data(withJSONObject: data, options: [.sortedKeys]), encoding: .utf8)!)
