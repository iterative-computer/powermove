import Cocoa
import WebKit
import UniformTypeIdentifiers

private final class CodexLineBuffer {
    private var data = Data()
    private let lock = NSLock()
    private let receive: (Data) -> Void

    init(receive: @escaping (Data) -> Void) { self.receive = receive }

    func append(_ chunk: Data, flush: Bool = false) {
        lock.lock()
        data.append(chunk)
        var lines: [Data] = []
        while let newline = data.firstIndex(of: 0x0A) {
            lines.append(data.prefix(upTo: newline))
            data.removeSubrange(...newline)
        }
        if flush && !data.isEmpty { lines.append(data); data.removeAll() }
        lock.unlock()
        lines.filter { !$0.isEmpty }.forEach(receive)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, WKNavigationDelegate, WKUIDelegate, WKScriptMessageHandler, NSWindowDelegate {
    private var window: NSWindow!
    private var webView: WKWebView!
    private var panelWCs: [NSWindowController] = []
    private var dragMonitor: Any?
    private var dragEndMonitor: Any?
    private var dragOrigin: NSPoint = .zero
    private var dragStart: NSPoint = .zero
    private let codexStateQueue = DispatchQueue(label: "com.zellzoi.powermove.codex-state")
    private var codexProcesses: [String: Process] = [:]
    private var cancelledCodexRequests = Set<String>()

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
        config.userContentController.add(self, name: "pmCodexCancel")
        config.userContentController.add(self, name: "pmAgentArtifact")
        config.userContentController.add(self, name: "pmAgentReveal")
        config.userContentController.add(self, name: "pmCaptureWindow")
        config.userContentController.add(self, name: "pmPanelTitle")
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
        endWindowDrag()
        dragOrigin = win.frame.origin
        dragStart = startScreen
        /* A lost mouse-up must never leave the native titlebar tracker armed.
           A later panel gesture begins with a fresh mouse-down, so use that as
           an unambiguous boundary before any of its drag events can move the window. */
        dragMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .leftMouseDragged, .leftMouseUp]) { [weak self] event in
            guard let self = self else { return event }
            if event.type == .leftMouseDown || event.type == .leftMouseUp {
                self.endWindowDrag()
                return event
            }
            let loc = NSEvent.mouseLocation
            let dx = loc.x - self.dragStart.x
            let dy = loc.y - self.dragStart.y
            win.setFrameOrigin(NSPoint(x: self.dragOrigin.x + dx, y: self.dragOrigin.y + dy))
            return event
        }
        /* Local monitors do not receive a release delivered to another app. */
        dragEndMonitor = NSEvent.addGlobalMonitorForEvents(matching: .leftMouseUp) { [weak self] _ in
            self?.endWindowDrag()
        }
    }
    private func endWindowDrag() {
        if let m = dragMonitor { NSEvent.removeMonitor(m); dragMonitor = nil }
        if let m = dragEndMonitor { NSEvent.removeMonitor(m); dragEndMonitor = nil }
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
        publishAvailableFonts()
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

    /* Canvas can render every macOS font, but JavaScript cannot enumerate those
       families reliably. Publish AppKit's real catalog after the editor boots. */
    private func publishAvailableFonts() {
        let families = NSFontManager.shared.availableFontFamilies.sorted {
            $0.localizedCaseInsensitiveCompare($1) == .orderedAscending
        }
        guard let data = try? JSONSerialization.data(withJSONObject: families),
              let json = String(data: data, encoding: .utf8) else { return }
        webView.evaluateJavaScript("window.PM && PM.Fonts && PM.Fonts.setSystemFamilies(\(json))")
    }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        if message.name == "pmPanelTitle" {
            guard let title = message.body as? String, !title.isEmpty,
                  let childWebView = message.webView,
                  let childWindow = panelWCs.compactMap({ $0.window }).first(where: { $0.contentView === childWebView }) else { return }
            childWindow.title = String(title.prefix(80))
            return
        }
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
            let allowedModels = ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"]
            let allowedEfforts = ["low", "medium", "high", "xhigh", "max"]
            let requestedModel = body["model"] as? String
            let requestedEffort = body["reasoningEffort"] as? String
            let model = requestedModel.flatMap { allowedModels.contains($0) ? $0 : nil }
            let effort = requestedEffort.flatMap { allowedEfforts.contains($0) ? $0 : nil }
            if (body["mode"] as? String) == "autonomous" {
                guard let projectId = body["projectId"] as? String,
                      let projectName = body["projectName"] as? String,
                      let projectJSON = body["projectJSON"] as? String,
                      !projectId.isEmpty, projectId.utf8.count <= 160,
                      projectName.utf8.count <= 240,
                      projectJSON.utf8.count <= 24_000_000 else { return }
                let access = (body["access"] as? String) == "computer" ? "computer" : "project"
                let attachments = (body["attachments"] as? [[String: Any]] ?? []).prefix(6)
                runAutonomousCodex(
                    requestId: requestId, prompt: prompt, projectId: projectId,
                    projectName: projectName, projectJSON: projectJSON,
                    access: access, attachments: Array(attachments), images: Array(images),
                    model: model, reasoningEffort: effort)
            } else {
                runCodex(requestId: requestId, prompt: prompt, schema: body["schema"], images: Array(images), model: model, reasoningEffort: effort)
            }
            return
        }
        if message.name == "pmCodexCancel" {
            guard let body = message.body as? [String: Any],
                  let requestId = body["id"] as? String else { return }
            cancelCodex(requestId: requestId)
            return
        }
        if message.name == "pmAgentArtifact" {
            guard let body = message.body as? [String: Any],
                  let requestId = body["id"] as? String,
                  let projectId = body["projectId"] as? String,
                  let path = body["path"] as? String else { return }
            loadAgentArtifact(requestId: requestId, projectId: projectId, relativePath: path)
            return
        }
        if message.name == "pmAgentReveal" {
            guard let body = message.body as? [String: Any],
                  let projectId = body["projectId"] as? String else { return }
            revealAgentArtifact(projectId: projectId, relativePath: body["path"] as? String)
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

    /* ── autonomous project agent ──────────────────────── */
    private func safeAgentComponent(_ value: String, fallback: String = "project") -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_"))
        let cleaned = value.unicodeScalars.map { allowed.contains($0) ? Character(String($0)) : "-" }
        let result = String(cleaned).replacingOccurrences(of: "-+", with: "-", options: .regularExpression)
            .trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        return String((result.isEmpty ? fallback : result).prefix(120))
    }

    private func agentWorkspaceURL(projectId: String) throws -> URL {
        let fm = FileManager.default
        let support = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let root = support.appendingPathComponent("Powermove", isDirectory: true)
            .appendingPathComponent("Agent Workspaces", isDirectory: true)
        let workspace = root.appendingPathComponent(safeAgentComponent(projectId), isDirectory: true)
        try fm.createDirectory(at: workspace, withIntermediateDirectories: true)
        return workspace
    }

    private func agentResultSchema() -> [String: Any] {
        [
            "type": "object", "additionalProperties": false,
            "required": ["summary", "commands", "artifacts", "externalActions", "notes"],
            "properties": [
                "summary": ["type": "string"],
                "commands": ["type": "array", "maxItems": 80, "items": ["type": "string"]],
                "artifacts": [
                    "type": "array", "maxItems": 80,
                    "items": [
                        "type": "object", "additionalProperties": false,
                        "required": ["path", "importToTimeline"],
                        "properties": [
                            "path": ["type": "string"],
                            "importToTimeline": ["type": "boolean"],
                        ],
                    ],
                ],
                "externalActions": ["type": "array", "maxItems": 80, "items": ["type": "string"]],
                "notes": ["type": "array", "maxItems": 80, "items": ["type": "string"]],
            ],
        ]
    }

    private func agentInstructions(projectName: String, artifactRelativePath: String, access: String) -> String {
        """
        You are the general production agent working beside Powermove. Complete the user's request end to end, using web search, shell tools, installed creative applications, and reusable integrations when useful. The current editable Powermove project snapshot is inputs/powermove-project.json. Treat it as read-only reference; return Powermove edits through typed commands instead of rewriting that file.

        Place every deliverable file under the artifacts directory for this run: \(artifactRelativePath). Do not leave deliverables elsewhere. You may create project-local scripts, notes, and adapters in this workspace when they help finish the task.

        Supported Powermove command types are: set_property, replace_keyframes, set_easing, set_expression, set_content, set_layer, set_composition, add_layer, delete_layers, reorder_layer, add_effect, remove_effect, set_effect, set_scene_parameter, add_marker, create_section, update_section, transform_layers. Return each command as one JSON-encoded string. Files that should become editable media layers must be listed in artifacts with importToTimeline=true.

        Record uploads, messages, publications, remote changes, application launches, or other outside-world side effects in externalActions. Never claim an external action succeeded unless a tool result proves it. The active authority is \(access). Project authority limits writes to this project workspace; computer authority was explicitly granted for this run and may operate outside it when required by the user's request.

        Project: \(projectName)
        """
    }

    private func captureCodexSession(from data: Data, sessionURL: URL) {
        guard let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              event["type"] as? String == "thread.started",
              let threadId = event["thread_id"] as? String,
              !threadId.isEmpty else { return }
        try? threadId.write(to: sessionURL, atomically: true, encoding: .utf8)
    }

    private func mimeType(for url: URL) -> String {
        UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
    }

    private func collectAgentArtifacts(
        workspace: URL, runDirectory: URL, runId: String,
        requested: [[String: Any]]) -> [[String: Any]] {
        let fm = FileManager.default
        let requestedImports = Set(requested.compactMap { item -> String? in
            guard (item["importToTimeline"] as? Bool) == true,
                  let path = item["path"] as? String else { return nil }
            return path
        })
        guard let enumerator = fm.enumerator(
            at: runDirectory,
            includingPropertiesForKeys: [.isRegularFileKey, .fileSizeKey],
            options: [.skipsHiddenFiles, .skipsPackageDescendants]) else { return [] }
        var artifacts: [[String: Any]] = []
        for case let url as URL in enumerator {
            guard artifacts.count < 80,
                  let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey]),
                  values.isRegularFile == true else { continue }
            let local = url.path.replacingOccurrences(of: runDirectory.path + "/", with: "")
            let path = runId + "/" + local
            let requestedImport = requestedImports.contains(path) || requestedImports.contains(local)
                || requestedImports.contains(url.lastPathComponent)
            artifacts.append([
                "path": path,
                "name": url.lastPathComponent,
                "size": values.fileSize ?? 0,
                "mime": mimeType(for: url),
                "importToTimeline": requestedImport,
            ])
        }
        return artifacts.sorted { (($0["path"] as? String) ?? "") < (($1["path"] as? String) ?? "") }
    }

    private func runAutonomousCodex(
        requestId: String, prompt: String, projectId: String, projectName: String,
        projectJSON: String, access: String, attachments: [[String: Any]],
        images: [String], model: String?, reasoningEffort: String?) {
        guard let executable = codexBinaryURL() else {
            sendCodexResult(requestId: requestId, ok: false, text: "Codex is not installed. Install Codex and sign in with ChatGPT first.")
            return
        }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            let fm = FileManager.default
            defer { self.finishCodexRequest(requestId) }
            do {
                let workspace = try self.agentWorkspaceURL(projectId: projectId)
                let inputs = workspace.appendingPathComponent("inputs", isDirectory: true)
                let internalDir = workspace.appendingPathComponent(".powermove", isDirectory: true)
                let artifactRoot = workspace.appendingPathComponent("artifacts", isDirectory: true)
                let runId = self.safeAgentComponent("\(Int(Date().timeIntervalSince1970))-\(UUID().uuidString)", fallback: "run")
                let runDirectory = artifactRoot.appendingPathComponent(runId, isDirectory: true)
                try fm.createDirectory(at: inputs, withIntermediateDirectories: true)
                try fm.createDirectory(at: internalDir, withIntermediateDirectories: true)
                try fm.createDirectory(at: runDirectory, withIntermediateDirectories: true)
                try projectJSON.write(
                    to: inputs.appendingPathComponent("powermove-project.json"),
                    atomically: true, encoding: .utf8)

                let attachmentDir = inputs.appendingPathComponent("attachments", isDirectory: true)
                try fm.createDirectory(at: attachmentDir, withIntermediateDirectories: true)
                for (index, item) in attachments.enumerated() {
                    guard let content = item["content"] as? String, content.utf8.count <= 100_000 else { continue }
                    let name = self.safeAgentComponent(item["name"] as? String ?? "attachment-\(index).txt", fallback: "attachment-\(index).txt")
                    try content.write(to: attachmentDir.appendingPathComponent(name), atomically: true, encoding: .utf8)
                }

                var imageURLs: [URL] = []
                let referenceDir = inputs.appendingPathComponent("references", isDirectory: true)
                try fm.createDirectory(at: referenceDir, withIntermediateDirectories: true)
                for (index, encoded) in images.enumerated() {
                    guard let comma = encoded.firstIndex(of: ",") else { continue }
                    let header = String(encoded[..<comma])
                    let payload = String(encoded[encoded.index(after: comma)...])
                    guard header.hasPrefix("data:image/"),
                          let data = Data(base64Encoded: payload), data.count <= 4_000_000 else { continue }
                    let ext = header.contains("image/png") ? "png" : "jpg"
                    let url = referenceDir.appendingPathComponent("reference-\(index).\(ext)")
                    try data.write(to: url, options: .atomic)
                    imageURLs.append(url)
                }

                let schemaURL = internalDir.appendingPathComponent("powermove-result-schema.json")
                let outputURL = internalDir.appendingPathComponent("result-\(runId).json")
                let schemaData = try JSONSerialization.data(withJSONObject: self.agentResultSchema(), options: [.prettyPrinted, .sortedKeys])
                try schemaData.write(to: schemaURL, options: .atomic)
                let sessionURL = internalDir.appendingPathComponent(access == "computer" ? "session-computer.txt" : "session-project.txt")
                let sessionId = (try? String(contentsOf: sessionURL, encoding: .utf8))?
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                let instructions = self.agentInstructions(
                    projectName: projectName,
                    artifactRelativePath: "artifacts/\(runId)", access: access)
                let fullPrompt = instructions + "\n\nUSER REQUEST\n" + prompt

                let process = Process()
                process.executableURL = executable
                process.currentDirectoryURL = workspace
                var arguments: [String]
                if let sessionId = sessionId, !sessionId.isEmpty {
                    arguments = ["--search"]
                    if access == "computer" {
                        arguments.append("--dangerously-bypass-approvals-and-sandbox")
                    } else {
                        arguments.append(contentsOf: ["--sandbox", "workspace-write", "--approve-for-me"])
                    }
                    arguments.append(contentsOf: [
                        "exec", "resume", "--skip-git-repo-check",
                        "--output-schema", schemaURL.path,
                        "--output-last-message", outputURL.path, "--json",
                    ])
                    if let model = model { arguments.append(contentsOf: ["--model", model]) }
                    if let effort = reasoningEffort { arguments.append(contentsOf: ["--config", "model_reasoning_effort=\"\(effort)\""]) }
                    arguments.append(sessionId)
                } else {
                    arguments = ["--search"]
                    if access == "computer" {
                        arguments.append("--dangerously-bypass-approvals-and-sandbox")
                    } else {
                        arguments.append(contentsOf: ["--sandbox", "workspace-write", "--approve-for-me"])
                    }
                    arguments.append(contentsOf: [
                        "exec",
                        "--skip-git-repo-check",
                        "--output-schema", schemaURL.path,
                        "--output-last-message", outputURL.path, "--json",
                    ])
                    if let model = model { arguments.append(contentsOf: ["--model", model]) }
                    if let effort = reasoningEffort { arguments.append(contentsOf: ["--config", "model_reasoning_effort=\"\(effort)\""]) }
                }
                arguments.append(fullPrompt)
                for imageURL in imageURLs {
                    arguments.append("--image")
                    arguments.append(imageURL.path)
                }
                process.arguments = arguments

                let events = Pipe()
                let errors = Pipe()
                let eventBuffer = CodexLineBuffer { [weak self] line in
                    self?.captureCodexSession(from: line, sessionURL: sessionURL)
                    self?.forwardCodexEvent(requestId: requestId, data: line)
                }
                events.fileHandleForReading.readabilityHandler = { handle in
                    let data = handle.availableData
                    if data.isEmpty { handle.readabilityHandler = nil; return }
                    eventBuffer.append(data)
                }
                process.standardOutput = events
                process.standardError = errors
                if self.codexWasCancelled(requestId) { return }
                try process.run()
                if !self.registerCodexProcess(process, requestId: requestId), process.isRunning { process.terminate() }
                process.waitUntilExit()
                events.fileHandleForReading.readabilityHandler = nil
                eventBuffer.append(events.fileHandleForReading.readDataToEndOfFile(), flush: true)
                if self.codexWasCancelled(requestId) { return }
                guard process.terminationStatus == 0, fm.fileExists(atPath: outputURL.path) else {
                    let data = errors.fileHandleForReading.readDataToEndOfFile()
                    let message = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
                    self.sendCodexResult(requestId: requestId, ok: false, text: message?.isEmpty == false ? message! : "The autonomous agent failed.")
                    return
                }
                let resultData = try Data(contentsOf: outputURL)
                guard var result = try JSONSerialization.jsonObject(with: resultData) as? [String: Any] else {
                    throw NSError(domain: "PowermoveAgent", code: 1, userInfo: [NSLocalizedDescriptionKey: "The autonomous agent returned an invalid result."])
                }
                let requested = result["artifacts"] as? [[String: Any]] ?? []
                result["artifacts"] = self.collectAgentArtifacts(
                    workspace: workspace, runDirectory: runDirectory, runId: runId, requested: requested)
                result["projectId"] = projectId
                result["access"] = access
                let normalized = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
                self.sendCodexResult(requestId: requestId, ok: true, text: String(data: normalized, encoding: .utf8) ?? "{}")
            } catch {
                if !self.codexWasCancelled(requestId) {
                    self.sendCodexResult(requestId: requestId, ok: false, text: error.localizedDescription)
                }
            }
        }
    }

    private func validatedAgentArtifactURL(projectId: String, relativePath: String) -> URL? {
        guard !relativePath.isEmpty, !relativePath.hasPrefix("/"), !relativePath.contains("..") else { return nil }
        guard let workspace = try? agentWorkspaceURL(projectId: projectId) else { return nil }
        let root = workspace.appendingPathComponent("artifacts", isDirectory: true).resolvingSymlinksInPath().standardizedFileURL
        let candidate = root.appendingPathComponent(relativePath).resolvingSymlinksInPath().standardizedFileURL
        guard candidate.path.hasPrefix(root.path + "/") else { return nil }
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: candidate.path, isDirectory: &isDirectory), !isDirectory.boolValue else { return nil }
        return candidate
    }

    private func loadAgentArtifact(requestId: String, projectId: String, relativePath: String) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self,
                  let url = self.validatedAgentArtifactURL(projectId: projectId, relativePath: relativePath),
                  let values = try? url.resourceValues(forKeys: [.fileSizeKey]),
                  let size = values.fileSize, size <= 64 * 1024 * 1024,
                  let data = try? Data(contentsOf: url) else {
                self?.sendAgentArtifact(requestId: requestId, ok: false, payload: ["message": "This artifact is unavailable or larger than 64 MB. Reveal it in Finder and import it normally."])
                return
            }
            self.sendAgentArtifact(requestId: requestId, ok: true, payload: [
                "name": url.lastPathComponent,
                "mime": self.mimeType(for: url),
                "dataBase64": data.base64EncodedString(),
            ])
        }
    }

    private func sendAgentArtifact(requestId: String, ok: Bool, payload: [String: Any]) {
        var result = payload
        result["ok"] = ok
        guard let data = try? JSONSerialization.data(withJSONObject: result),
              let json = String(data: data, encoding: .utf8) else { return }
        let safeId = requestId.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "'", with: "\\'")
        DispatchQueue.main.async { [weak self] in
            self?.webView.evaluateJavaScript("window.PM && PM.AgentArtifacts && PM.AgentArtifacts.resolve('\(safeId)', \(json))")
        }
    }

    private func revealAgentArtifact(projectId: String, relativePath: String?) {
        guard let workspace = try? agentWorkspaceURL(projectId: projectId) else { return }
        let artifactRoot = workspace.appendingPathComponent("artifacts", isDirectory: true)
        let target = relativePath.flatMap { validatedAgentArtifactURL(projectId: projectId, relativePath: $0) }
        DispatchQueue.main.async {
            if let target = target { NSWorkspace.shared.activateFileViewerSelecting([target]) }
            else { NSWorkspace.shared.open(artifactRoot) }
        }
    }

    private func cancelCodex(requestId: String) {
        let process = codexStateQueue.sync { () -> Process? in
            cancelledCodexRequests.insert(requestId)
            return codexProcesses[requestId]
        }
        if process?.isRunning == true { process?.terminate() }
    }

    private func registerCodexProcess(_ process: Process, requestId: String) -> Bool {
        codexStateQueue.sync {
            if cancelledCodexRequests.contains(requestId) { return false }
            codexProcesses[requestId] = process
            return true
        }
    }

    private func codexWasCancelled(_ requestId: String) -> Bool {
        codexStateQueue.sync { cancelledCodexRequests.contains(requestId) }
    }

    private func finishCodexRequest(_ requestId: String) {
        codexStateQueue.sync {
            codexProcesses.removeValue(forKey: requestId)
            cancelledCodexRequests.remove(requestId)
        }
    }

    private func runCodex(requestId: String, prompt: String, schema: Any?, images: [String] = [], model: String? = nil, reasoningEffort: String? = nil) {
        guard let executable = codexBinaryURL() else {
            sendCodexResult(requestId: requestId, ok: false, text: "Codex is not installed. Install Codex and sign in with ChatGPT first.")
            return
        }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            let fm = FileManager.default
            let directory = fm.temporaryDirectory.appendingPathComponent("powermove-codex-\(UUID().uuidString)", isDirectory: true)
            defer {
                self.finishCodexRequest(requestId)
                try? fm.removeItem(at: directory)
            }
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
                    "--output-last-message", outputURL.path, "--json",
                ]
                if let model = model { arguments.append(contentsOf: ["--model", model]) }
                if let effort = reasoningEffort { arguments.append(contentsOf: ["--config", "model_reasoning_effort=\"\(effort)\""]) }
                /* --image accepts a variadic list. Keep the positional prompt
                   before it or Codex will consume the prompt as another path
                   and fall back to an empty stdin prompt. */
                arguments.append(prompt)
                for imageURL in imageURLs { arguments.append(contentsOf: ["--image", imageURL.path]) }
                process.arguments = arguments
                let events = Pipe()
                let errors = Pipe()
                let eventBuffer = CodexLineBuffer { [weak self] line in
                    self?.forwardCodexEvent(requestId: requestId, data: line)
                }
                events.fileHandleForReading.readabilityHandler = { handle in
                    let data = handle.availableData
                    if data.isEmpty { handle.readabilityHandler = nil; return }
                    eventBuffer.append(data)
                }
                process.standardOutput = events
                process.standardError = errors
                if self.codexWasCancelled(requestId) { return }
                try process.run()
                if !self.registerCodexProcess(process, requestId: requestId), process.isRunning { process.terminate() }
                process.waitUntilExit()
                events.fileHandleForReading.readabilityHandler = nil
                eventBuffer.append(events.fileHandleForReading.readDataToEndOfFile(), flush: true)
                if self.codexWasCancelled(requestId) { return }
                if process.terminationStatus == 0, fm.fileExists(atPath: outputURL.path) {
                    let text = try String(contentsOf: outputURL, encoding: .utf8)
                    self.sendCodexResult(requestId: requestId, ok: true, text: text)
                } else {
                    let data = errors.fileHandleForReading.readDataToEndOfFile()
                    let message = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
                    self.sendCodexResult(requestId: requestId, ok: false, text: message?.isEmpty == false ? message! : "ChatGPT generation failed.")
                }
            } catch {
                if self.codexWasCancelled(requestId) { return }
                self.sendCodexResult(requestId: requestId, ok: false, text: error.localizedDescription)
            }
        }
    }

    private func forwardCodexEvent(requestId: String, data: Data) {
        guard let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let eventType = event["type"] as? String,
              let item = event["item"] as? [String: Any],
              let type = item["type"] as? String else { return }
        if eventType == "item.started" {
            let activity: String?
            switch type {
            case "command_execution": activity = "Working with project files and shell tools…"
            case "web_search": activity = "Researching on the web…"
            case "mcp_tool_call":
                let tool = (item["tool"] as? String) ?? (item["name"] as? String) ?? "integration"
                activity = "Using the installed \(String(tool.prefix(80))) integration…"
            case "computer_use": activity = "Operating an application on this Mac…"
            case "image_generation": activity = "Generating a visual deliverable…"
            case "file_change": activity = "Preparing project files…"
            default: activity = nil
            }
            if let activity = activity { sendCodexProgress(requestId: requestId, text: activity) }
            return
        }
        guard eventType == "item.completed",
              ["reasoning", "agent_message"].contains(type),
              var raw = item["text"] as? String else { return }
        /* Current Codex versions expose user-facing reasoning summaries as an
           intermediate agent_message. Extract only a short display field from
           structured JSON; never stream edit commands or the full response. */
        if type == "agent_message", let encoded = raw.data(using: .utf8),
           let object = try? JSONSerialization.jsonObject(with: encoded) as? [String: Any] {
            let fields = ["message", "summary", "critique", "status"]
            raw = fields.compactMap { object[$0] as? String }.first { $0.count > 8 } ?? ""
        }
        let summary = raw.replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !summary.isEmpty else { return }
        sendCodexProgress(requestId: requestId, text: String(summary.prefix(320)))
    }

    private func sendCodexProgress(requestId: String, text: String) {
        let encoded = Data(text.utf8).base64EncodedString()
        let payload: [String: Any] = ["dataBase64": encoded]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8) else { return }
        let safeId = requestId.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "'", with: "\\'")
        DispatchQueue.main.async { [weak self] in
            self?.webView.evaluateJavaScript("window.PM && PM.CodexBridge && PM.CodexBridge.progress('\(safeId)', \(json))")
        }
    }

    private func sendCodexResult(requestId: String, ok: Bool, text: String) {
        let encoded = Data(text.utf8).base64EncodedString()
        let payload: [String: Any] = ["ok": ok, "dataBase64": encoded]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8) else { return }
        let safeId = requestId.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "'", with: "\\'")
        DispatchQueue.main.async { [weak self] in
            self?.webView.evaluateJavaScript("window.PM && PM.CodexBridge && PM.CodexBridge.resolve('\(safeId)', \(json))") { _, error in
                if let error = error { print("[codex-native] callback failed: \(error.localizedDescription)") }
            }
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
        win.title = "Powermove agent"
        win.appearance = NSAppearance(named: .aqua)
        win.backgroundColor = NSColor(white: 0.953, alpha: 1)
        win.contentView = wv
        win.isReleasedWhenClosed = false
        win.delegate = self
        /* Closing the child is the pin-back signal. Target the window directly,
           so this remains reliable even when the accessory is not first responder. */
        let returnButton = NSButton(title: "Return to layout", target: win, action: #selector(NSWindow.performClose(_:)))
        returnButton.bezelStyle = .inline
        returnButton.controlSize = .small
        returnButton.font = NSFont.systemFont(ofSize: 12, weight: .medium)
        returnButton.contentTintColor = .secondaryLabelColor
        returnButton.toolTip = "Pin this panel back into the main Powermove layout"
        returnButton.translatesAutoresizingMaskIntoConstraints = false
        let accessoryView = NSView(frame: NSRect(x: 0, y: 0, width: 132, height: 28))
        accessoryView.addSubview(returnButton)
        NSLayoutConstraint.activate([
            returnButton.trailingAnchor.constraint(equalTo: accessoryView.trailingAnchor, constant: -10),
            returnButton.centerYAnchor.constraint(equalTo: accessoryView.centerYAnchor),
            returnButton.widthAnchor.constraint(equalToConstant: 116),
            returnButton.heightAnchor.constraint(equalToConstant: 24),
        ])
        let accessory = NSTitlebarAccessoryViewController()
        accessory.view = accessoryView
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
