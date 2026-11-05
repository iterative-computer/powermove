use pm_design::{Accessibility, Scheme, Tokens};
use pm_surface::{
    Surface,
    kit::{Layout, Panel},
};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead},
    sync::Arc,
};
fn main() {
    let mut surface = Surface::default();
    for line in io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.expect("stdin")).expect("request");
        let flags = request["appearance"].as_u64().unwrap_or(0);
        let tokens = Tokens::new(
            if flags & 1 == 0 {
                Scheme::Dark
            } else {
                Scheme::Light
            },
            Accessibility {
                increase_contrast: flags & 2 != 0,
                reduce_transparency: flags & 4 != 0,
                reduce_motion: flags & 8 != 0,
            },
        );
        match surface.apply_json(&request["commit"].to_string()) {
            Ok(diagnostics) => {
                let panel = Panel::prepare(
                    Arc::new(surface.clone()),
                    &tokens,
                    &Layout::new(),
                    request["focus"].as_u64(),
                    true,
                );
                let ax: Vec<_> = panel.accessibility.nodes.iter().map(|(id, node)| json!({
                    "id":id.0,"role":format!("{:?}",node.role()),"label":node.label(),"value":node.value(),
                    "numericValue":node.numeric_value(),"toggled":format!("{:?}",node.toggled())
                })).collect();
                println!(
                    "{}",
                    json!({"accepted":true,"nodes":surface.nodes(),"diagnostics":panel.diagnostics,
                    "wireDiagnostics":diagnostics,"ax":ax,"focusOrder":pm_surface::kit::focus_order(&surface)})
                );
            }
            Err(error) => println!("{}", json!({"accepted":false,"error":error})),
        }
    }
}
