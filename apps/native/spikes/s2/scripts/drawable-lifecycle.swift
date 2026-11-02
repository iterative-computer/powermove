// Own-process AppKit stress harness. Run only through check-drawable-lifecycle.py.
import AppKit
import ApplicationServices
import Darwin

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let args = CommandLine.arguments
precondition(args.count == 4, "binary, temporary profile, seconds required")
let binary = args[1], profile = args[2], seconds = Int(args[3])!
let userActive = "/tmp/pm-shots/USER_ACTIVE"
let fm = FileManager.default
func log(_ text: String) { print(text); fflush(stdout) }
func checkUser() -> Bool { !fm.fileExists(atPath: userActive) }
guard checkUser() else { exit(75) }
guard AXIsProcessTrusted() else { log("UNVERIFIED: Accessibility access required for window moves"); exit(77) }
let cover = NSWindow(contentRect: NSScreen.main!.frame,
    styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
cover.title = "Powermove drawable lifecycle regression"
cover.collectionBehavior = [.fullScreenPrimary]
cover.contentView!.wantsLayer = true
cover.contentView!.layer!.backgroundColor = NSColor.black.cgColor
cover.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)
cover.toggleFullScreen(nil) // owns a real full-screen Space, like a full-screen game
let child = Process()
child.executableURL = URL(fileURLWithPath: binary)
child.arguments = ["--profile", profile]
let output = profile + "/app.log"
try fm.createDirectory(atPath: profile, withIntermediateDirectories: true)
fm.createFile(atPath: output, contents: nil)
let handle = try FileHandle(forWritingTo: URL(fileURLWithPath: output))
child.standardOutput = handle; child.standardError = handle
var target: NSRunningApplication?
var originalPosition: CGPoint?
var window: AXUIElement?
var elapsed = 0, activations = 0, hides = 0, offscreen = 0, spaceSwitches = 0
var stopped = false
func finish(_ code: Int32, _ reason: String) {
    guard !stopped else { return }; stopped = true
    log("\(reason) seconds=\(elapsed) activations=\(activations) hides=\(hides) offscreen=\(offscreen) space_switches=\(spaceSwitches)")
    if child.isRunning { child.terminate(); child.waitUntilExit() }
    cover.close()
    exit(code)
}
func activateTarget() {
    guard let target else { return }
    target.unhide(); target.activate(options: [.activateAllWindows, .activateIgnoringOtherApps])
    activations += 1
}
let timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { _ in
    guard checkUser() else { finish(75, "STOP USER_ACTIVE"); return }
    elapsed += 1
    if elapsed == 3 {
        do { try child.run() } catch { finish(1, "launch failed: \(error)"); return }
        target = NSRunningApplication(processIdentifier: child.processIdentifier)
        cover.makeKeyAndOrderFront(nil); app.activate(ignoringOtherApps: true)
        log("launched pid=\(child.processIdentifier) behind full-screen cover")
    }
    guard elapsed > 3 else { return }
    guard child.isRunning else { finish(1, "CRASH/EXIT status=\(child.terminationStatus)"); return }
    if window == nil {
        var value: CFTypeRef?
        let ax = AXUIElementCreateApplication(child.processIdentifier)
        if AXUIElementCopyAttributeValue(ax, kAXWindowsAttribute as CFString, &value) == .success,
           let windows = value as? [AXUIElement], let first = windows.first {
            window = first
            var position: CFTypeRef?
            if AXUIElementCopyAttributeValue(first, kAXPositionAttribute as CFString, &position) == .success,
               let position, CFGetTypeID(position) == AXValueGetTypeID() {
                var p = CGPoint.zero
                AXValueGetValue(unsafeBitCast(position, to: AXValue.self), .cgPoint, &p)
                originalPosition = p
            }
        }
    }
    switch (elapsed - 4) % 20 {
    case 0:
        target?.hide(); hides += 1
        app.activate(ignoringOtherApps: true); spaceSwitches += 1
    case 4:
        target?.unhide() // remain behind the cover in its full-screen Space
        cover.makeKeyAndOrderFront(nil); app.activate(ignoringOtherApps: true)
    case 7:
        activateTarget(); spaceSwitches += 1
    case 10:
        if let window, originalPosition != nil {
            var p = CGPoint(x: -20000, y: -20000)
            let result = AXUIElementSetAttributeValue(window, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &p)!)
            if result == .success { offscreen += 1 }
        }
    case 12:
        if let window, var p = originalPosition {
            _ = AXUIElementSetAttributeValue(window, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &p)!)
        }
        activateTarget()
    case 15:
        target?.hide(); hides += 1; app.activate(ignoringOtherApps: true); spaceSwitches += 1
    case 18:
        activateTarget(); spaceSwitches += 1
    default: break
    }
    if elapsed % 20 == 0 { log("alive elapsed=\(elapsed) window=\(window != nil)") }
    if elapsed >= seconds + 3 {
        let passed = window != nil && offscreen > 0 && activations > 0
        finish(passed ? 0 : 1, passed ? "DRAWABLE_STRESS_PASS" : "UNVERIFIED missing window transitions")
    }
}
RunLoop.main.add(timer, forMode: .common)
app.run()
