//! Test-only Linux replay through the real six-op, token and AccessKit validators.
use pm_design::{Accessibility, Scheme, Tokens};
use pm_surface::{
    Surface,
    kit::{Layout, Panel, focus_order},
};
use serde_json::json;
use std::{
    io::{self, BufRead},
    sync::Arc,
};
fn main() {
    let mut surface = Surface::default();
    for line in io::stdin().lock().lines() {
        match surface.apply_json(&line.expect("stdin")) {
            Ok(wire) => {
                let appearances = [Scheme::Light, Scheme::Dark].map(|scheme| {
                    let panel = Panel::prepare(Arc::new(surface.clone()), &Tokens::new(scheme, Accessibility::default()), &Layout::new(), None, false);
                    let ax = panel.accessibility.nodes.iter().map(|(id, node)| json!({
                        "id": id.0, "role": format!("{:?}", node.role()),
                        "label": node.label(), "value": node.value(), "disabled": node.is_disabled(),
                    })).collect::<Vec<_>>();
                    json!({ "diagnostics": panel.diagnostics, "accessibility": ax })
                });
                println!(
                    "{}",
                    json!({"accepted":true, "revision":surface.revision(), "wireDiagnostics":wire,
                    "nodes":surface.nodes(), "focusOrder":focus_order(&surface), "appearances":appearances})
                );
            }
            Err(error) => println!(
                "{}",
                json!({"accepted":false,"error":error,"revision":surface.revision()})
            ),
        }
    }
}
