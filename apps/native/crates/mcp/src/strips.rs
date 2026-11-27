//! Timeline strip gestures, with the tag's explicit-target and sibling rules.
use crate::{Error, Result, commands, json_value, model};
use pm_kernel::{Command, DocumentStore, LayerCommand, Origin, TimingCommand, WriteBody};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
pub struct Plan {
    pub body: WriteBody,
    pub result: Value,
    pub results: Vec<Value>,
    pub state: bool,
}
fn id(l: &Value) -> &str {
    l["id"].as_str().unwrap_or("")
}
fn from(l: &Value) -> f64 {
    l["from"].as_f64().unwrap_or(0.)
}
fn dur(l: &Value) -> f64 {
    l["dur"].as_f64().unwrap_or(0.)
}
fn expanded(layers: &[Value], ids: &[String]) -> HashSet<String> {
    let mut wanted = ids.iter().cloned().collect::<HashSet<_>>();
    loop {
        let n = wanted.len();
        for l in layers {
            if l["group"].as_str().is_some_and(|g| wanted.contains(g)) {
                wanted.insert(id(l).into());
            }
        }
        if wanted.len() == n {
            return wanted;
        }
    }
}
fn editable(layers: &[Value], ids: &HashSet<String>) -> Result<()> {
    for l in layers.iter().filter(|l| ids.contains(id(l))) {
        let mut current = Some(l);
        let mut seen = HashSet::new();
        while let Some(c) = current {
            if c["lock"] == true {
                return Err(Error::invalid("Layer or enclosing group is locked"));
            }
            let Some(group) = c["group"].as_str() else {
                break;
            };
            if !seen.insert(group) {
                return Err(Error::invalid("Invalid group cycle"));
            }
            current = layers.iter().find(|l| id(l) == group);
        }
        if l["locked_intent"]
            .as_object()
            .is_some_and(|o| o.values().any(|v| !v.is_null() && v != &json!(false)))
        {
            return Err(Error::invalid("Layer has protected hand edits"));
        }
    }
    Ok(())
}
fn realize(s: &DocumentStore) -> Value {
    json_value(&pm_model::canonical::realize_project(
        s.snapshot().document.to_value().unwrap(),
    ))
}
pub fn plan(project: &Value, a: &Value, time: f64) -> Result<Plan> {
    let layers = project["layers"].as_array().unwrap();
    let ids = a["layerIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let selected = ids
        .iter()
        .map(|wanted| {
            layers
                .iter()
                .find(|l| id(l) == wanted)
                .ok_or_else(|| Error::invalid(format!("Layer not found: {wanted}")))
        })
        .collect::<Result<Vec<_>>>()?;
    let operation = a["operation"].as_str().unwrap();
    let expanded = expanded(layers, &ids);
    editable(layers, &expanded)?;
    let affected = layers
        .iter()
        .filter(|l| expanded.contains(id(l)))
        .collect::<Vec<_>>();
    let affected_ids = affected
        .iter()
        .map(|l| id(l).to_owned())
        .collect::<Vec<_>>();
    if operation == "state" {
        let patch = a["patch"]
            .as_object()
            .filter(|o| !o.is_empty())
            .ok_or_else(|| Error::invalid("Supply a state patch"))?;
        for (k, v) in patch {
            if ["name", "color"].contains(&k.as_str()) {
                if v.as_str().is_none_or(|s| s.trim().is_empty()) {
                    return Err(Error::invalid(format!("Invalid {k}")));
                }
            } else if !["visible", "solo", "shy", "locked", "collapsed"].contains(&k.as_str())
                || !v.is_boolean()
            {
                return Err(Error::invalid(format!("Invalid {k}")));
            }
        }
        let raw = ids
            .iter()
            .map(|id| json!({"type":"set_layer","target":id,"patch":patch}))
            .collect::<Vec<_>>();
        let batch = commands::plan(project, &[], &raw, time)?;
        return Ok(Plan {
            body: WriteBody::Commands(batch.commands),
            result: Value::Null,
            results: batch.results,
            state: true,
        });
    }
    let fps = project["fps"].as_f64().unwrap_or(30.);
    let duration = project["dur"].as_f64().unwrap_or(10.);
    let frame = |key: &str, default: Option<f64>, negative: bool| -> Result<f64> {
        let n = a[key]
            .as_f64()
            .or(default)
            .ok_or_else(|| Error::invalid(format!("{key} is required")))?;
        if n < if negative { -duration } else { 0. } || n > duration {
            return Err(Error::invalid(format!("Invalid {key}")));
        }
        Ok((n * fps).round() / fps)
    };
    let fit = |l: &Value, start: f64| -> Result<()> {
        if start < -1e-7 || start + dur(l) > duration + 1e-7 {
            Err(Error::invalid(
                "Every affected strip must fit the composition. Extend it with apply_commands first.",
            ))
        } else {
            Ok(())
        }
    };
    let mut scratch = DocumentStore::new_untitled(model(project));
    let before = scratch.snapshot();
    let mut cmds: Vec<Command> = vec![];
    let mut result = json!({"layerIds":affected_ids});
    let mut reordered = None;
    match operation {
        "move" | "duplicate" => {
            let delta = if a.get("at").is_some() {
                frame("at", None, false)?
                    - affected
                        .iter()
                        .map(|l| from(l))
                        .fold(f64::INFINITY, f64::min)
            } else {
                frame("delta", None, true)?
            };
            for l in &affected {
                fit(l, from(l) + delta)?;
            }
            if operation == "move" {
                cmds.push(
                    TimingCommand::Move {
                        layer_ids: ids.clone(),
                        delta,
                    }
                    .into(),
                );
            } else {
                let new_ids = affected
                    .iter()
                    .map(|_| pm_kernel::new_id("L"))
                    .collect::<Vec<_>>();
                let mapping = affected_ids
                    .iter()
                    .cloned()
                    .zip(new_ids.iter().cloned())
                    .collect::<HashMap<_, _>>();
                cmds.push(
                    LayerCommand::Duplicate {
                        layer_ids: affected_ids.clone(),
                        new_ids: new_ids.clone(),
                    }
                    .into(),
                );
                // Move only copy roots: TimingCommand expands their descendants.
                let copy_roots = ids
                    .iter()
                    .filter(|wanted| {
                        let l = layers.iter().find(|l| id(l) == wanted.as_str()).unwrap();
                        !l["group"].as_str().is_some_and(|g| expanded.contains(g))
                    })
                    .map(|id| mapping[id].clone())
                    .collect::<Vec<_>>();
                cmds.push(
                    TimingCommand::Move {
                        layer_ids: copy_roots,
                        delta,
                    }
                    .into(),
                );
                for c in &cmds {
                    scratch
                        .execute(c.clone(), Origin::Mcp)
                        .map_err(Error::kernel)?;
                }
                cmds.clear();
                let value = realize(&scratch);
                let next = value["layers"].as_array().unwrap();
                let mut ordered = vec![];
                for l in layers {
                    if ids.iter().any(|wanted| wanted == id(l))
                        && !l["group"].as_str().is_some_and(|g| expanded.contains(g))
                    {
                        let block = expanded_ids(layers, id(l));
                        for source in layers.iter().filter(|l| block.contains(id(l))) {
                            ordered.push(
                                next.iter()
                                    .find(|l| id(l) == mapping[id(source)])
                                    .unwrap()
                                    .clone(),
                            );
                        }
                    }
                    ordered.push(next.iter().find(|n| id(n) == id(l)).unwrap().clone());
                }
                let mut value = value;
                value["layers"] = json!(ordered);
                reordered = Some(value);
                result = json!({"layerIds":new_ids,"copies":affected_ids.iter().zip(new_ids.iter()).map(|(source,id)|json!({"sourceId":source,"layerId":id})).collect::<Vec<_>>()});
            }
        }
        "sequence" | "align" => {
            if selected.iter().any(|l| l["type"] == "group") {
                return Err(Error::invalid(
                    "Sequence or align individual strips; move a group as a block.",
                ));
            }
            let mut at = frame("at", None, false)?;
            let gap = if operation == "sequence" {
                frame("gap", Some(0.), true)?
            } else {
                0.
            };
            for l in selected {
                let start = if operation == "align" && a["edge"] == "end" {
                    at - dur(l)
                } else {
                    at
                };
                fit(l, start)?;
                cmds.push(
                    LayerCommand::Set {
                        layer_id: id(l).into(),
                        patch: pm_kernel::commands::layers::LayerPatch {
                            from: Some(start),
                            ..Default::default()
                        },
                    }
                    .into(),
                );
                if operation == "sequence" {
                    at = ((at + dur(l) + gap) * fps).round() / fps;
                }
            }
        }
        "close_gap" => {
            let start = frame("start", None, false)?;
            let end = frame("end", None, false)?;
            if end <= start {
                return Err(Error::invalid("Gap must be at least one frame"));
            }
            if affected
                .iter()
                .any(|l| from(l) < end - 1e-7 && from(l) + dur(l) > start + 1e-7)
            {
                return Err(Error::invalid(
                    "The gap intersects an affected strip. Trim, split, or remove it first.",
                ));
            }
            for l in &affected {
                if from(l) >= end - 1e-7 {
                    cmds.push(
                        LayerCommand::Set {
                            layer_id: id(l).into(),
                            patch: pm_kernel::commands::layers::LayerPatch {
                                from: Some(from(l) - (end - start)),
                                ..Default::default()
                            },
                        }
                        .into(),
                    );
                }
            }
        }
        "remove" => {
            let parented = layers
                .iter()
                .filter(|l| {
                    !expanded.contains(id(l))
                        && l["parent"].as_str().is_some_and(|p| expanded.contains(p))
                })
                .map(|l| id(l).to_owned())
                .collect::<HashSet<_>>();
            editable(layers, &parented)?;
            cmds.push(LayerCommand::Delete { layer_ids: ids }.into());
        }
        "reorder" => {
            let anchor = a["anchorId"]
                .as_str()
                .and_then(|id| layers.iter().find(|l| l["id"] == id))
                .ok_or_else(|| Error::invalid("Anchor layer not found"))?;
            editable(layers, &HashSet::from([id(anchor).to_owned()]))?;
            if expanded.contains(id(anchor)) {
                return Err(Error::invalid(
                    "The anchor must be outside the moved strips",
                ));
            }
            if selected
                .iter()
                .any(|l| l["group"].as_str() != anchor["group"].as_str())
            {
                return Err(Error::invalid(
                    "Reorder siblings; use move_to_group to change membership.",
                ));
            }
            let mut remaining = layers
                .iter()
                .filter(|l| !expanded.contains(id(l)))
                .cloned()
                .collect::<Vec<_>>();
            let anchor_block = expanded_ids(layers, id(anchor));
            let index = match a["placement"].as_str() {
                Some("before") => remaining.iter().position(|l| id(l) == id(anchor)).unwrap(),
                Some("after") => {
                    remaining
                        .iter()
                        .rposition(|l| anchor_block.contains(id(l)))
                        .unwrap()
                        + 1
                }
                _ => return Err(Error::invalid("Choose before or after")),
            };
            remaining.splice(index..index, affected.iter().map(|l| (*l).clone()));
            let mut value = project.clone();
            value["layers"] = json!(remaining);
            reordered = Some(value);
        }
        _ => return Err(Error::invalid("Unknown strip operation")),
    }
    for c in cmds {
        scratch.execute(c, Origin::Mcp).map_err(Error::kernel)?;
    }
    if let Some(value) = reordered {
        scratch
            .restore_project(model(&value), "Reorder strips", Origin::Mcp)
            .map_err(Error::kernel)?;
    }
    let after = scratch.snapshot();
    let ops = pm_model::ops::diff_node_ops(before.root(), after.root());
    Ok(Plan {
        body: WriteBody::Ops(ops),
        result,
        results: vec![],
        state: false,
    })
}
fn expanded_ids(layers: &[Value], id: &str) -> HashSet<String> {
    expanded(layers, &[id.into()])
}
