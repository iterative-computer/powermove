//! Timeline-only test validator: the real surface, kit recipes and AccessKit tree.
//! No renderer or macOS screenshot acceptance is claimed.
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
    let args = std::env::args().collect::<Vec<_>>();
    let enabled = |flag| args.iter().any(|arg| arg == flag);
    let tokens = Tokens::new(
        if enabled("--light") {
            Scheme::Light
        } else {
            Scheme::Dark
        },
        Accessibility {
            increase_contrast: enabled("--contrast"),
            reduce_transparency: enabled("--transparency"),
            reduce_motion: enabled("--motion"),
        },
    );
    let mut surface = Surface::default();
    for line in io::stdin().lock().lines() {
        match surface.apply_json(&line.expect("fixture stdin")) {
            Ok(wire) => {
                let panel = Panel::prepare(
                    Arc::new(surface.clone()),
                    &tokens,
                    &Layout::new(),
                    None,
                    false,
                );
                let accessibility = panel.accessibility.nodes.iter().map(|(id, node)| json!({
                    "id":id.0,"role":format!("{:?}", node.role()),"label":node.label(),"value":node.value(),
                    "selected":node.is_selected(),"disabled":node.is_disabled(),"expanded":node.is_expanded(),
                })).collect::<Vec<_>>();
                println!(
                    "{}",
                    json!({"accepted":true,"revision":panel.revision,"diagnostics":panel.diagnostics,
                    "wireDiagnostics":wire,"accessibilityNodes":accessibility.len(),"accessibility":accessibility,
                    "focusOrder":pm_surface::kit::focus_order(&surface),"nodes":surface.nodes()})
                );
            }
            Err(error) => println!(
                "{}",
                json!({"accepted":false,"error":error,"revision":surface.revision()})
            ),
        }
    }
}
