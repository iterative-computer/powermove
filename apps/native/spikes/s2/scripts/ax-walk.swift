// Synthetic S2 fixture only. No screenshots, input events, activation or unlock.
// Build: swiftc ax-walk.swift -o "$CARGO_TARGET_DIR/s2b/ax-walk"
// Run: ax-walk PID [--exercise] (the latter also checks focus/value/AXPress).
import AppKit
import ApplicationServices

let args = CommandLine.arguments
 guard args.count >= 2, let pid = Int32(args[1]) else {
    fputs("usage: ax-walk PID [--exercise]\n", stderr); exit(2)
}
let exercise = args.contains("--exercise")
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
        for key in ["AXRole", "AXTitle", "AXDescription", "AXIdentifier", "AXValue", "AXFrame", "AXPosition", "AXSize"] {
            let (value, error) = attribute(node.element, key)
            row[key] = value.map { String(describing: $0) } ?? "<AXError \(error.rawValue)>"
        }
        if let rect = frame(node.element) { row["frame"] = [rect.minX, rect.minY, rect.width, rect.height] }
        if let data = try? JSONSerialization.data(withJSONObject: row, options: [.sortedKeys]), let json = String(data: data, encoding: .utf8) { print(json) }
    }
}
print("AX_TRUSTED \(AXIsProcessTrusted()) PID \(pid) exercise=\(exercise)")
guard AXIsProcessTrusted() else {
    print("BLOCKED: AX client lacks Accessibility permission; no permission prompt or unlock requested."); exit(2)
}
let expected = ["s2-navigation", "s2-viewer", "s2-field", "s2-web", "s2-transport", "s2-timeline"]
var nodes: [Node] = []
// Allow asynchronous WebKit AX initialization; no main-window activation.
let deadline = Date().addingTimeInterval(12)
repeat {
    nodes = tree()
    if nodes.contains(where: { $0.role == "AXWebArea" }) && expected.allSatisfy({ id in nodes.contains(where: { $0.id == id }) }) { break }
    RunLoop.current.run(until: Date().addingTimeInterval(0.25))
} while Date() < deadline
 dump(nodes)
let (children, childError) = attribute(app, "AXChildren")
require(childError == .success && children != nil, "application AXChildren query (AXError \(childError.rawValue))")
// Matches Frame.accessibility: navigation, below viewer, native field, compat,
// native transport, remaining GPUI (timeline). This is reading order, not z-order.

let markers = nodes.filter { expected.contains($0.id) }
require(markers.map { $0.id } == expected, "published interleaved reading order: \(markers.map { $0.id })")
if let parent = markers.first?.parent {
    require(markers.count == expected.count && markers.allSatisfy { $0.parent == parent }, "GPUI and slot entries share the AccessKit root")
    let direct = elements(nodes[parent].element, "AXChildren")
    let navigation = elements(nodes[parent].element, "AXChildrenInNavigationOrder")
    // AXChildrenInNavigationOrder isn't exposed by all OS AX bridges. The
    // in-process adapter test checks the selector even when AXError=-25205.
    if !navigation.isEmpty {
        require(direct.count == navigation.count && zip(direct, navigation).allSatisfy { CFEqual($0.0, $0.1) }, "AX navigation order agrees with children")
    } else { print("UNVERIFIED external AXChildrenInNavigationOrder: AXError \(attribute(nodes[parent].element, "AXChildrenInNavigationOrder").1.rawValue)") }
    for marker in markers {
        let parents = attribute(marker.element, "AXParent").0
        require(parents.map { CFEqual($0, nodes[parent].element) } ?? false, "\(marker.id) AXParent matches root")
        let rect = frame(marker.element)
        require(rect.map { $0.width > 0 && $0.height > 0 && $0.minX.isFinite && $0.minY.isFinite } ?? false, "\(marker.id) has a finite, positive screen frame")
    }
}
let web = markers.first { $0.id == "s2-web" }
let webNodes = web.map { descendants($0, in: nodes) } ?? []
let area = webNodes.first { $0.role == "AXWebArea" }
require(area != nil, "native WKWebView exposes AXWebArea")
let input = area.flatMap { area in descendants(area, in: nodes).first { $0.role == "AXTextField" } }
require(input != nil, "WKWebView AXWebArea exposes its inner text field")
if let input = input { require(frame(input.element).map { $0.width > 0 && $0.height > 0 } ?? false, "web input frame is positive") }
let transport = markers.first { $0.id == "s2-transport" }
let controls = transport.map { descendants($0, in: nodes).filter { $0.role == "AXButton" } } ?? []
require(controls.count == 3, "native glass transport exposes three buttons")
if exercise {
    func settle() { RunLoop.current.run(until: Date().addingTimeInterval(0.35)) }
    func focus(_ node: Node, _ name: String) {
        let error = AXUIElementSetAttributeValue(node.element, "AXFocused" as CFString, kCFBooleanTrue)
        settle()
        let focused = attribute(node.element, "AXFocused").0 as? Bool ?? false
        let reported = attribute(app, "AXFocusedUIElement").0
        let matches = reported.map { CFEqual($0, node.element) } ?? false
        if let reported = reported, CFGetTypeID(reported) == AXUIElementGetTypeID() {
            let element = reported as! AXUIElement
            print("FOCUS_REPORT role=\(text(element, "AXRole")) id=\(text(element, "AXIdentifier")) title=\(text(element, "AXTitle"))")
        }
        require(error == .success && focused && matches, "focus \(name): set=\(error.rawValue), AXFocused=\(focused), application focus matches=\(matches)")
    }
    if let gpui = markers.first(where: { $0.id == "s2-navigation" }), let input = input, let field = markers.first(where: { $0.id == "s2-field" }) {
        focus(gpui, "GPUI before slot")
        focus(input, "WebKit inner text field")
        focus(gpui, "GPUI after WebKit")
        focus(field, "native text field")
        focus(gpui, "GPUI after slots")
        let viewer = markers.first { $0.id == "s2-viewer" }
        let hitNodes = [input, field, gpui] + [viewer, controls.first].compactMap { $0 }
        for node in hitNodes {
            if let rect = frame(node.element) {
                var hit: AXUIElement?
                let error = AXUIElementCopyElementAtPosition(app, Float(rect.midX), Float(rect.midY), &hit)
                if let hit = hit { print("HIT_REPORT role=\(text(hit, "AXRole")) id=\(text(hit, "AXIdentifier")) title=\(text(hit, "AXTitle"))") }
                var inSubtree = false
                var ancestor = hit
                for _ in 0..<24 {
                    guard let current = ancestor else { break }
                    if CFEqual(current, node.element) { inSubtree = true; break }
                    let parent = attribute(current, "AXParent").0
                    ancestor = parent.flatMap { CFGetTypeID($0) == AXUIElementGetTypeID() ? ($0 as! AXUIElement) : nil }
                }
                require(error == .success && inSubtree, "hit test \(node.role) at its screen frame center (\(error.rawValue))")
            }
        }
        for node in [input, field] {
            let old = attribute(node.element, "AXValue").0 ?? "" as CFString
            let error = AXUIElementSetAttributeValue(node.element, "AXValue" as CFString, "S2b AX text probe" as CFString)
            settle()
            require(error == .success && text(node.element, "AXValue") == "S2b AX text probe", "\(node.role) native AXValue action (\(error.rawValue))")
            let restore = AXUIElementSetAttributeValue(node.element, "AXValue" as CFString, old)
            require(restore == .success, "restore synthetic text value")
        }
    }
    if let label = webNodes.first(where: { $0.role == "AXStaticText" && text($0.element, "AXValue") == "Compat overlay hole" }), let rect = frame(label.element) {
        var hit: AXUIElement?
        let error = AXUIElementCopyElementAtPosition(app, Float(rect.minX + 1), Float(rect.midY), &hit)
        require(error == .success && hit.map { CFEqual($0, label.element) } ?? false, "compat overlay label's leading edge is inside the native hit clip (\(error.rawValue))")
    } else { require(false, "compat overlay exposes its complete label") }
    if let button = webNodes.first(where: { $0.role == "AXButton" && text($0.element, "AXTitle") == "Click inside WebKit" }) {
        let error = AXUIElementPerformAction(button.element, "AXPress" as CFString); settle()
        require(error == .success && text(button.element, "AXTitle") == "Clicked", "WebKit AXPress invokes the DOM button (\(error.rawValue))")
    } else { require(false, "find WebKit overlay action") }
    if let control = controls.first(where: { text($0.element, "AXDescription") == "Play" || text($0.element, "AXTitle") == "Play" }) {
        let error = AXUIElementPerformAction(control.element, "AXPress" as CFString)
        require(error == .success, "glass transport native AXPress (\(error.rawValue))")
    } else { require(false, "find native transport action") }
} else { print("UNVERIFIED focus/value/press: rerun with --exercise; VoiceOver traversal requires a separate attended check.") }
print("S2b AX RESULT \(failures.isEmpty ? "PASS" : "FAIL") failures=\(failures.count)")
exit(failures.isEmpty ? 0 : 1)
