//! Plans over v1 edge objects. A pair is submitted as one transaction, so
//! adding, changing or deleting both sides is one history entry.
use super::{
    Command,
    effects::{Edge, EffectCommand, TransitionSpec},
};
use crate::model::{find_layer, layer_at, list_layers};
use pm_model::{Node, Value};

/// The transition's existing edge, plus the adjacent edge in Tracks mode.
/// No persistent cut object is created. Ambiguous cuts are refused rather
/// than changing an arbitrary clip when several clips occupy one track.
pub fn plan(
    root: &Node,
    target: &str,
    edge: Edge,
    transition: Option<TransitionSpec>,
    pair: bool,
) -> Result<Vec<Command>, String> {
    let loc = find_layer(root, target).ok_or("Layer not found")?;
    let node = layer_at(root, &loc).ok_or("Layer not found")?;
    let layer = node.to_value();
    if layer.get("type").and_then(Value::as_str) == Some("audio") {
        return Err("Audio layers do not support visual transitions".into());
    }
    // Removal follows the stored reciprocal peer, even if a later edit moved
    // that clip. Never remove a newly adjacent clip's unrelated v1 fade.
    if transition.is_none() {
        return Ok(vec![
            EffectCommand::SetTransition {
                layer_id: target.into(),
                edge,
                transition: None,
            }
            .into(),
        ]);
    }
    let mut edges = vec![(target.to_owned(), edge)];
    if pair {
        let track = layer.get("track").and_then(Value::as_f64).unwrap_or(0.);
        {
            let clips = list_layers(root, loc.comp_id.as_deref());
            let clip = clips
                .iter()
                .find(|c| c.id == target)
                .ok_or("Layer not found")?;
            let cut = if edge == Edge::In {
                clip.in_time
            } else {
                clip.out_time
            };
            let peers: Vec<_> = clips
                .iter()
                .filter(|c| {
                    if c.id == target || c.kind == "audio" || c.group != clip.group {
                        return false;
                    }
                    let boundary = if edge == Edge::In {
                        c.out_time
                    } else {
                        c.in_time
                    };
                    if (boundary - cut).abs() > 0.000001 {
                        return false;
                    }
                    let Some(peer_loc) = find_layer(root, &c.id) else {
                        return false;
                    };
                    layer_at(root, &peer_loc).is_some_and(|p| {
                        p.to_value()
                            .get("track")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.)
                            == track
                    })
                })
                .collect();
            if peers.len() > 1 {
                return Err("More than one clip meets this cut".into());
            }
            if let Some(peer) = peers.first() {
                edges.push((
                    peer.id.clone(),
                    if edge == Edge::In {
                        Edge::Out
                    } else {
                        Edge::In
                    },
                ));
            }
        }
    }
    if edges.len() == 2 {
        let (left, right) = if edge == Edge::Out {
            (&edges[0].0, &edges[1].0)
        } else {
            (&edges[1].0, &edges[0].0)
        };
        return Ok(vec![
            EffectCommand::SetPairedTransition {
                left: left.clone(),
                right: right.clone(),
                offset: None,
                transition: transition.map(Box::new),
            }
            .into(),
        ]);
    }
    Ok(edges
        .into_iter()
        .map(|(layer_id, edge)| {
            EffectCommand::SetTransition {
                layer_id,
                edge,
                transition: transition.clone().map(Box::new),
            }
            .into()
        })
        .collect())
}

pub(super) fn apply_pair(
    cx: &mut super::Cx<'_>,
    left: &str,
    right: &str,
    spec: Option<&TransitionSpec>,
    requested_offset: Option<f64>,
) -> Result<(), crate::error::CommandError> {
    use super::{effects, values};
    use crate::error::CommandError;
    let (la, a) = cx.layer_mut_checked(left)?;
    let (lb, b) = cx.layer_mut_checked(right)?;
    let n = |v: &Value, key: &str| v.get(key).and_then(Value::as_f64).unwrap_or(0.);
    if [
        a.get("type").and_then(Value::as_str),
        b.get("type").and_then(Value::as_str),
    ]
    .contains(&Some("audio"))
    {
        return Err(CommandError::Invalid(
            "Audio layers do not support visual transitions".into(),
        ));
    }
    let left_paired = a
        .get("transitionOut")
        .and_then(|t| t.get("paired"))
        .and_then(Value::as_str)
        == Some(right);
    let right_paired = b
        .get("transitionIn")
        .and_then(|t| t.get("paired"))
        .and_then(Value::as_str)
        == Some(left);
    let existing = left_paired && right_paired;
    let Some(spec) = spec else {
        // Deletion follows stored links after a move too. It is idempotent
        // and never removes an unrelated v1 edge fade on either clip.
        if left_paired {
            effects::unlink_transition_peer(cx, left, Edge::Out, None)?;
            effects::set_transition(cx, left, Edge::Out, None)?;
        }
        if right_paired {
            effects::unlink_transition_peer(cx, right, Edge::In, None)?;
            effects::set_transition(cx, right, Edge::In, None)?;
        }
        return Ok(());
    };
    let cut = n(&a, "from") + n(&a, "dur");
    if la.comp_id != lb.comp_id
        || left == right
        || (cut - n(&b, "from")).abs() > 1e-6
        || n(&a, "track") != n(&b, "track")
        || a.get("group").and_then(Value::as_str) != b.get("group").and_then(Value::as_str)
    {
        return Err(CommandError::Invalid(
            "Transition clips must meet on the same track".into(),
        ));
    }
    let (effective_duration, _) = fit_overlap(
        cx.root(),
        &a,
        &b,
        spec.duration.unwrap_or(0.5),
        requested_offset,
    )?;
    if effective_duration <= 0. && !existing {
        return Err(CommandError::Invalid(
            "Not enough source media for a transition".into(),
        ));
    }
    let duration = spec.duration.unwrap_or(0.5);
    let offset = requested_offset
        .unwrap_or(-duration / 2.)
        .clamp(-duration, 0.);
    effects::unlink_transition_peer(cx, left, Edge::Out, Some(right))?;
    effects::unlink_transition_peer(cx, right, Edge::In, Some(left))?;
    let mut spec = spec.clone();
    spec.duration = Some(duration);
    for (id, peer, edge, field) in [
        (left, right, Edge::Out, "transitionOut"),
        (right, left, Edge::In, "transitionIn"),
    ] {
        effects::set_transition(cx, id, edge, Some(&spec))?;
        let (loc, mut clip) = cx.layer_mut_checked(id)?;
        let transition = clip
            .as_object_mut()
            .unwrap()
            .get_mut(field)
            .unwrap()
            .as_object_mut()
            .unwrap();
        transition.insert("paired", values::s(peer));
        transition.insert("offset", Value::Number(offset));
        cx.put_layer(&loc, clip)?;
    }
    Ok(())
}

/// The immutable v1 built-in defaults, shared by menu actions. Registry-driven
/// browsers pass their winning definition directly instead.
pub fn builtin_spec(kind: &str) -> Option<TransitionSpec> {
    use super::effects::{ParamDefinition, TransitionDefinition};
    let definition = pm_fx_defs::builtins().transition(kind)?;
    Some(TransitionSpec {
        definition: TransitionDefinition {
            transition_type: kind.into(),
            params: definition
                .params
                .iter()
                .map(|p| ParamDefinition {
                    key: p.k.clone(),
                    kind: p.kind.clone().unwrap_or_else(|| "number".into()),
                    default: pm_model::json::parse(&p.def.to_string())
                        .expect("JSON parameter default"),
                    min: p.min.as_ref().and_then(|n| n.as_f64()),
                    max: p.max.as_ref().and_then(|n| n.as_f64()),
                    reference: false,
                })
                .collect(),
        },
        duration: Some(0.5),
        params: Default::default(),
    })
}

/// One fitter for admission, capsule drawing and render/export evaluation.
pub use pm_eval::cut_transition::{Fitted, SHORTENED_REASON, fit as fit_nodes, fitted_pair};
pub const CUT_RETIMING_REASON: &str = pm_eval::cut_transition::RETIMING_REASON;
pub fn fitted_edge(root: &Node, layer: &Node, edge: Edge) -> Option<Fitted> {
    let field = if edge == Edge::In {
        "transitionIn"
    } else {
        "transitionOut"
    };
    let peer = layer.get(field)?.get("paired")?.to_value();
    let loc = find_layer(root, peer.as_str()?)?;
    let id = layer.get("id")?.to_value();
    if find_layer(root, id.as_str()?)?.comp_id != loc.comp_id {
        return None;
    }
    fitted_pair(root, layer, layer_at(root, &loc)?, edge == Edge::In)
}
pub fn fit_overlap(
    root: &Node,
    a: &Value,
    b: &Value,
    requested: f64,
    requested_offset: Option<f64>,
) -> Result<(f64, f64), crate::error::CommandError> {
    let fitted = fit_nodes(
        root,
        &Node::from_value(a.clone(), 0),
        &Node::from_value(b.clone(), 0),
        requested,
        requested_offset.unwrap_or(-requested / 2.),
    )
    .map_err(|reason| crate::error::CommandError::Invalid(reason.into()))?;
    Ok((fitted.duration, fitted.offset))
}

/// v1 single-edge fades remain available; only render-only cut overlaps need
/// source handles on both sides of the original edit point.
pub fn retiming_refusal(layer: &Value) -> Option<&'static str> {
    let d = layer.get("d")?;
    let animated = |value: &Value| {
        value
            .get("kf")
            .and_then(Value::as_array)
            .is_some_and(|keys| !keys.is_empty())
            || value.get("expr").is_some_and(Value::is_truthy)
    };
    let remapped = d.get("timeRemap").is_some_and(|value| {
        if value.as_object().is_some() {
            animated(value) || value.get("v").is_some_and(Value::is_truthy)
        } else {
            value.is_truthy()
        }
    });
    let speed_animated = d.get("speed").is_some_and(animated);
    (remapped || speed_animated).then_some(CUT_RETIMING_REASON)
}

/// Read-only admission for menus and drag targets. The kernel repeats the
/// check when executing the actual command, including MCP requests.
pub fn refusal_for_commands(root: &Node, commands: &[Command]) -> Option<&'static str> {
    for command in commands {
        if let Command::Effect(EffectCommand::SetPairedTransition {
            left,
            right,
            transition: Some(_),
            ..
        }) = command
        {
            for id in [left, right] {
                let loc = find_layer(root, id)?;
                let layer = layer_at(root, &loc)?.to_value();
                if let Some(reason) = retiming_refusal(&layer) {
                    return Some(reason);
                }
            }
        }
    }
    None
}

/// Split keeps the original head ID and gives the tail a fresh ID. Only
/// the outgoing edge changes owner; repair its reciprocal incoming link
/// in the same transaction. Geometry and handle loss still use the fitter.
pub(super) fn after_split(
    clips: &mut [Value],
    splits: &[(String, String)],
) -> Result<(), crate::error::CommandError> {
    use super::{layers, values};
    use crate::error::CommandError;
    let index_by_id: std::collections::HashMap<String, usize> = clips
        .iter()
        .enumerate()
        .filter_map(|(i, clip)| values::text(clip, "id").map(|id| (id.to_owned(), i)))
        .collect();
    for (source, tail) in splits {
        let peer = index_by_id
            .get(tail)
            .and_then(|&ti| clips[ti].get("transitionOut"))
            .and_then(|edge| values::text(edge, "paired"))
            .map(str::to_owned);
        let Some(peer) = peer else { continue };
        let Some(&index) = index_by_id.get(&peer) else {
            continue;
        };
        if clips[index]
            .get("transitionIn")
            .and_then(|edge| values::text(edge, "paired"))
            != Some(source.as_str())
        {
            continue;
        }
        if values::flag(&clips[index], "lock")
            || layers::ancestors(clips, &clips[index])
                .iter()
                .any(|group| values::flag(group, "lock"))
        {
            return Err(CommandError::Invalid(
                "Unlock the other transition clip before splitting.".into(),
            ));
        }
        let edge = values::obj(&mut clips[index])
            .get_mut("transitionIn")
            .unwrap();
        values::obj(edge).insert("paired", values::s(tail));
    }
    // Never turn an orphan into a v1 fade by merely removing its flag.
    // Clear the broken edge on the split pieces; unrelated edges stay put.
    let touched: std::collections::HashSet<_> = splits
        .iter()
        .flat_map(|(head, tail)| [head.as_str(), tail.as_str()])
        .collect();
    let mut clear = Vec::new();
    for (index, clip) in clips.iter().enumerate() {
        let Some(id) = values::text(clip, "id").filter(|id| touched.contains(id)) else {
            continue;
        };
        for (field, other) in [
            ("transitionIn", "transitionOut"),
            ("transitionOut", "transitionIn"),
        ] {
            let Some(peer) = clip
                .get(field)
                .and_then(|edge| values::text(edge, "paired"))
            else {
                continue;
            };
            let reciprocal = clips
                .iter()
                .find(|clip| values::text(clip, "id") == Some(peer))
                .and_then(|clip| clip.get(other))
                .and_then(|edge| values::text(edge, "paired"));
            if reciprocal != Some(id) {
                clear.push((index, field));
            }
        }
    }
    for (index, field) in clear {
        values::obj(&mut clips[index]).insert(field, Value::Null);
    }
    Ok(())
}
