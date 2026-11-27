// Usage: winid <pid> -> "id x y w h layer name" per on-screen window of pid.
// Read-only window listing for window-only screenshots (`screencapture -l`).
import CoreGraphics
import Foundation
let pid = Int32(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
for w in list where (w[kCGWindowOwnerPID as String] as? Int32) == pid {
  let b = w[kCGWindowBounds as String] as! [String: Any]
  print(w[kCGWindowNumber as String]!, b["X"]!, b["Y"]!, b["Width"]!, b["Height"]!,
        w[kCGWindowLayer as String]!, (w[kCGWindowName as String] as? String) ?? "")
}
