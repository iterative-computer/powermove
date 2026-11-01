// Windowed CAMetalLayer presentation-latency probe. No Powermove code, document
// or profile is involved: it isolates the WindowServer/compositor floor for a
// below-slot style layer (layer-hosting root + RGBA16F EDR CAMetalLayer) so app
// latency can be compared against it. Run on screen while holding the GUI lock.
//
//   swiftc -O tools/perf-gate/present_probe.swift -o /tmp/pm-present-probe
//   /tmp/pm-present-probe mode=ondemand samples=40 count=3 sync=1 txn=0 link=0
//   /tmp/pm-present-probe mode=continuous seconds=10 count=3 sync=1 at=1
//
// Keys: mode=ondemand|continuous, count=2|3 (maximumDrawableCount), sync=0|1
// (displaySyncEnabled), txn=0|1 (presentsWithTransaction, present on main in an
// explicit CATransaction), link=0|1 (keep a 120 Hz CADisplayLink active while
// idle), at=0|1 (continuous: presentDrawable:atTime: targetTimestamp),
// edr=0|1, busy=0|1 (a second GPUI-like layer presents with transaction on
// main every display tick). Output: one JSON line, host seconds/milliseconds.
import AppKit
import Metal
import QuartzCore

var opts: [String: String] = ["mode": "ondemand", "samples": "40", "seconds": "10", "count": "3", "sync": "1",
                              "txn": "0", "link": "0", "at": "0", "edr": "1", "busy": "0", "hz": "120"]
for a in CommandLine.arguments.dropFirst() {
    let kv = a.split(separator: "=", maxSplits: 1).map(String.init)
    if kv.count == 2 { opts[kv[0]] = kv[1] }
}
func flag(_ k: String) -> Bool { opts[k] == "1" }
func num(_ k: String) -> Double { Double(opts[k]!)! }

final class Probe: NSObject, NSApplicationDelegate {
    let device = MTLCreateSystemDefaultDevice()!
    lazy var queue = device.makeCommandQueue()!
    var window: NSWindow!
    let layer = CAMetalLayer()
    let busyLayer = CAMetalLayer()
    var link: CADisplayLink?
    let lock = NSLock()
    var rows: [[String: Double]] = []
    var pending = 0
    var color = 0.0
    var started = 0.0
    var interval = 1.0 / 120

    func applicationDidFinishLaunching(_: Notification) {
        let frame = NSRect(x: 200, y: 200, width: 960, height: 600)
        window = NSWindow(contentRect: frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.title = "pm-present-probe"
        let content = NSView(frame: NSRect(origin: .zero, size: frame.size))
        let root = CALayer()
        content.layer = root
        content.wantsLayer = true
        layer.device = device
        layer.pixelFormat = .rgba16Float
        layer.isOpaque = true
        layer.framebufferOnly = false
        layer.maximumDrawableCount = Int(num("count"))
        layer.displaySyncEnabled = flag("sync")
        layer.presentsWithTransaction = flag("txn")
        layer.allowsNextDrawableTimeout = true
        if flag("edr") {
            layer.wantsExtendedDynamicRangeContent = true
            layer.colorspace = CGColorSpace(name: CGColorSpace.extendedLinearDisplayP3)
        }
        layer.frame = CGRect(x: 0, y: 120, width: 960, height: 480)
        layer.contentsScale = window.backingScaleFactor
        layer.drawableSize = CGSize(width: 960 * window.backingScaleFactor, height: 480 * window.backingScaleFactor)
        root.addSublayer(layer)
        if flag("busy") {
            busyLayer.device = device
            busyLayer.pixelFormat = .bgra8Unorm
            busyLayer.presentsWithTransaction = true
            busyLayer.frame = CGRect(x: 0, y: 0, width: 960, height: 120)
            busyLayer.contentsScale = window.backingScaleFactor
            busyLayer.drawableSize = CGSize(width: 960 * window.backingScaleFactor, height: 120 * window.backingScaleFactor)
            root.addSublayer(busyLayer)
        }
        window.contentView = content
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        interval = window.screen?.minimumRefreshInterval ?? interval
        // Settle: let the window reach the screen before timing.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { self.begin() }
    }

    func begin() {
        started = CACurrentMediaTime()
        if flag("link") || opts["mode"] == "continuous" || flag("busy") {
            let l = window.contentView!.displayLink(target: self, selector: #selector(tick(_:)))
            let hz = Float(num("hz"))
            l.preferredFrameRateRange = CAFrameRateRange(minimum: hz, maximum: hz, preferred: hz)
            l.add(to: .main, forMode: .common)
            link = l
        }
        if opts["mode"] == "ondemand" {
            Thread.detachNewThread { self.onDemand() }
        } else {
            DispatchQueue.main.asyncAfter(deadline: .now() + num("seconds") + 0.5) { self.finish() }
        }
    }

    var ticks = 0
    @objc func tick(_ l: CADisplayLink) {
        ticks += 1
        if flag("busy") {
            // GPUI-like transactional present on main every display tick.
            if let d = busyLayer.nextDrawable() {
                let cb = queue.makeCommandBuffer()!
                clear(cb, d.texture, 0.1)
                CATransaction.begin()
                cb.commit()
                cb.waitUntilScheduled()
                d.present()
                CATransaction.commit()
            }
        }
        guard opts["mode"] == "continuous", CACurrentMediaTime() - started < num("seconds") else { return }
        let req = CACurrentMediaTime()
        draw(request: req, target: l.targetTimestamp)
    }

    func onDemand() {
        Thread.current.qualityOfService = .userInteractive
        let samples = Int(num("samples"))
        for _ in 0..<samples {
            // Random phase relative to vsync; idle long enough for VRR to fall back.
            usleep(UInt32.random(in: 140_000...220_000))
            let req = CACurrentMediaTime()
            if flag("txn") {
                DispatchQueue.main.sync { self.draw(request: req, target: 0) }
            } else {
                draw(request: req, target: 0)
            }
        }
        usleep(400_000)
        DispatchQueue.main.async { self.finish() }
    }

    func clear(_ cb: MTLCommandBuffer, _ t: MTLTexture, _ v: Double) {
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = t
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].storeAction = .store
        pass.colorAttachments[0].clearColor = MTLClearColor(red: v, green: 0.3, blue: 1 - v, alpha: 1)
        cb.makeRenderCommandEncoder(descriptor: pass)!.endEncoding()
    }

    func draw(request: Double, target: Double) {
        let asked = CACurrentMediaTime()
        guard let d = layer.nextDrawable() else {
            lock.lock(); rows.append(["request": request, "no_drawable": 1]); lock.unlock()
            return
        }
        let acquired = CACurrentMediaTime()
        color = color > 0.5 ? 0.1 : 0.9
        let cb = queue.makeCommandBuffer()!
        clear(cb, d.texture, color)
        var row: [String: Double] = ["request": request, "asked": asked, "acquired": acquired, "target": target]
        let index: Int = { lock.lock(); defer { lock.unlock() }; rows.append(row); return rows.count - 1 }()
        d.addPresentedHandler { [weak self] dr in
            guard let self else { return }
            self.lock.lock(); self.rows[index]["display"] = dr.presentedTime; self.rows[index]["callback"] = CACurrentMediaTime(); self.lock.unlock()
        }
        cb.addCompletedHandler { [weak self] _ in
            guard let self else { return }
            self.lock.lock(); self.rows[index]["completed"] = CACurrentMediaTime(); self.lock.unlock()
        }
        if flag("txn") {
            CATransaction.begin()
            row["commit"] = CACurrentMediaTime()
            cb.commit()
            cb.waitUntilScheduled()
            d.present()
            CATransaction.commit()
        } else {
            if flag("at") && target > 0 {
                cb.present(d, atTime: target)
            } else {
                cb.present(d)
            }
            row["commit"] = CACurrentMediaTime()
            cb.commit()
        }
        lock.lock(); rows[index]["commit"] = row["commit"]; lock.unlock()
    }

    func finish() {
        link?.invalidate()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
            self.lock.lock()
            let rows = self.rows
            self.lock.unlock()
            func dist(_ v: [Double]) -> [String: Double] {
                let s = v.sorted()
                guard !s.isEmpty else { return ["n": 0] }
                func q(_ p: Double) -> Double { s[max(0, Int((p * Double(s.count)).rounded(.up)) - 1)] }
                return ["n": Double(s.count), "p50": q(0.5), "p95": q(0.95), "max": s.last!]
            }
            let shown = rows.filter { ($0["display"] ?? 0) > 0 }
            let c2d = shown.compactMap { r in r["commit"].map { (r["display"]! - $0) * 1000 } }
            let r2d = shown.map { ($0["display"]! - $0["request"]!) * 1000 }
            let t2d = shown.filter { ($0["target"] ?? 0) > 0 }.map { ($0["display"]! - $0["target"]!) * 1000 }
            let stamps = shown.map { $0["display"]! }.sorted()
            let gaps = zip(stamps, stamps.dropFirst()).filter { $1 - $0 > 1.5 * self.interval }.count
            let comp = rows.compactMap { r in r["completed"].flatMap { c in r["commit"].map { (c - $0) * 1000 } } }
            var out: [String: Any] = ["opts": opts, "interval_ms": self.interval * 1000, "rows": rows.count,
                                      "presented": shown.count, "ticks": self.ticks, "missing": rows.count - shown.count,
                                      "commit_to_display_ms": dist(c2d), "request_to_display_ms": dist(r2d),
                                      "commit_to_completed_ms": dist(comp), "gaps": gaps]
            if !t2d.isEmpty { out["target_to_display_ms"] = dist(t2d) }
            print(String(data: try! JSONSerialization.data(withJSONObject: out, options: [.sortedKeys]), encoding: .utf8)!)
            fflush(stdout)
            exit(0)
        }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let probe = Probe()
app.delegate = probe
app.run()
