//! Linux-only validation/AX exporter for the owned panel's golden tests.
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
                let ax = panel.accessibility.nodes.iter().map(|(id, node)| json!({
                    "id": id.0, "role": format!("{:?}", node.role()), "label": node.label(), "disabled": node.is_disabled(),
                    "selected": node.is_selected(), "value": node.value(), "numericValue": node.numeric_value(),
                    "toggled": format!("{:?}", node.toggled()), "expanded": node.is_expanded(),
                    "children": node.children().iter().map(|id| id.0).collect::<Vec<_>>()
                })).collect::<Vec<_>>();
                println!(
                    "{}",
                    pm_surface::inspection_json(
                        json!({"accepted":true,"wireDiagnostics":wire_diagnostics,"diagnostics":panel.diagnostics,
                    "nodes":surface.nodes(),"focusOrder":pm_surface::kit::focus_order(&surface),"accessibilityNodes":ax.len(),"accessibility":ax})
                    )
                );
            }
            Err(error) => println!("{}", json!({"accepted":false,"error":error})),
        }
    }
}
