//! Settings-only test executable: replay real six-op commits and inspect AccessKit.
//! Compiled by the Bun tests against the locked shared validator build artifacts.
use pm_design::{Accessibility, Scheme, Tokens};
use pm_surface::{
    Surface,
    kit::{Layout, Panel},
};
use serde_json::json;
use std::{
    io::{self, BufRead},
    sync::Arc,
};

fn main() {
    let mut surface = Surface::default();
    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    for line in io::stdin().lock().lines() {
        let wire = surface
            .apply_json(&line.expect("test input"))
            .expect("valid test commit");
        let panel = Panel::prepare(
            Arc::new(surface.clone()),
            &tokens,
            &Layout::new(),
            None,
            false,
        );
        let ax: Vec<_> = panel.accessibility.nodes.iter().map(|(id, node)| json!({
            "id": id.0, "role": format!("{:?}", node.role()), "label": node.label(),
            "value": node.value(), "help": node.description(), "selected": node.is_selected(),
            "disabled": node.is_disabled(), "rowCount": node.row_count(),
            "children": node.children().iter().map(|child| child.0).collect::<Vec<_>>()
        })).collect();
        println!(
            "{}",
            json!({"wireDiagnostics": wire, "diagnostics": panel.diagnostics,
            "ax": ax, "focusOrder": pm_surface::kit::focus_order(&surface)})
        );
    }
}
