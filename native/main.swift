import Cocoa
import WebKit

final class AppDelegate: NSObject, NSApplicationDelegate, WKNavigationDelegate, WKUIDelegate, WKScriptMessageHandler, NSWindowDelegate {
    private var window: NSWindow!
    private var webView: WKWebView!
    private var panelWCs: [NSWindowController] = []
    private var dragMonitor: Any?
    private var dragOrigin: NSPoint = .zero
    private var dragStart: NSPoint = .zero

    func applicationDidFinishLaunching(_ notification: Notification) {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        config.preferences.setValue(true, forKey: "developerExtrasEnabled")
        config.preferences.javaScriptCanOpenWindowsAutomatically = true
        config.defaultWebpagePreferences.allowsContentJavaScript = true
        config.mediaTypesRequiringUserActionForPlayback = []
        config.userContentController.add(self, name: "saveFile")
        config.userContentController.add(self, name: "windowDrag")
        config.userContentController.add(self, name: "windowZoom")
        config.userContentController.add(self, name: "pmLog")
        config.userContentController.add(self, name: "pmTheme")
        config.userContentController.add(self, name: "pmCodex")
        config.userContentController.add(self, name: "pmCaptureWindow")
        /* about:blank child WebViews do not inherit the file-read grant used by the
           main app page. Inject the packaged product CSS at document end so detached
           panels retain Powermove's styling without granting broader file access. */
        let cssDirectory = Bundle.main.resourceURL?.appendingPathComponent("web/css")
        let detachedCSS = ["tokens.css", "app.css"].compactMap { name in
            cssDirectory.flatMap { try? String(contentsOf: $0.appendingPathComponent(name), encoding: .utf8) }
        }.joined(separator: "\n")
        if !detachedCSS.isEmpty,
           let encoded = try? JSONSerialization.data(withJSONObject: detachedCSS, options: .fragmentsAllowed),
           let cssJSON = String(data: encoded, encoding: .utf8) {
            let source = "if(location.href==='about:blank'){const s=document.createElement('style');s.dataset.powermove='panel';s.textContent=\(cssJSON);document.head.appendChild(s)}"
            config.userContentController.addUserScript(WKUserScript(source: source, injectionTime: .atDocumentEnd, forMainFrameOnly: true))
        }

        webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.setValue(false, forKey: "drawsBackground")
        webView.allowsMagnification = false

        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1440, height: 900),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "Powermove"
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.isMovableByWindowBackground = false
        window.appearance = NSAppearance(named: .aqua)
        window.backgroundColor = NSColor(white: 0.929, alpha: 1)
        window.colorSpace = NSColorSpace.extendedSRGB
        window.contentView?.wantsLayer = true
        window.contentView?.layer?.wantsExtendedDynamicRangeContent = true
        window.minSize = NSSize(width: 980, height: 640)
        window.collectionBehavior = [.fullScreenPrimary]
        window.contentView = webView
        window.center()
        window.delegate = self
        window.makeKeyAndOrderFront(nil)

        /* Single source of truth for traffic lights: the REAL native buttons.
           The web UI no longer draws fake ones; it just leaves left padding so the
           native lights sit cleanly inside the custom titlebar. */
        positionTrafficLights()

        NSApp.activate(ignoringOtherApps: true)

        // Hold Shift while launching to reset the local editing state cleanly.
        if NSEvent.modifierFlags.contains(.shift) {
            WKWebsiteDataStore.default().fetchDataRecords(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes()) { [weak self] records in
                guard self != nil else { return }
                WKWebsiteDataStore.default().removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), for: records) { [weak self] in
                    self?.loadApp()
                }
            }
        } else {
            /* Installed updates reuse the same file URL. Clear only compiled web
               caches so new JS/CSS is loaded while local projects, provider
               choice, workspaces, and takes remain intact. */
            URLCache.shared.removeAllCachedResponses()
            let cacheTypes: Set<String> = [WKWebsiteDataTypeDiskCache, WKWebsiteDataTypeMemoryCache]
            WKWebsiteDataStore.default().removeData(ofTypes: cacheTypes, modifiedSince: .distantPast) { [weak self] in
                self?.loadApp()
            }
        }
    }

    /* Keep the native lights vertically centered in our compact 44px titlebar. */
    private func applyTheme(dark: Bool) {
        let name: NSAppearance.Name = dark ? .darkAqua : .aqua
        window.appearance = NSAppearance(named: name)
        window.backgroundColor = dark
            ? NSColor(red: 0.043, green: 0.043, blue: 0.047, alpha: 1)
            : NSColor(white: 0.929, alpha: 1)
        for wc in panelWCs {
            wc.window?.appearance = NSAppearance(named: name)
            wc.window?.backgroundColor = dark
                ? NSColor(red: 0.078, green: 0.078, blue: 0.086, alpha: 1)
                : NSColor(white: 0.953, alpha: 1)
        }
    }

    private func positionTrafficLights() {
        guard let close = window.standardWindowButton(.closeButton),
              let mini = window.standardWindowButton(.miniaturizeButton),
              let zoom = window.standardWindowButton(.zoomButton) else { return }
        [close, mini, zoom].forEach { $0.isHidden = false; $0.alphaValue = 1 }
        if let container = close.superview { container.isHidden = false }
    }

    /* ── native window-drag bridge ─────────────────────── */
    /* WKWebView does not honor -webkit-app-region:drag, so the HTML titlebar posts its
       pointerdown and we drive the window frame from native mouse tracking. */
    private func beginWindowDrag(startScreen: NSPoint) {
        guard let win = window else { return }
        dragOrigin = win.frame.origin
        dragStart = startScreen
        endWindowDrag()
        dragMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDragged, .leftMouseUp]) { [weak self] event in
            guard let self = self else { return event }
            if event.type == .leftMouseUp { self.endWindowDrag(); return event }
            let loc = NSEvent.mouseLocation
            let dx = loc.x - self.dragStart.x
            let dy = loc.y - self.dragStart.y
            win.setFrameOrigin(NSPoint(x: self.dragOrigin.x + dx, y: self.dragOrigin.y + dy))
            return event
        }
    }
    private func endWindowDrag() {
        if let m = dragMonitor { NSEvent.removeMonitor(m); dragMonitor = nil }
    }

    @objc func webAction(_ sender: NSMenuItem) {
        guard let cmd = sender.representedObject as? String else { return }
        webView.evaluateJavaScript("window.PM && PM.cmd && PM.cmd('\(cmd)')")
    }

    @objc func resetState(_ sender: Any?) {
        webView.evaluateJavaScript("try{localStorage.clear()}catch(e){}; location.reload()")
    }

    private func loadApp() {
        guard let resourceURL = Bundle.main.resourceURL else { return }
        let site = resourceURL.appendingPathComponent("web", isDirectory: true)
        let index = site.appendingPathComponent("index.html")
        webView.loadFileURL(index, allowingReadAccessTo: site)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        webView.evaluateJavaScript("document.documentElement.classList.add('native-app')")
        positionTrafficLights()
        /* Boot self-check: surfaces renderer state on stderr for support/diagnostics. */
        webView.evaluateJavaScript("""
        setTimeout(function() {
          try {
            var ok = !!(window.PM && PM.GL && PM.GL.gl);
            var msg = 'boot gl=' + ok +
              ' layers=' + (window.PM && PM.proj ? PM.proj.layers.length : -1) +
              ' ms=' + (PM.perf ? (PM.perf.ms || 0).toFixed(1) : '?');
            window.webkit.messageHandlers.pmLog.postMessage(msg);
          } catch (e) {}
        }, 1200);
        """)
    }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        if message.name == "pmLog" {
            if let body = message.body as? String { FileHandle.standardError.write((body + "\n").data(using: .utf8)!) }
            return
        }
        if message.name == "saveFile" {
            guard let body = message.body as? [String: Any],
                  let encoded = body["data"] as? String,
                  let bytes = Data(base64Encoded: encoded) else { return }
            let suggested = (body["name"] as? String) ?? "powermove.bin"
            let panel = NSSavePanel()
            panel.nameFieldStringValue = suggested
            panel.canCreateDirectories = true
            panel.beginSheetModal(for: window) { [weak self] response in
                guard let self = self, response == .OK, let url = panel.url else { return }
                do {
                    try bytes.write(to: url, options: .atomic)
                    var safe = url.lastPathComponent
                    safe = safe.replacingOccurrences(of: "\\", with: "\\\\")
                    safe = safe.replacingOccurrences(of: "'", with: "\\'")
                    self.webView.evaluateJavaScript("window.PM && PM.toast && PM.toast('Saved \(safe)')")
                } catch {
                    self.webView.evaluateJavaScript("window.PM && PM.toast && PM.toast('Save failed')")
                }
            }
            return
        }
        if message.name == "windowDrag" {
            guard let body = message.body as? [String: Any],
                  let x = body["x"] as? CGFloat, let y = body["y"] as? CGFloat,
                  let win = window else { return }
            let contentRect = win.contentRect(forFrameRect: win.frame)
            let screenPt = NSPoint(x: contentRect.origin.x + x, y: contentRect.origin.y + contentRect.size.height - y)
            beginWindowDrag(startScreen: screenPt)
            return
        }
        if message.name == "windowZoom" {
            window.performZoom(nil)
            return
        }
        if message.name == "pmTheme" {
            let dark = (message.body as? String) == "dark"
            applyTheme(dark: dark)
            return
        }
        if message.name == "pmCodex" {
            guard let body = message.body as? [String: Any],
                  let requestId = body["id"] as? String,
                  let prompt = body["prompt"] as? String,
                  !prompt.isEmpty, prompt.utf8.count < 400_000 else { return }
            let images = (body["images"] as? [String] ?? []).prefix(6).filter { $0.utf8.count < 6_000_000 }
            runCodex(requestId: requestId, prompt: prompt, schema: body["schema"], images: Array(images))
            return
        }
        if message.name == "pmCaptureWindow" {
            guard let body = message.body as? [String: Any],
                  let requestId = body["id"] as? String else { return }
            captureWindow(requestId: requestId)
            return
        }
    }

    /* WebGPU cannot directly sample DOM pixels. Capture the live WKWebView before
       the overlay mounts so the WGSL ripple can genuinely displace the interface. */
    private func captureWindow(requestId: String) {
        let configuration = WKSnapshotConfiguration()
        configuration.afterScreenUpdates = true
        webView.takeSnapshot(with: configuration) { [weak self] image, error in
            guard let self = self,
                  error == nil,
                  let tiff = image?.tiffRepresentation,
                  let bitmap = NSBitmapImageRep(data: tiff),
                  let png = bitmap.representation(using: .png, properties: [:]) else {
                self?.sendWindowCapture(requestId: requestId, ok: false, data: nil)
                return
            }
            self.sendWindowCapture(requestId: requestId, ok: true, data: png)
        }
    }

    private func sendWindowCapture(requestId: String, ok: Bool, data: Data?) {
        let payload: [String: Any] = ["ok": ok, "dataBase64": data?.base64EncodedString() ?? ""]
        guard let encoded = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: encoded, encoding: .utf8) else { return }
        let safeId = requestId.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "'", with: "\\'")
        webView.evaluateJavaScript("window.PM && PM.WindowCapture && PM.WindowCapture.resolve('\(safeId)', \(json))")
    }

    /* ChatGPT subscription bridge. Powermove never reads account tokens: it asks
       the already-authenticated local Codex client for a structured action plan. */
    private func codexBinaryURL() -> URL? {
        let fm = FileManager.default
        let candidates = [
            ProcessInfo.processInfo.environment["CODEX_BINARY"],
            "/opt/homebrew/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex",
            "/usr/local/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex"
        ].compactMap { $0 }
        return candidates.first(where: { fm.isExecutableFile(atPath: $0) }).map(URL.init(fileURLWithPath:))
    }

    private func runCodex(requestId: String, prompt: String, schema: Any?, images: [String] = []) {
        guard let executable = codexBinaryURL() else {
            sendCodexResult(requestId: requestId, ok: false, text: "Codex is not installed. Install Codex and sign in with ChatGPT first.")
            return
        }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            let fm = FileManager.default
            let directory = fm.temporaryDirectory.appendingPathComponent("powermove-codex-\(UUID().uuidString)", isDirectory: true)
            do {
                try fm.createDirectory(at: directory, withIntermediateDirectories: true)
                let schemaURL = directory.appendingPathComponent("schema.json")
                let outputURL = directory.appendingPathComponent("result.json")
                let schemaObject = schema ?? ["type": "object"]
                let schemaData = try JSONSerialization.data(withJSONObject: schemaObject, options: [.prettyPrinted, .sortedKeys])
                try schemaData.write(to: schemaURL, options: .atomic)

                var imageURLs: [URL] = []
                for (index, encoded) in images.enumerated() {
                    guard let comma = encoded.firstIndex(of: ",") else { continue }
                    let header = String(encoded[..<comma])
                    let payload = String(encoded[encoded.index(after: comma)...])
                    guard header.hasPrefix("data:image/"),
                          let data = Data(base64Encoded: payload),
                          data.count <= 4_000_000 else { continue }
                    let ext = header.contains("image/png") ? "png" : "jpg"
                    let imageURL = directory.appendingPathComponent("frame-\(index).\(ext)")
                    try data.write(to: imageURL, options: .atomic)
                    imageURLs.append(imageURL)
                }

                let process = Process()
                process.executableURL = executable
                process.currentDirectoryURL = directory
                var arguments = [
                    "exec", "--ephemeral", "--skip-git-repo-check", "--ignore-rules",
                    "--sandbox", "read-only", "--output-schema", schemaURL.path,
                    "--output-last-message", outputURL.path,
                ]
                /* --image accepts a variadic list. Keep the positional prompt
                   before it or Codex will consume the prompt as another path
                   and fall back to an empty stdin prompt. */
                arguments.append(prompt)
                for imageURL in imageURLs { arguments.append(contentsOf: ["--image", imageURL.path]) }
                process.arguments = arguments
                let errors = Pipe()
                process.standardOutput = Pipe()
                process.standardError = errors
                try process.run()
                process.waitUntilExit()
                if process.terminationStatus == 0, fm.fileExists(atPath: outputURL.path) {
                    let text = try String(contentsOf: outputURL, encoding: .utf8)
                    self.sendCodexResult(requestId: requestId, ok: true, text: text)
                } else {
                    let data = errors.fileHandleForReading.readDataToEndOfFile()
                    let message = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
                    self.sendCodexResult(requestId: requestId, ok: false, text: message?.isEmpty == false ? message! : "ChatGPT generation failed.")
                }
            } catch {
                self.sendCodexResult(requestId: requestId, ok: false, text: error.localizedDescription)
            }
            try? fm.removeItem(at: directory)
        }
    }

    private func sendCodexResult(requestId: String, ok: Bool, text: String) {
        let encoded = Data(text.utf8).base64EncodedString()
        let payload: [String: Any] = ["ok": ok, "dataBase64": encoded]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8) else { return }
        let safeId = requestId.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "'", with: "\\'")
        DispatchQueue.main.async { [weak self] in
            self?.webView.evaluateJavaScript("window.PM && PM.CodexBridge && PM.CodexBridge.resolve('\(safeId)', \(json))")
        }
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        if let url = navigationAction.request.url,
           navigationAction.navigationType == .linkActivated,
           url.scheme != "file" {
            NSWorkspace.shared.open(url)
            decisionHandler(.cancel)
            return
        }
        decisionHandler(.allow)
    }

    /* window.open() from the page → host the panel in a real child NSWindow.
       Returning the new WKWebView gives the parent a live window reference so it can
       reparent the actual panel element into the popout (same JS realm). */
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        let wv = WKWebView(frame: .zero, configuration: configuration)
        wv.navigationDelegate = self
        wv.uiDelegate = self
        wv.setValue(false, forKey: "drawsBackground")
        let w = max(windowFeatures.width?.doubleValue ?? 480, 320)
        let h = max(windowFeatures.height?.doubleValue ?? 620, 240)
        let win = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: w, height: h),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        win.title = "Panel"
        win.appearance = NSAppearance(named: .aqua)
        win.backgroundColor = NSColor(white: 0.953, alpha: 1)
        win.contentView = wv
        win.isReleasedWhenClosed = false
        win.delegate = self
        /* Closing the child is the pin-back signal. Target the window directly,
           so this remains reliable even when the accessory is not first responder. */
        let returnButton = NSButton(title: "Return to layout", target: win, action: #selector(NSWindow.performClose(_:)))
        returnButton.bezelStyle = .rounded
        returnButton.controlSize = .small
        returnButton.toolTip = "Pin this panel back into the main Powermove layout"
        let accessory = NSTitlebarAccessoryViewController()
        accessory.view = returnButton
        accessory.layoutAttribute = .right
        win.addTitlebarAccessoryViewController(accessory)
        let wc = NSWindowController(window: win)
        panelWCs.append(wc)
        win.setFrameOrigin(NSPoint(x: window.frame.midX - w / 2, y: window.frame.midY - h / 2))
        wc.showWindow(nil)
        NSApp.activate(ignoringOtherApps: true)
        return wv
    }

    func webView(_ webView: WKWebView, runOpenPanelWith parameters: WKOpenPanelParameters, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping ([URL]?) -> Void) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = parameters.allowsMultipleSelection
        panel.canChooseDirectories = parameters.allowsDirectories
        panel.canChooseFiles = true
        panel.beginSheetModal(for: window) { response in
            completionHandler(response == .OK ? panel.urls : nil)
        }
    }

    func windowWillClose(_ notification: Notification) {
        if (notification.object as? NSWindow) === window {
            NSApp.terminate(nil)
        } else if let closed = notification.object as? NSWindow {
            panelWCs.removeAll { $0.window === closed }
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    @objc func reload(_ sender: Any?) { webView.reload() }
    @objc func showInspector(_ sender: Any?) { webView.configuration.preferences.setValue(true, forKey: "developerExtrasEnabled") }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)

let mainMenu = NSMenu()
let appItem = NSMenuItem()
mainMenu.addItem(appItem)
let appMenu = NSMenu()
appMenu.addItem(withTitle: "About Powermove", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
appMenu.addItem(.separator())
appMenu.addItem(withTitle: "Hide Powermove", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
appMenu.addItem(withTitle: "Hide Others", action: #selector(NSApplication.hideOtherApplications(_:)), keyEquivalent: "h").keyEquivalentModifierMask = [.command, .option]
appMenu.addItem(.separator())
appMenu.addItem(withTitle: "Quit Powermove", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
appItem.submenu = appMenu

let fileItem = NSMenuItem()
mainMenu.addItem(fileItem)
let fileMenu = NSMenu(title: "File")
func fileItem_(_ title: String, _ key: String, _ action: Selector, _ rep: String? = nil) -> NSMenuItem {
    let mi = NSMenuItem(title: title, action: action, keyEquivalent: key)
    mi.representedObject = rep
    return mi
}
fileMenu.addItem(fileItem_("New Project", "n", #selector(AppDelegate.webAction), "newProject"))
fileMenu.addItem(fileItem_("Save Project", "s", #selector(AppDelegate.webAction), "save"))
fileMenu.addItem(fileItem_("Open Project…", "o", #selector(AppDelegate.webAction), "open"))
fileMenu.addItem(.separator())
fileMenu.addItem(fileItem_("Export…", "e", #selector(AppDelegate.webAction), "export"))
fileItem.submenu = fileMenu

let editItem = NSMenuItem()
mainMenu.addItem(editItem)
let editMenu = NSMenu(title: "Edit")
editMenu.addItem(fileItem_("Undo", "z", #selector(AppDelegate.webAction), "undo"))
editMenu.addItem(fileItem_("Redo", "Z", #selector(AppDelegate.webAction), "redo"))
editMenu.addItem(.separator())
editMenu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
editMenu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
editMenu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
editMenu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
editItem.submenu = editMenu

let windowItem = NSMenuItem()
mainMenu.addItem(windowItem)
let windowMenu = NSMenu(title: "Window")
windowMenu.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
windowMenu.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
windowMenu.addItem(.separator())
windowMenu.addItem(withTitle: "Enter Full Screen", action: #selector(NSWindow.toggleFullScreen(_:)), keyEquivalent: "f").keyEquivalentModifierMask = [.command, .control]
windowItem.submenu = windowMenu
app.windowsMenu = windowMenu
app.mainMenu = mainMenu
app.run()
