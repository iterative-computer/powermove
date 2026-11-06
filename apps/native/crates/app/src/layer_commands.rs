//! Menu commands over the selected layers: plain plans over a publication, so
//! they run and test without a window. The kernel validates at execution.
use crate::dialogs::PrecomposeDraft;
use pm_kernel::commands::comps::PrecomposeMode;
use pm_kernel::commands::layers::LayerPatch;
use pm_kernel::commands::text::{SplitMode, TextCommand};
use pm_kernel::commands::transform::TransformCommand;
use pm_kernel::text_host::{SharedTextHost, TextHost};
use pm_kernel::{Command, CompCommand, LayerCommand, Published, new_id};
use pm_model::Value;
use std::sync::Arc;

pub struct Plan {
    pub label: String,
    pub commands: Vec<Command>,
    pub select: Option<Vec<String>>,
    pub open: Option<String>,
}

fn layer(published: &Published, id: &str) -> Option<Value> {
    let root = published.snapshot.root()?;
    let loc = pm_kernel::model::find_layer(root, id)?;
    Some(pm_kernel::model::layer_at(root, &loc)?.to_value())
}
fn kind(layer: &Value) -> Option<&str> {
    layer.get("type").and_then(Value::as_str)
}
fn plain(label: &str, commands: Vec<Command>) -> Plan {
    Plan {
        label: label.into(),
        commands,
        select: None,
        open: None,
    }
}

pub fn plan(name: &str, published: &Published, time: f64) -> Option<Plan> {
    let selected = published.selection.layers().to_vec();
    if selected.is_empty() {
        return None;
    }
    let of_type = |wanted: &str| -> Vec<String> {
        selected
            .iter()
            .filter(|id| layer(published, id).is_some_and(|l| kind(&l) == Some(wanted)))
            .cloned()
            .collect()
    };
    match name {
        "releasePrecomp" => {
            let layer_id = of_type("precomp").into_iter().next()?;
            Some(Plan {
                select: Some(vec![layer_id.clone()]),
                ..plain(
                    "Release Pre-comp",
                    vec![CompCommand::ReleasePrecomp { layer_id }.into()],
                )
            })
        }
        "toggleContinuousRaster" => {
            let precomps = of_type("precomp");
            if precomps.is_empty() {
                return None;
            }
            let all_on = precomps.iter().all(|id| {
                layer(published, id)
                    .and_then(|l| l.get("continuousRaster").and_then(Value::as_bool))
                    == Some(true)
            });
            let commands = precomps
                .into_iter()
                .map(|layer_id| {
                    LayerCommand::Set {
                        layer_id,
                        patch: LayerPatch {
                            continuous_raster: Some(!all_on),
                            time,
                            ..Default::default()
                        },
                    }
                    .into()
                })
                .collect();
            Some(plain("Continuously Rasterize", commands))
        }
        "centerAnchor" => Some(plain(
            "Center Anchor Point",
            vec![
                TransformCommand::CenterAnchor {
                    layer_ids: selected,
                    time,
                }
                .into(),
            ],
        )),
        "splitTextWords" | "splitTextCharacters" | "splitTextLines" => {
            let layer_ids = of_type("text");
            if layer_ids.is_empty() {
                return None;
            }
            let group_ids: Vec<String> = layer_ids.iter().map(|_| new_id("L")).collect();
            let mode = match name {
                "splitTextWords" => SplitMode::Words,
                "splitTextCharacters" => SplitMode::Characters,
                _ => SplitMode::Lines,
            };
            Some(Plan {
                select: Some(group_ids.clone()),
                ..plain(
                    "Split Text",
                    vec![
                        TextCommand::Split {
                            layer_ids,
                            group_ids,
                            mode,
                            time,
                        }
                        .into(),
                    ],
                )
            })
        }
        _ => None,
    }
}

/// The sheet's starting state; Leave All Attributes needs one layer with a source.
pub fn precompose_draft(published: &Published) -> Option<PrecomposeDraft> {
    let selected = published.selection.layers();
    let first = layer(published, selected.first()?)?;
    let can_leave = selected.len() == 1
        && kind(&first).is_some_and(|k| {
            ["solid", "image", "video", "precomp", "shader", "extension"].contains(&k)
        });
    let root = published.snapshot.root()?.to_value();
    let taken: Vec<String> = root
        .get("comps")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|comps| comps.iter())
        .filter_map(|(_, comp)| comp.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let name = (1..)
        .map(|n| format!("Pre-comp {n}"))
        .find(|name| !taken.contains(name))?;
    Some(PrecomposeDraft {
        name,
        layer_name: first.get("name").and_then(Value::as_str).map(str::to_owned),
        can_leave,
        leave: false,
        adjust_duration: false,
        open_after: false,
    })
}

pub fn precompose_plan(published: &Published, draft: &PrecomposeDraft, time: f64) -> Plan {
    let leave = draft.leave && draft.can_leave;
    let layer_ids = published.selection.layers().to_vec();
    let precomp_layer_id = new_id("L");
    let new_comp_id = new_id("C");
    Plan {
        label: "Pre-compose".into(),
        select: Some(if leave {
            layer_ids.clone()
        } else {
            vec![precomp_layer_id.clone()]
        }),
        open: draft.open_after.then(|| new_comp_id.clone()),
        commands: vec![
            CompCommand::Precompose {
                layer_ids,
                new_comp_id,
                precomp_layer_id,
                name: Some(draft.name.clone()),
                mode: if leave {
                    PrecomposeMode::LeaveAttributes
                } else {
                    PrecomposeMode::MoveAll
                },
                adjust_duration: draft.adjust_duration && !leave,
                time,
            }
            .into(),
        ],
    }
}

/// The same font registry rendering asks, measured on the kernel's own thread.
struct RegistryText(pm_node::fonts::render::RenderFonts);
impl TextHost for RegistryText {
    fn measure(&self) -> Box<dyn pm_eval::text_geom::TextMeasure> {
        Box::new(pm_eval::text_measure::CompatMeasure::new(
            pm_node::fonts::render::BrokerResolver::new(
                self.0.broker.clone(),
                self.0.health.clone(),
            ),
        ))
    }
    fn take_failure(&self) -> Option<String> {
        self.0.health.take_failure()
    }
}
pub fn text_host(fonts: &pm_node::fonts::render::RenderFonts) -> SharedTextHost {
    SharedTextHost(Arc::new(RegistryText(fonts.sibling())))
}

#[cfg(test)]
#[path = "layer_commands_tests.rs"]
mod tests;
