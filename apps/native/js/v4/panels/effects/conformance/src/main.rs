//! Test-only JSON-lines probe. Uses the real wire, kit and AccessKit projection.
use accesskit::Action;
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
                let accessibility: Vec<_> = panel.accessibility.nodes.iter().map(|(id, node)| {
                    let actions: Vec<_> = [Action::Focus, Action::Click, Action::Expand, Action::Collapse, Action::SetValue]
                        .into_iter().filter(|action| node.supports_action(*action))
                        .map(|action| format!("{action:?}")).collect();
                    json!({"id": id.0, "role": format!("{:?}", node.role()), "label": node.label(),
                        "value": node.value(), "selected": node.is_selected(), "expanded": node.is_expanded(),
                        "disabled": node.is_disabled(), "actions": actions})
                }).collect();
                println!(
                    "{}",
                    json!({"accepted":true,"revision":panel.revision,
                    "nodes":surface.nodes(), "diagnostics":panel.diagnostics, "wireDiagnostics":wire_diagnostics,
                    "accessibility":accessibility, "focusOrder":focus_order(&surface)})
                );
            }
            Err(error) => println!(
                "{}",
                json!({"accepted":false,"revision":surface.revision(),"error":error})
            ),
        }
    }
}
