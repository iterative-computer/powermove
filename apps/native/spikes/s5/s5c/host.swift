// On-screen proof host, not a production foreign-view slot or browser automation.
import AppKit
import Darwin
import WebKit

func emit(_ x: [String: Any]) {
  let d = try! JSONSerialization.data(withJSONObject: x, options: [.sortedKeys])
  print(String(data: d, encoding: .utf8)!)
  fflush(stdout)
}
func footprint(_ pid: pid_t) -> UInt64 {
  var info = rusage_info_v4()
  let rc = withUnsafeMutablePointer(to: &info) { p in
    p.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) {
      proc_pid_rusage(pid, RUSAGE_INFO_V4, $0)
    }
  }
  return rc == 0 ? info.ri_phys_footprint : 0
}
final class Host: NSObject, WKScriptMessageHandler {
  let mode: String
  let app = NSApplication.shared
  var window: NSWindow!
  var web: WKWebView!
  var count = 0
  var keyStart = 0.0
  var timer: Timer!
  var jobs: Process?
  var input: FileHandle?
  var phase = 0
  var scrub: [Double] = []
  init(_ mode: String) {
    self.mode = mode
    super.init()
  }
  func start() {
    app.setActivationPolicy(.regular)
    let cfg = WKWebViewConfiguration()
    cfg.userContentController.add(self, name: "probe")
    cfg.preferences.inactiveSchedulingPolicy = .none
    web = WKWebView(frame: NSRect(x: 0, y: 0, width: 800, height: 500), configuration: cfg)
    window = NSWindow(
      contentRect: NSRect(x: 100, y: 100, width: 800, height: 500),
      styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.title = "Powermove S5c \(mode)"
    window.contentView = web
    app.finishLaunching()
    window.makeKeyAndOrderFront(nil)
    app.activate(ignoringOtherApps: true)
    window.makeFirstResponder(web)
    let root = URL(fileURLWithPath: CommandLine.arguments[2])
    var html = try! String(contentsOf: root.appendingPathComponent("panel.html"), encoding: .utf8)
    let url =
      mode == "B" ? try! String(contentsOfFile: CommandLine.arguments[3], encoding: .utf8) : ""
    func literal(_ s: String) -> String {
      String(
        data: try! JSONSerialization.data(withJSONObject: s, options: .fragmentsAllowed),
        encoding: .utf8)!
    }
    html = html.replacingOccurrences(of: "MODE", with: literal(mode)).replacingOccurrences(
      of: "URL_VALUE", with: literal(url))
    web.loadHTMLString(html, baseURL: nil)
    timer = Timer.scheduledTimer(withTimeInterval: 1.0 / 120, repeats: true) { _ in
      let t = ProcessInfo.processInfo.systemUptime
      var sum = 0.0
      repeat {
        for i in 0..<5000 { sum += sin(Double(i) * 0.17) }
      } while ProcessInfo.processInfo.systemUptime - t < 0.001
      self.window.title = "Powermove S5c \(self.mode) \(Int(sum)%10)"
      self.scrub.append((ProcessInfo.processInfo.systemUptime - t) * 1000)
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + 140) {
      emit(["probe": "host_timeout"])
      exit(2)
    }
  }
  func key() {
    guard app.isActive, window.isKeyWindow else {
      emit(["probe": "foreground_failed"])
      exit(2)
    }
    keyStart = ProcessInfo.processInfo.systemUptime
    for type in [NSEvent.EventType.keyDown, .keyUp] {
      let e = NSEvent.keyEvent(
        with: type, location: .zero, modifierFlags: [],
        timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
        context: nil, characters: "a", charactersIgnoringModifiers: "a", isARepeat: false,
        keyCode: 0)!
      app.sendEvent(e)
    }
  }
  func userContentController(_ c: WKUserContentController, didReceive m: WKScriptMessage) {
    let v = m.body as! [String: Any]
    if v["kind"] as? String == "failure" {
      emit(v)
      exit(2)
    }
    if v["kind"] as? String == "ready" {
      emit([
        "probe": "dom_ready", "mode": mode, "state": v, "host_active": app.isActive,
        "window_key": window.isKeyWindow,
      ])
      if mode == "life" {
        startLife()
      } else {
        if mode == "H" {
          startWorker()
          input!.write("{\"kind\":\"bench\",\"build\":\"hybrid-release\"}\n".data(using: .utf8)!)
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { self.key() }
      }
      return
    }
    if v["kind"] as? String == "services" {
      emit(["probe": "services_page", "mode": mode, "result": v])
      return
    }
    guard v["kind"] as? String == "paint" else { return }
    let ack = (ProcessInfo.processInfo.systemUptime - keyStart) * 1000
    web.takeSnapshot(with: nil) { image, error in
      guard image != nil, error == nil,
        (v["text"] as? String) == "Value: " + String(repeating: "a", count: self.count + 1)
      else {
        emit(["probe": "snapshot_failed"])
        exit(2)
      }
      emit([
        "probe": "dom_sample", "mode": self.mode, "page": v, "native_key_to_ack_ms": ack,
        "native_key_to_snapshot_ms": (ProcessInfo.processInfo.systemUptime - self.keyStart) * 1000,
        "snapshot": true,
      ])
      self.count += 1
      if self.count < 30 {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.03) { self.key() }
      } else {
        self.finish()
      }
    }
  }
  func startWorker() {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: CommandLine.arguments[3])
    let pipe = Pipe()
    process.standardInput = pipe
    process.standardOutput = FileHandle.standardOutput
    process.standardError = FileHandle.standardError
    try! process.run()
    jobs = process
    input = pipe.fileHandleForWriting
  }
  func startLife() {
    startWorker()
    lifePhase("foreground")
    DispatchQueue.main.asyncAfter(deadline: .now() + 7) {
      self.window.miniaturize(nil)
      self.app.hide(nil)
      emit(["probe": "life_dwell", "seconds": 60])
      DispatchQueue.main.asyncAfter(deadline: .now() + 60) {
        emit([
          "probe": "life_hidden", "minimized": self.window.isMiniaturized,
          "hidden": self.app.isHidden,
        ])
        self.lifePhase("hidden")
        DispatchQueue.main.asyncAfter(deadline: .now() + 7) {
          self.window.close()
          emit(["probe": "life_closed", "visible": self.window.isVisible])
          self.lifePhase("closed")
          DispatchQueue.main.asyncAfter(deadline: .now() + 7) { self.finish() }
        }
      }
    }
  }
  func lifePhase(_ phase: String) {
    input!.write(("{\"kind\":\"phase\",\"phase\":\"\(phase)\",\"ms\":5000}\n").data(using: .utf8)!)
  }
  func finish() {
    input?.closeFile()
    input = nil
    if let jobs = jobs, jobs.isRunning {
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { self.finish() }
      return
    }
    timer.invalidate()
    let sel = NSSelectorFromString("_webProcessIdentifier")
    var pid: pid_t = 0
    if web.responds(to: sel) {
      typealias Getter = @convention(c) (AnyObject, Selector) -> pid_t
      pid = unsafeBitCast(web.method(for: sel), to: Getter.self)(web, sel)
    }
    emit([
      "probe": "host_memory", "mode": mode, "host_bytes": footprint(getpid()),
      "web_bytes": footprint(pid), "web_pid": pid, "scrub_samples": scrub.count,
      "scrub_p95_ms": scrub.sorted()[Int(Double(scrub.count - 1) * 0.95)],
    ])
    window.orderOut(nil)
    app.terminate(nil)
  }
}
let h = Host(CommandLine.arguments[1])
h.start()
NSApplication.shared.run()
