//! Test-only inspection of the real validator and AccessKit projection.
use pm_design::{Accessibility, Scheme, Tokens};
use pm_project_panel_check::accessibility;
use pm_surface::{
    Surface,
    kit::{Layout, Panel, focus_order},
};
use serde_json::json;
use std::{
    io::{self, BufRead},
    sync::Arc,
};
struct SyntheticMetrics;
impl pm_panel_media::render::TextMetrics for SyntheticMetrics {
    fn width(&self, text: &str) -> f64 {
        text.chars().count() as f64 * 6.
    }
    fn boundaries(&self, text: &str) -> Vec<usize> {
        text.char_indices()
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect()
    }
}
impl pm_project_panel_check::render::Metrics for SyntheticMetrics {
    fn capsule_size(&self, text: &str) -> (f64, f64) {
        (text.chars().count() as f64 * 6., 18.)
    }
}
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
                let disabled = surface.nodes().values().any(|n| {
                    n.props.get("id").and_then(serde_json::Value::as_str)
                        == Some("media:transport-error")
                });
                let mut hovered = std::collections::HashSet::new();
                let mut unknown_loading = std::collections::HashSet::new();
                for n in surface.nodes().values() {
                    if let Some(id) =
                        n.props
                            .get("attr")
                            .and_then(|a| a.get("item"))
                            .and_then(|id| {
                                serde_json::from_value::<pm_panel_media::ItemId>(id.clone()).ok()
                            })
                    {
                        if n.props
                            .get("state")
                            .and_then(|s| s.get("hovered"))
                            .and_then(serde_json::Value::as_bool)
                            == Some(true)
                        {
                            hovered.insert(id.clone());
                        }
                        if n.props
                            .get("state")
                            .and_then(|s| s.get("loading"))
                            .and_then(serde_json::Value::as_bool)
                            == Some(true)
                            && let pm_panel_media::ItemId::Asset(id) = id
                        {
                            unknown_loading.insert(id);
                        }
                    }
                }
                let cards = serde_json::from_value::<pm_panel_media::v4::Projection>(
                    envelope["projection"].clone(),
                )
                .ok()
                .map(|view| {
                    pm_project_panel_check::render::prepare(
                        &view,
                        pm_project_panel_check::render::Grid {
                            width: 300.,
                            columns: 2,
                            row_height: 160.,
                        },
                        &Tokens::new(Scheme::Dark, Accessibility::default()),
                        &SyntheticMetrics,
                        disabled,
                        &unknown_loading,
                        &hovered,
                    )
                    .expect("synthetic media geometry")
                });
                let enriched = serde_json::from_value::<pm_panel_media::v4::Projection>(
                    envelope["projection"].clone(),
                )
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
                let ax=tree.nodes.iter().map(|(id,node)| json!({"id":id.0,"role":format!("{:?}",node.role()),"label":node.label(),"value":node.value(),"selected":node.is_selected(),"disabled":node.is_disabled(),"rowCount":node.row_count(),"multiselectable":node.is_multiselectable(),"actions":node.custom_actions().iter().map(|a| json!({"id":a.id,"label":a.description.as_ref()})).collect::<Vec<_>>()})).collect::<Vec<_>>();
                println!(
                    "{}",
                    json!({"accepted":true,"diagnostics":p.diagnostics,"wireDiagnostics":wire,"nodes":surface.nodes(),"focusOrder":focus_order(&surface),"accessibility":ax,"cards":cards})
                );
            }
            Err(error) => println!("{}", json!({"accepted":false,"error":error})),
        }
    }
}
