//! Test-only inspection of the real validator and AccessKit projection.
use pm_design::{Accessibility, Scheme, Tokens};
use pm_project_panel_check::{accessibility, bridge};
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
        let envelope: serde_json::Value =
            serde_json::from_str(&line.expect("stdin")).expect("envelope");
        match surface.apply_json(&envelope["commit"].to_string()) {
            Ok(wire) => {
                let p = Panel::prepare(
                    Arc::new(surface.clone()),
                    &Tokens::new(Scheme::Dark, Accessibility::default()),
                    &Layout::new(),
                    None,
                    false,
                );
                let enriched =
                    serde_json::from_value::<bridge::Projection>(envelope["projection"].clone())
                        .ok()
                        .map(|view| {
                            accessibility::ProjectAccessibility::prepare(
                                &surface,
                                &view,
                                &Layout::new(),
                                None,
                            )
                        });
                let tree = enriched
                    .as_ref()
                    .map(|ax| &ax.tree)
                    .unwrap_or(&p.accessibility);
                let ax=tree.nodes.iter().map(|(id,node)| json!({"id":id.0,"role":format!("{:?}",node.role()),"label":node.label(),"value":node.value(),"selected":node.is_selected(),"disabled":node.is_disabled(),"rowCount":node.row_count(),"multiselectable":node.is_multiselectable(),"actions":node.custom_actions().iter().map(|a| a.description.as_ref()).collect::<Vec<_>>()})).collect::<Vec<_>>();
                println!(
                    "{}",
                    json!({"accepted":true,"diagnostics":p.diagnostics,"wireDiagnostics":wire,"nodes":surface.nodes(),"focusOrder":focus_order(&surface),"accessibility":ax})
                );
            }
            Err(error) => println!("{}", json!({"accepted":false,"error":error})),
        }
    }
}
