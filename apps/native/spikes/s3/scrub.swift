// External, timestamped HID mouse events through the AppKit/GPUI input path.
// Run only while run.sh owns the S3 GUI lock. Coordinates derive from its window.
import AppKit
import CoreGraphics
let lock = try String(contentsOfFile: "/tmp/pm-shots/gui.lock", encoding: .utf8)
precondition(lock.trimmingCharacters(in: .whitespacesAndNewlines) == "s3")
let pid = pid_t(CommandLine.arguments[1])!
let wid = CGWindowID(CommandLine.arguments[2])!
let windows = CGWindowListCopyWindowInfo(.optionIncludingWindow, wid) as! [[String:Any]]
precondition((windows.first![kCGWindowOwnerPID as String] as! Int) == Int(pid))
precondition(CGPreflightPostEventAccess(), "Input posting access is required")
let b = windows.first![kCGWindowBounds as String] as! NSDictionary
let bounds = CGRect(dictionaryRepresentation: b)!
for i in 0..<120 {
    let fraction = Double((i*37)%120)/120
    let point = CGPoint(x: bounds.minX+30+fraction*(bounds.width-60), y: bounds.maxY-58)
    CGEvent(mouseEventSource:nil, mouseType:.leftMouseDown, mouseCursorPosition:point, mouseButton:.left)!.post(tap: .cghidEventTap)
    Thread.sleep(forTimeInterval: 0.001)
    CGEvent(mouseEventSource:nil, mouseType:.leftMouseUp, mouseCursorPosition:point, mouseButton:.left)!.post(tap: .cghidEventTap)
    Thread.sleep(forTimeInterval: 0.035)
}
print("Posted 120 timestamped AppKit scrub clicks")
