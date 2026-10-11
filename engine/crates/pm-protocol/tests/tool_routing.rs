//! Contract 15: the host-services tool list mirrors the app's own lists, so
//! a tool added to the desktop bridge cannot silently route to a document
//! host on the hidden MCP host.

use std::collections::BTreeSet;
use std::path::Path;

use pm_protocol::document_host::{route_tool, ToolRoute, DOM_TOOLS, HOST_TOOLS, JOB_TOOLS};

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The quoted names in `export const <name> = [ ... ]`, following `...OTHER`
/// spreads of lists defined in the same file.
fn ts_list(source: &str, name: &str) -> BTreeSet<String> {
    let marker = format!("export const {name} = [");
    let start = source.find(&marker).unwrap_or_else(|| panic!("{name} not found")) + marker.len();
    let end = start + source[start..].find(']').expect("list ends");
    let body = &source[start..end];
    let mut names = BTreeSet::new();
    for part in body.split(',') {
        let part = part.trim();
        if let Some(spread) = part.strip_prefix("...") {
            names.extend(ts_list(source, spread.trim()));
        } else if let Some(quoted) = part.strip_prefix('\'').and_then(|p| p.strip_suffix('\'')) {
            names.insert(quoted.to_owned());
        } else if !part.is_empty() {
            panic!("unexpected entry {part:?} in {name}");
        }
    }
    names
}

#[test]
fn host_tools_mirror_the_desktop_bridge_lists() {
    let media = ts_list(&read("apps/desktop/src/shared/media-tools.ts"), "AGENT_MEDIA_TOOL_NAMES");
    let store = ts_list(&read("apps/desktop/src/main/agent-tools/spec.ts"), "POWERMOVE_STORE_TOOL_NAMES");
    let orchestration = ts_list(&read("apps/desktop/src/shared/agent-orchestration.ts"), "ORCHESTRATION_TOOL_NAMES");
    assert!(!media.is_empty() && !store.is_empty() && !orchestration.is_empty());
    let host: BTreeSet<&str> = HOST_TOOLS.iter().copied().collect();
    for tool in media.iter().chain(&store).chain(&orchestration) {
        assert!(host.contains(tool.as_str()), "{tool} runs in main on the desktop but is not in HOST_TOOLS");
        assert_eq!(route_tool(tool), ToolRoute::HostServices, "{tool}");
    }
    let listed: BTreeSet<&str> = media.iter().chain(&store).chain(&orchestration).map(String::as_str).collect();
    let extra: Vec<&&str> = host.iter().filter(|t| !listed.contains(**t)).collect();
    // The remaining host tools are dispatched by name in main/agent-tools/bridge.ts.
    let bridge = read("apps/desktop/src/main/agent-tools/bridge.ts");
    for tool in extra {
        assert!(
            bridge.contains(&format!("'{tool}'")),
            "{tool} is in HOST_TOOLS but neither in a desktop list nor handled by the bridge"
        );
    }
}

#[test]
fn dom_and_job_tools_exist_in_the_tool_spec() {
    let spec = read("apps/desktop/src/main/agent-tools/spec.ts");
    // `get_render_status` is new in Phase 3b (section 20) and has no spec yet.
    for tool in DOM_TOOLS.iter().chain(JOB_TOOLS.iter().filter(|t| **t != "get_render_status")) {
        assert!(spec.contains(&format!("'{tool}'")), "{tool} is routed but not in main/agent-tools/spec.ts");
    }
}
