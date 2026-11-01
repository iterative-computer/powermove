//! Synthetic headless UI kit validator, used by the JSX wire tests.
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
        match surface.apply_json(&line.expect("stdin")) {
            Ok(wire_diagnostics) => {
                let panel = Panel::prepare(
                    Arc::new(surface.clone()),
                    &tokens,
                    &Layout::new(),
                    None,
                    false,
                );
                println!(
                    "{}",
                    json!({"accepted": true, "revision": panel.revision,
                    "diagnostics": panel.diagnostics, "wireDiagnostics": wire_diagnostics,
                    "components": panel.components.keys().collect::<Vec<_>>(),
                    "accessibilityNodes": panel.accessibility.nodes.len(),
                    "focusOrder": pm_surface::kit::focus_order(&surface), "nodes": surface.nodes()})
                );
            }
            Err(error) => println!(
                "{}",
                json!({"accepted": false, "error": error, "revision": surface.revision()})
            ),
        }
    }
}
