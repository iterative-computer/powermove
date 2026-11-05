//! Semantic actions admitted by the native hit tester/fields. The production
//! built-in worker and headless drivers use the same kernel-backed entrypoint.
//! Physical coordinates and menu/field presentation stay with the native host.
use crate::{Draft, DragKind, DragSample, HostEffect, Plan, TimelineClient, v4};
use pm_kernel::commands::{
    keyframes::{EaseSide, EaseSpec, Interpolation, SimplifyScope},
    layers::LayerPatch,
    pro_edit::{Edge, ProEditCommand, TrackTarget},
};
use pm_kernel::{Command, KeyframeCommand, LayerCommand, SelectionCall};
use serde_json::Value;

type Result<T> = std::result::Result<T, v4::Error>;
fn invalid(field: &'static str) -> v4::Error {
    v4::Error::InvalidIntent(field)
}
fn text<'a>(v: &'a Value, field: &'static str) -> Result<&'a str> {
    v[field]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid(field))
}
fn number(v: &Value, field: &'static str) -> Result<f64> {
    v[field]
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid(field))
}
fn optional_number(v: &Value, field: &'static str, default: f64) -> Result<f64> {
    if v.get(field).is_none() {
        Ok(default)
    } else {
        number(v, field)
    }
}
fn flag(v: &Value, field: &'static str) -> Result<bool> {
    match v.get(field) {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(invalid(field)),
    }
}
fn ids(v: &Value, field: &'static str) -> Result<Vec<String>> {
    let list = v[field]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| invalid(field))?;
    let mut result = Vec::new();
    for id in list {
        let id = id
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid(field))?;
        if result.iter().any(|s| s == id) {
            return Err(invalid(field));
        }
        result.push(id.to_owned());
    }
    Ok(result)
}
fn side(v: &Value) -> Result<EaseSide> {
    match text(v, "side")? {
        "in" => Ok(EaseSide::In),
        "out" => Ok(EaseSide::Out),
        "both" => Ok(EaseSide::Both),
        _ => Err(invalid("side")),
    }
}
fn edge(v: &Value) -> Result<Edge> {
    match text(v, "edge")? {
        "in" => Ok(Edge::In),
        "out" => Ok(Edge::Out),
        _ => Err(invalid("edge")),
    }
}
fn tracks(v: &Value) -> Result<Vec<TrackTarget>> {
    v["tracks"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| invalid("tracks"))?
        .iter()
        .map(|t| {
            let track = t["track"]
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or_else(|| invalid("track"))?;
            let group_id = match &t["group"] {
                Value::Null => None,
                Value::String(s) if !s.is_empty() => Some(s.clone()),
                _ => return Err(invalid("group")),
            };
            Ok(TrackTarget { track, group_id })
        })
        .collect()
}
fn pro_edit(v: &Value) -> Result<ProEditCommand> {
    let linked = flag(v, "linked")?;
    Ok(match text(v, "operation")? {
        "move" => ProEditCommand::Move {
            layer_ids: ids(v, "ids")?,
            delta: number(v, "delta")?,
            linked,
        },
        "trim" => ProEditCommand::Trim {
            layer_ids: ids(v, "ids")?,
            edge: edge(v)?,
            delta: number(v, "delta")?,
            linked,
        },
        "slip" => ProEditCommand::Slip {
            layer_ids: ids(v, "ids")?,
            delta: number(v, "delta")?,
            linked,
        },
        "rippleDelete" => ProEditCommand::RippleDelete {
            layer_ids: ids(v, "ids")?,
            tracks: tracks(v)?,
            linked,
        },
        "rippleTrim" => ProEditCommand::RippleTrim {
            layer_id: text(v, "layer")?.into(),
            edge: edge(v)?,
            delta: number(v, "delta")?,
            tracks: tracks(v)?,
            linked,
        },
        "roll" => ProEditCommand::Roll {
            left_id: text(v, "left")?.into(),
            right_id: text(v, "right")?.into(),
            delta: number(v, "delta")?,
            linked,
        },
        "slide" => ProEditCommand::Slide {
            previous_id: text(v, "previous")?.into(),
            layer_id: text(v, "layer")?.into(),
            next_id: text(v, "next")?.into(),
            delta: number(v, "delta")?,
            linked,
        },
        _ => return Err(invalid("operation")),
    })
}
fn drag_kind(c: &TimelineClient, v: &Value, time: f64) -> Result<DragKind> {
    Ok(match text(v, "kind")? {
        "proEdit" => DragKind::ProEdit(pro_edit(v)?),
        "layers" => DragKind::Layers {
            ids: ids(v, "ids")?,
        },
        "trimIn" => DragKind::TrimIn {
            ids: ids(v, "ids")?,
        },
        "trimOut" => DragKind::TrimOut {
            ids: ids(v, "ids")?,
        },
        "offsetLayers" => DragKind::QuickOffsetLayers {
            ids: ids(v, "ids")?,
        },
        "keys" => DragKind::Keys {
            ids: c.model.expand_scale_keys(&ids(v, "ids")?),
        },
        "graphKeys" => DragKind::GraphKeys {
            ids: c.model.expand_scale_keys(&ids(v, "ids")?),
        },
        "stretchKeys" => DragKind::StretchKeys {
            ids: c.model.expand_scale_keys(&ids(v, "ids")?),
            anchor_time: number(v, "anchorTime")?,
            grabbed_time: number(v, "grabbedTime")?,
        },
        "transformKeys" => DragKind::TransformKeys {
            ids: c.model.expand_scale_keys(&ids(v, "ids")?),
            anchor_time: number(v, "anchorTime")?,
            anchor_value: number(v, "anchorValue")?,
        },
        "handle" => DragKind::Handle {
            id: text(v, "id")?.into(),
            side: side(v)?,
        },
        "property" => {
            let layer = text(v, "layer")?;
            let path = text(v, "path")?;
            let paths = c
                .model
                .track_channels(layer, path)
                .iter()
                .map(|c| c.path.clone())
                .collect::<Vec<_>>();
            if paths.is_empty() {
                return Err(invalid("path"));
            }
            let targets = c.model.property_targets(layer, &paths)?;
            if targets.is_empty() {
                return Err(invalid("targets"));
            }
            DragKind::Property { targets, time }
        }
        "workIn" => DragKind::WorkIn,
        "workOut" => DragKind::WorkOut,
        "workMove" => DragKind::WorkMove,
        _ => return Err(invalid("kind")),
    })
}

pub(super) async fn dispatch(
    c: &mut TimelineClient,
    v: &Value,
    context: &v4::Context<'_>,
) -> Result<Option<Vec<HostEffect>>> {
    let action = text(v, "type")?;
    // Optional stamps are decimal strings, as in the native frame. Never round
    // u64 document identity through a JS Number. Existing v4 controls use Context.
    for (field, expected) in [
        ("document", context.expected_document),
        ("revision", context.expected_revision),
    ] {
        if let Some(value) = v.get(field) {
            let actual = value
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| invalid(field))?;
            if actual != expected {
                return Err(invalid("stale native surface"));
            }
        }
    }
    match action {
        "beginDrag" => {
            let kind = drag_kind(c, v, context.time)?;
            c.begin_drag(kind).await?;
            return Ok(Some(vec![]));
        }
        "drag" => {
            let m = &v["modifiers"];
            let sample = DragSample {
                time_delta: optional_number(v, "timeDelta", 0.)?,
                value_delta: optional_number(v, "valueDelta", 0.)?,
                time_scale: optional_number(v, "timeScale", 1.)?,
                value_scale: optional_number(v, "valueScale", 1.)?,
                handle_time: optional_number(v, "handleTime", 0.)?,
                handle_value: optional_number(v, "handleValue", 0.)?,
                modifiers: crate::Modifiers {
                    shift: flag(m, "shiftKey")?,
                    alt: flag(m, "altKey")?,
                    meta: flag(m, "metaKey")?,
                    control: flag(m, "ctrlKey")?,
                },
                acquire_tolerance: optional_number(v, "acquireTolerance", 0.)?,
                release_tolerance: optional_number(v, "releaseTolerance", 0.)?,
                playhead: context.time,
            };
            if sample.acquire_tolerance < 0. || sample.release_tolerance < 0. {
                return Err(invalid("snap tolerance"));
            }
            c.drag(sample).await?;
            return Ok(Some(vec![]));
        }
        "endDrag" | "cancelDrag" | "dispose" => {
            if action != "dispose" || c.active_drag().is_some() {
                c.finish_drag(action == "endDrag").await?;
            }
            return Ok(Some(vec![]));
        }
        _ => {}
    }
    let edit = |commands: Vec<Command>| Plan::edit(commands, c.model.index.revision);
    let mut reveal_dimensions = None;
    let persist_view = action == "command"
        && v["command"].as_str().is_some_and(|id| {
            (id.starts_with("timeline.reveal")
                || id.starts_with("timeline.mode:")
                || matches!(id, "timeline.toggleMode" | "toggleLayerStrips"))
                || matches!(
                    id,
                    "graph" | "graph.value" | "graph.speed" | "graph.combined"
                )
        });
    let plan = match action {
        "selectKeys" => {
            let calls = c.model.select_keys(
                text(v, "layer")?,
                text(v, "path")?,
                &ids(v, "ids")?,
                flag(v, "additive")?,
                flag(v, "dragging")?,
            )?;
            Plan {
                calls,
                ..Default::default()
            }
        }
        "selectProperty" => {
            let layer = text(v, "layer")?;
            let path = text(v, "path")?;
            if c.model.track_channels(layer, path).is_empty() {
                return Err(invalid("path"));
            }
            c.model.graph_focus = Some((layer.into(), path.into()));
            Plan {
                calls: vec![
                    SelectionCall::Layers {
                        ids: vec![layer.into()],
                        add: false,
                    }
                    .into(),
                    SelectionCall::Channel(Some(path.into())).into(),
                ],
                ..Default::default()
            }
        }
        "parent" => {
            let parent = match &v["parent"] {
                Value::Null => None,
                Value::String(s) if !s.is_empty() => Some(s.clone()),
                _ => return Err(invalid("parent")),
            };
            c.model.parent(&ids(v, "ids")?, parent, context.time)
        }
        "separateDimensions" => {
            let layer = text(v, "layer")?;
            let separated = v["separate"].as_bool().ok_or_else(|| invalid("separate"))?;
            let paths = c
                .model
                .track_channels(layer, "position")
                .iter()
                .map(|c| c.path.clone())
                .collect::<Vec<_>>();
            if paths.len() < 2 {
                return Err(invalid("position"));
            }
            let targets = c
                .model
                .property_targets(layer, &paths)?
                .into_iter()
                .map(|t| t.0)
                .collect::<std::collections::BTreeSet<_>>();
            if targets.is_empty() {
                return Err(invalid("targets"));
            }
            reveal_dimensions = Some(targets.clone());
            let commands = targets
                .into_iter()
                .map(|layer_id| {
                    LayerCommand::Set {
                        layer_id,
                        patch: LayerPatch {
                            separate_dimensions: Some(separated),
                            ..Default::default()
                        },
                    }
                    .into()
                })
                .collect();
            let mut plan = edit(commands);
            if let Some(pm_kernel::Call::Transact { label, .. }) = plan.calls.first_mut() {
                *label = Some(
                    if separated {
                        "Separate Dimensions"
                    } else {
                        "Join Dimensions"
                    }
                    .into(),
                );
            }
            plan
        }
        "editKeys" => {
            let keys = ids(v, "ids")?;
            let time = if v.get("time").is_some() {
                if keys.len() != 1 {
                    return Err(invalid("single key time"));
                }
                Some(number(v, "time")?)
            } else {
                None
            };
            let value = v
                .get("value")
                .map(|v| {
                    v.as_f64()
                        .filter(|n| n.is_finite())
                        .map(pm_model::Value::Number)
                        .ok_or_else(|| invalid("value"))
                })
                .transpose()?;
            if time.is_none() && value.is_none() {
                return Err(invalid("key edit"));
            }
            Draft::Keys {
                ids: keys,
                time,
                value,
            }
            .finish(true, c.model.index.revision)
        }
        "easing" => {
            let curve = if let Some(curve) = v.get("curve") {
                let points = curve
                    .as_array()
                    .filter(|a| a.len() == 4)
                    .ok_or_else(|| invalid("curve"))?;
                let mut result = [0.; 4];
                for (n, p) in result.iter_mut().zip(points) {
                    *n = p
                        .as_f64()
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| invalid("curve"))?;
                }
                EaseSpec::Curve(result)
            } else {
                EaseSpec::Preset(text(v, "preset")?.into())
            };
            edit(vec![
                KeyframeCommand::SetEasing {
                    key_ids: c.model.expand_scale_keys(&ids(v, "ids")?),
                    curve,
                }
                .into(),
            ])
        }
        "interpolation" => {
            let interpolation = match text(v, "interpolation")? {
                "linear" => Interpolation::Linear,
                "bezier" => Interpolation::Bezier,
                "hold" => Interpolation::Hold,
                "continuous" => Interpolation::Continuous,
                "auto" => Interpolation::Auto,
                _ => return Err(invalid("interpolation")),
            };
            edit(vec![
                KeyframeCommand::SetInterpolation {
                    key_ids: c.model.expand_scale_keys(&ids(v, "ids")?),
                    side: side(v)?,
                    interpolation,
                }
                .into(),
            ])
        }
        "simplify" => edit(vec![
            KeyframeCommand::Simplify {
                scope: SimplifyScope::Layers(ids(v, "ids")?),
            }
            .into(),
        ]),
        "command" => c.model.command(
            text(v, "command")?,
            context.time,
            None,
            false,
            context.millis,
            context.property_facts,
        )?,
        "proEdit" => {
            let command = pro_edit(v)?;
            edit(vec![command.into()])
        }
        _ => return Ok(None),
    };
    let effects = c.dispatch(plan).await?;
    if let Some(layers) = reveal_dimensions {
        for id in layers {
            let d = c.model.reveal.layers.entry(id).or_default();
            d.collapsed = false;
            if let Some(paths) = &mut d.paths {
                for path in ["position.x", "position.y", "position.z"] {
                    if !paths.iter().any(|p| p == path) {
                        paths.push(path.into());
                    }
                }
            }
        }
        c.model.rebuild_rows();
    }
    if action == "separateDimensions" || persist_view {
        if let Err(error) = c.publish_view().await {
            if let Some(published) = c.kernel.published(&c.project) {
                c.model.restore_ui(&published.ui_state);
            }
            return Err(error.into());
        }
    }
    Ok(Some(effects))
}
