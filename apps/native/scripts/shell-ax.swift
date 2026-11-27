// Inspect only the disposable native-shell test process supplied by PID.
// Skips the system menu bar and its potentially private Recent Items.
// swiftc shell-ax.swift -o /tmp/pm-a11y/shell-ax
import AppKit
import ApplicationServices

let args = CommandLine.arguments
guard args.count >= 2, let pid = Int32(args[1]) else {
    fputs("usage: shell-ax PID [--main|--sheet|--palette] [--expect-focus LABEL]\n", stderr); exit(2)
}

let app = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(app, 3)
var failures: [String] = []
func require(_ condition: Bool, _ message: String) {
    print("\(condition ? "PASS" : "FAIL") \(message)")
    if !condition { failures.append(message) }
}
func attribute(_ element: AXUIElement, _ key: String) -> (CFTypeRef?, AXError) {
    var value: CFTypeRef?
    let error = AXUIElementCopyAttributeValue(element, key as CFString, &value)
    return (value, error)
}
func text(_ element: AXUIElement, _ key: String) -> String {
    let (value, _) = attribute(element, key)
    return value as? String ?? ""
}
func elements(_ element: AXUIElement, _ key: String) -> [AXUIElement] {
    attribute(element, key).0 as? [AXUIElement] ?? []
}
func frame(_ element: AXUIElement) -> CGRect? {
    // AXFrame is not required by macOS' AXUIElement API. AppKit exports
    // accessibilityFrame as AXPosition + AXSize, so check both forms.
    if let value = attribute(element, "AXFrame").0, CFGetTypeID(value) == AXValueGetTypeID() {
        var rect = CGRect.zero
        if AXValueGetValue(value as! AXValue, .cgRect, &rect) { return rect }
    }
    guard let p = attribute(element, "AXPosition").0, CFGetTypeID(p) == AXValueGetTypeID(),
          let s = attribute(element, "AXSize").0, CFGetTypeID(s) == AXValueGetTypeID() else { return nil }
    var point = CGPoint.zero; var size = CGSize.zero
    guard AXValueGetValue(p as! AXValue, .cgPoint, &point), AXValueGetValue(s as! AXValue, .cgSize, &size) else { return nil }
    return CGRect(origin: point, size: size)
}
struct Node {
    let element: AXUIElement
    let depth: Int
    let parent: Int?
    var id: String { text(element, "AXIdentifier") }
    var role: String { text(element, "AXRole") }
}
func tree() -> [Node] {
    var nodes: [Node] = []
    func walk(_ element: AXUIElement, _ depth: Int, _ parent: Int?, _ ancestors: [AXUIElement]) {
        guard depth <= 24, nodes.count < 1200, !ancestors.contains(where: { CFEqual($0, element) }) else { return }
        // The test owns synthetic window content only. Exclude the system
        // menu bar, whose Recent Items can contain unrelated private names.
        if text(element, "AXRole") == "AXMenuBar" { return }
        let index = nodes.count
        nodes.append(Node(element: element, depth: depth, parent: parent))
        for child in elements(element, "AXChildren") { walk(child, depth + 1, index, ancestors + [element]) }
    }
    walk(app, 0, nil, [])
    return nodes
}
func descendants(_ node: Node, in nodes: [Node]) -> [Node] {
    guard let index = nodes.firstIndex(where: { CFEqual($0.element, node.element) }) else { return [] }
    return Array(nodes.dropFirst(index + 1).prefix(while: { $0.depth > node.depth }))
}
func dump(_ nodes: [Node]) {
    for node in nodes {
        var row: [String: Any] = ["depth": node.depth]
        for key in ["AXRole", "AXTitle", "AXDescription", "AXIdentifier", "AXValue", "AXFocused", "AXSelected"] {
            let (value, error) = attribute(node.element, key)
            row[key] = value.map { String(describing: $0) } ?? "<AXError \(error.rawValue)>"
        }
        if let rect = frame(node.element) { row["frame"] = [rect.minX, rect.minY, rect.width, rect.height] }
        if let data = try? JSONSerialization.data(withJSONObject: row, options: [.sortedKeys]), let json = String(data: data, encoding: .utf8) { print(json) }
    }
}
guard AXIsProcessTrusted() else {
    fputs("BLOCKED: Accessibility permission missing; no prompt requested.\n", stderr); exit(2)
}
// The first external AX query activates AccessKit asynchronously.
var nodes = tree()
if !nodes.contains(where: { $0.id == "shell.editor" }) && !args.contains("--sheet") && !args.contains("--palette") {
    Thread.sleep(forTimeInterval: 0.2)
    nodes = tree()
}
dump(nodes)
func label(_ n: Node) -> String {
    let description = text(n.element, "AXDescription")
    return description.isEmpty ? text(n.element, "AXTitle") : description
}
func has(_ role: String, _ name: String) -> Bool {
    nodes.contains { $0.role == role && label($0) == name }
}
if args.contains("--main") {
    let panels = nodes.filter { $0.id.hasPrefix("shell.panel.") }
    require(panels.map { $0.id } == ["shell.panel.assets", "shell.panel.viewer", "shell.panel.inspector", "shell.panel.timeline", "shell.panel.fxbrowser"], "default panel reading order")
    for name in ["Project", "Viewer", "Properties", "Timeline", "Effects & Presets"] { require(has("AXGroup", name), "panel group: \(name)") }
    for name in ["Sidebar", "Agent", "Export", "Inspector"] { require(has("AXButton", name), "toolbar button: \(name)") }
    require(has("AXGroup", "Tools") || nodes.contains { $0.role == "AXRadioGroup" }, "toolbar tool group")
    require(nodes.contains { $0.id == "shell.status" && $0.role == "AXGroup" && label($0) == "Status" && text($0.element, "AXValue") == "No project" }, "status role/label/value")
    require(has("AXButton", "Workspace: Default"), "workspace value")
    require(nodes.contains { $0.id == "shell.tab.assets" && $0.role == "AXRadioButton" }, "panel tab role")
}
if args.contains("--sheet") {
    let sheet = nodes.first { $0.role == "AXSheet" }
    require(sheet != nil, "Save Workspace sheet role")
    if let sheet = sheet {
        let controls = descendants(sheet, in: nodes).filter { ["AXTextField", "AXButton"].contains($0.role) }
        require(controls.map(label) == ["Workspace Name", "Cancel", "Save"], "sheet roles/labels/order")
    }
}
if args.contains("--palette") {
    require(has("AXTextField", "Search"), "palette search field")
    let rows = nodes.filter { $0.id.hasPrefix("shell.palette.") }
    require(!rows.isEmpty && rows.allSatisfy { $0.role == "AXButton" && !label($0).isEmpty }, "palette row roles and labels")
    require(rows.filter { text($0.element, "AXValue") == "Selected" }.count == 1, "one selected palette value")
}
if let value = attribute(app, "AXFocusedUIElement").0 {
    let element = value as! AXUIElement
    let node = Node(element: element, depth: 0, parent: nil)
    print("FOCUSED \(node.role) | \(label(node)) | \(node.id)")
    if let i = args.firstIndex(of: "--expect-focus"), i + 1 < args.count {
        require(label(node) == args[i + 1] || node.id == args[i + 1], "keyboard focus: \(args[i + 1])")
    }
} else if args.contains("--expect-focus") { require(false, "no AX focused element") }
exit(failures.isEmpty ? 0 : 1)
