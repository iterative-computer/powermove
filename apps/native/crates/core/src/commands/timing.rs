//! Layer timing: strip moves, trims, `[`/`]`, slip, split and stagger.
//!
//! Ports the timeline's strip gestures (`extensions/timeline/timeline.ts`),
//! its split command (`extensions/timeline/index.ts`: Cmd+Shift+D / Cmd+B),
//! the layer-timing shortcuts of `extensions/keymap-default/keymap.ts`, the
//! quick stagger (Cmd+Option drag), and the agent's video edits
//! (`legacy/assistant/video-editing.ts`: split, trim, slip).
//!
//! Key times are layer-local, so moving a layer moves its animation with it.
//! Group strips act on their members; caption cues stay put in composition
//! time. There is no ripple edit in the old app (the parity inventory found
//! none), so none is offered.

use std::collections::HashSet;

use pm_model::{Map, Value};

use super::{Cx, channels, layers, new_id, track, values};
use crate::error::CommandError;

/// Timing commands. Explicitly targeted locked layers refuse, except where
/// a variant says they are skipped.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TimingCommand {
    /// Strip drag: `from += delta` for each layer (and a group's members).
    /// Label "Move Layers".
    Move { layer_ids: Vec<String>, delta: f64 },
    /// Drag the In edge / Option+`[` (the timeline's `trim`): the layer
    /// starts at `time` and keeps its end, at least one frame remains; video
    /// and audio cannot start before their source (`earliestStart`) and keep
    /// showing the same source frames (`d.trim` follows the edge); caption
    /// cues stay in place. Keys are layer-local and move with the In point,
    /// as before. Group and locked layers are skipped. Send absolute times
    /// in a drag. Label "Trim Layer".
    TrimIn { layer_ids: Vec<String>, time: f64 },
    /// Drag the Out edge / Option+`]`: the layer ends at `time`, at least
    /// one frame long. Group and locked layers are skipped. Label "Trim
    /// Layer".
    TrimOut { layer_ids: Vec<String>, time: f64 },
    /// `[`: move the layer so its In point is at `time` (Option+Home: the
    /// composition start). Label "Move Layer In".
    MoveInTo { layer_ids: Vec<String>, time: f64 },
    /// `]`: move the layer so its Out point is at `time` (Option+End: the
    /// composition end). Label "Move Layer Out".
    MoveOutTo { layer_ids: Vec<String>, time: f64 },
    /// Slip (`video-editing.ts` `slip`): a video or audio layer shows its
    /// source from `source_in` seconds, keeping the strip and its animation
    /// where they are (`d.trim = P(sourceIn)`). Refuses when the source's
    /// known duration cannot cover the strip (`Slip exceeds available source
    /// handles.`), a locked clip, and protected hand edits. Label "Slip
    /// Layer".
    Slip { layer_id: String, source_in: f64 },
    /// Split at `time` (`splitLayers`, Cmd+Shift+D): each unlocked layer
    /// whose span strictly contains `time` becomes a head and a tail copy
    /// inserted above it (`PM.cloneLayer`); the tail's keys are re-based so
    /// the animation is continuous, media continue at the matching source
    /// time, caption cues are split between the halves, the head loses its
    /// out transition and the tail its in transition (audio: the fades at
    /// the cut are cleared). A group splits its whole hierarchy: crossing
    /// rows get tails, rows after the cut move into the tail group. The
    /// tail of `layer_ids[i]` gets `new_ids[i]` (same length; select those);
    /// inner tails of a group get fresh ids. Label "Split".
    Split {
        layer_ids: Vec<String>,
        time: f64,
        new_ids: Vec<String>,
    },
    /// Quick stagger (Cmd+Option drag): the i-th layer (in the given
    /// order) moves by `i * step`, at subframe precision (send the whole
    /// drag with `update_gesture_from_start`). Label "Stagger Layers".
    Stagger { layer_ids: Vec<String>, step: f64 },
}

impl TimingCommand {
    pub fn label(&self) -> String {
        match self {
            TimingCommand::Move { .. } => "Move Layers",
            TimingCommand::TrimIn { .. } | TimingCommand::TrimOut { .. } => "Trim Layer",
            TimingCommand::MoveInTo { .. } => "Move Layer In",
            TimingCommand::MoveOutTo { .. } => "Move Layer Out",
            TimingCommand::Slip { .. } => "Slip Layer",
            TimingCommand::Split { .. } => "Split",
            TimingCommand::Stagger { .. } => "Stagger Layers",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            TimingCommand::Move { layer_ids, delta } => {
                let d = values::finite(*delta, "delta")?;
                shift(cx, layer_ids, |_, from, _| from + d, true)
            }
            TimingCommand::TrimIn { layer_ids, time } => trim(cx, layer_ids, *time, true),
            TimingCommand::TrimOut { layer_ids, time } => trim(cx, layer_ids, *time, false),
            TimingCommand::MoveInTo { layer_ids, time } => {
                let t = values::finite(*time, "time")?;
                shift(cx, layer_ids, |_, _, _| t, false)
            }
            TimingCommand::MoveOutTo { layer_ids, time } => {
                let t = values::finite(*time, "time")?;
                shift(cx, layer_ids, |_, _, dur| t - dur, false)
            }
            TimingCommand::Slip {
                layer_id,
                source_in,
            } => slip(cx, layer_id, *source_in),
            TimingCommand::Split {
                layer_ids,
                time,
                new_ids,
            } => split(cx, layer_ids, *time, new_ids),
            TimingCommand::Stagger { layer_ids, step } => {
                let step = values::finite(*step, "step")?;
                let order: Vec<String> = layer_ids.clone();
                shift(
                    cx,
                    layer_ids,
                    |id, from, _| {
                        let i = order.iter().position(|x| x == id).unwrap_or(0);
                        from + i as f64 * step
                    },
                    false,
                )
            }
        }
    }
}

fn layer_of<'v>(layers: &'v [Value], id: &str) -> Option<&'v Value> {
    layers.iter().find(|l| values::text(l, "id") == Some(id))
}

/// Sets each target's `from` (explicit locked targets refuse; with
/// `groups`, a group's members move with it and locked members stay).
fn shift(
    cx: &mut Cx<'_>,
    layer_ids: &[String],
    to: impl Fn(&str, f64, f64) -> f64,
    groups: bool,
) -> Result<(), CommandError> {
    let mut targets: Vec<(String, f64)> = Vec::new();
    for id in layer_ids {
        let (loc, layer) = cx.layer(id)?;
        cx.check_lock(&loc, id)?;
        let from = values::num(&layer, "from").unwrap_or(0.0);
        let dur = values::num(&layer, "dur").unwrap_or(0.0);
        let next = to(id, from, dur);
        let delta = next - from;
        targets.push((id.clone(), next));
        if groups && values::text(&layer, "type") == Some("group") {
            let layers = cx.layers(loc.comp_id.as_deref());
            let one: HashSet<&str> = [id.as_str()].into_iter().collect();
            for member in layers::expand_groups(&layers, &one) {
                if member == *id
                    || layer_ids.contains(&member)
                    || targets.iter().any(|(t, _)| *t == member)
                {
                    continue;
                }
                let Some(m) = layer_of(&layers, &member) else {
                    continue;
                };
                if values::flag(m, "lock") {
                    continue;
                }
                targets.push((
                    member.clone(),
                    values::num(m, "from").unwrap_or(0.0) + delta,
                ));
            }
        }
    }
    for (id, from) in targets {
        let loc = cx.find(&id)?;
        let mut path = loc.path();
        path.push("from".into());
        cx.set(path, Value::Number(values::finite(from, "layer start")?))?;
    }
    Ok(())
}

/// `PM.MediaTiming.isTimed`.
fn is_timed(layer: &Value) -> bool {
    matches!(values::text(layer, "type"), Some("audio" | "video"))
}

/// A content field's static value (`resolveContent` without animation).
fn content_number(layer: &Value, key: &str) -> Option<f64> {
    let v = layer.get("d")?.get(key)?;
    let v = if values::is_property(v) {
        v.get("v")?
    } else {
        v
    };
    Some(track::js_number(v))
}

/// `PM.MediaTiming.rate`.
fn rate(layer: &Value) -> f64 {
    if values::text(layer, "type") == Some("video") {
        let s = content_number(layer, "speed")
            .filter(|s| *s != 0.0 && !s.is_nan())
            .unwrap_or(1.0);
        s.max(0.0001)
    } else {
        1.0
    }
}

fn trim_of(layer: &Value) -> f64 {
    content_number(layer, "trim")
        .filter(|t| !t.is_nan())
        .unwrap_or(0.0)
        .max(0.0)
}

/// `earliestStart(L)`.
fn earliest_start(layer: &Value) -> f64 {
    values::num(layer, "from").unwrap_or(0.0) - trim_of(layer) / rate(layer)
}

/// Writes `d[key]` keeping a channel's form.
fn set_content(layer: &mut Value, key: &str, v: Value) {
    let m = values::obj(layer);
    if m.get("d").and_then(Value::as_object).is_none() {
        m.insert("d", Value::Object(Map::new()));
    }
    let d = m.get_mut("d").and_then(Value::as_object_mut).expect("made");
    match d.get_mut(key) {
        Some(ch) if values::is_property(ch) => {
            values::obj(ch).insert("v", v);
        }
        _ => {
            d.insert(key, v);
        }
    }
}

/// `shiftAllCues(cues, by)`: every cue and word time moves by `by`.
fn shift_cues(cues: &mut [Value], by: f64) {
    for cue in cues {
        for k in ["start", "end"] {
            if let Some(t) = values::num(cue, k) {
                values::obj(cue).insert(k, Value::Number(t + by));
            }
        }
        if let Some(words) = cue
            .as_object_mut()
            .and_then(|c| c.get_mut("words"))
            .and_then(Value::as_array_mut)
        {
            for w in words {
                for k in ["start", "end"] {
                    if let Some(t) = values::num(w, k) {
                        values::obj(w).insert(k, Value::Number(t + by));
                    }
                }
            }
        }
    }
}

fn cues_mut(layer: &mut Value) -> Option<&mut Vec<Value>> {
    layer
        .as_object_mut()?
        .get_mut("d")?
        .as_object_mut()?
        .get_mut("cues")?
        .as_array_mut()
}

/// `startPatch(L, nextFrom)` applied to `layer`.
fn start_patch(layer: &mut Value, source: &Value, next_from: f64) {
    let from = values::num(source, "from").unwrap_or(0.0);
    if is_timed(source) {
        let trim = (trim_of(source) + (next_from - from) * rate(source)).max(0.0);
        set_content(layer, "trim", Value::Number(trim));
    } else if values::text(source, "type") == Some("captions")
        && let Some(cues) = cues_mut(layer)
    {
        let delta = next_from - from;
        if delta.is_finite() && delta != 0.0 {
            shift_cues(cues, -delta);
        }
    }
}

/// Trim In / Trim Out (the timeline's `trim`): groups and locked layers are
/// skipped.
fn trim(cx: &mut Cx<'_>, layer_ids: &[String], time: f64, start: bool) -> Result<(), CommandError> {
    let time = values::finite(time, "time")?;
    for id in layer_ids {
        let (loc, layer) = cx.layer(id)?;
        if values::text(&layer, "type") == Some("group") || cx.check_lock(&loc, id).is_err() {
            continue;
        }
        let fps = cx.fps(loc.comp_id.as_deref());
        let from = values::num(&layer, "from").unwrap_or(0.0);
        let dur = values::num(&layer, "dur").unwrap_or(0.0);
        let mut next = layer.clone();
        if start {
            let mut nf = time.min(from + dur - 1.0 / fps);
            if is_timed(&layer) {
                nf = nf.max(earliest_start(&layer));
            }
            let m = values::obj(&mut next);
            m.insert("from", Value::Number(nf));
            m.insert("dur", Value::Number(dur + (from - nf)));
            start_patch(&mut next, &layer, nf);
        } else {
            values::obj(&mut next).insert("dur", Value::Number((1.0 / fps).max(time - from)));
        }
        cx.put_layer(&loc, next)?;
    }
    Ok(())
}

/// The agent's `slip`.
fn slip(cx: &mut Cx<'_>, layer_id: &str, source_in: f64) -> Result<(), CommandError> {
    let (loc, layer) = cx.layer(layer_id)?;
    if !is_timed(&layer) {
        return Err(CommandError::Invalid(
            "Choose a video or audio layer from get_project_state.".into(),
        ));
    }
    if cx.check_lock(&loc, layer_id).is_err() {
        return Err(CommandError::Invalid("The clip is locked.".into()));
    }
    if layer
        .get("locked_intent")
        .and_then(Value::as_object)
        .is_some_and(|m| m.values().any(Value::is_truthy))
    {
        return Err(CommandError::Invalid(
            "The clip contains protected hand edits. Edit unprotected channels explicitly with apply_commands.".into(),
        ));
    }
    let source_in = values::finite(source_in, "sourceIn")?;
    let rate = rate(&layer);
    let dur = values::num(&layer, "dur").unwrap_or(0.0);
    let asset = layer
        .get("d")
        .and_then(|d| d.get("asset"))
        .and_then(Value::as_str);
    let source_duration = asset
        .and_then(|a| cx.root().get("assets")?.get(a)?.get("dur"))
        .and_then(crate::model::number_of);
    if !source_duration.is_some_and(|sd| sd.is_finite() && source_in + dur * rate <= sd + 1e-6) {
        return Err(CommandError::Invalid(
            "Slip exceeds available source handles.".into(),
        ));
    }
    let mut path = loc.path();
    path.extend(["d".into(), "trim".into()]);
    cx.set(path, values::property(Value::Number(source_in)))
}

/// `groupSpan(group)`: its non-group members' extent.
fn span(layers: &[Value], layer: &Value, comp_dur: f64) -> (f64, f64) {
    if values::text(layer, "type") != Some("group") {
        return (
            values::num(layer, "from").unwrap_or(f64::NAN),
            values::num(layer, "dur").unwrap_or(f64::NAN),
        );
    }
    let one: HashSet<&str> = values::text(layer, "id").into_iter().collect();
    let ids = layers::expand_groups(layers, &one);
    let children: Vec<&Value> = ids
        .iter()
        .filter_map(|id| layer_of(layers, id))
        .filter(|l| values::text(l, "type") != Some("group"))
        .collect();
    if children.is_empty() {
        return (0.0, comp_dur);
    }
    let from = children
        .iter()
        .map(|l| values::num(l, "from").unwrap_or(0.0))
        .fold(f64::INFINITY, f64::min);
    let end = children
        .iter()
        .map(|l| values::num(l, "from").unwrap_or(0.0) + values::num(l, "dur").unwrap_or(0.0))
        .fold(f64::NEG_INFINITY, f64::max);
    (from, end - from)
}

fn crosses((from, dur): (f64, f64), t: f64) -> bool {
    from.is_finite() && dur.is_finite() && t > from && t < from + dur
}

const CUE_EPS: f64 = 1e-6;

/// `splitLayerClone(PM, layer, tail, time)`: edits the head in place and
/// returns the tail.
fn split_pair(head: &mut Value, tail_id: &str, siblings: &[Value], t: f64) -> Value {
    let source = head.clone();
    let mut tail = layers::clone_layer(&source, tail_id, siblings);
    let from = values::num(&source, "from").unwrap_or(0.0);
    let end = from + values::num(&source, "dur").unwrap_or(0.0);
    let offset = t - from;
    {
        let m = values::obj(&mut tail);
        m.insert("from", Value::Number(t));
        m.insert("dur", Value::Number(end - t));
    }
    // `rebaseTailAnimation`: keys move back by the offset.
    if offset.is_finite() && offset != 0.0 {
        for (_, path) in channels::all(&tail) {
            if let Some(keys) = channels::get_mut(&mut tail, &path)
                .and_then(|p| p.as_object_mut())
                .and_then(|p| p.get_mut("kf"))
                .and_then(Value::as_array_mut)
            {
                for k in keys {
                    let kt = track::js_number(k.get("t").unwrap_or(&Value::Null));
                    if kt.is_finite() {
                        values::obj(k).insert("t", Value::Number(kt - offset));
                    }
                }
            }
        }
    }
    start_patch(&mut tail, &source, t);
    if values::text(&source, "type") == Some("captions")
        && let Some(cues) = cues_mut(head)
    {
        // `clipCuesToEnd`.
        let cut = t - from;
        cues.retain(|c| values::num(c, "start").is_some_and(|s| s < cut - CUE_EPS));
        for c in cues.iter_mut() {
            if values::num(c, "end").is_some_and(|e| e > cut) {
                values::obj(c).insert("end", Value::Number(cut));
            }
        }
    }
    values::obj(head).insert("transitionOut", Value::Null);
    values::obj(&mut tail).insert("transitionIn", Value::Null);
    if values::text(&source, "type") == Some("audio") {
        set_content(head, "fadeOut", Value::Number(0.0));
        set_content(&mut tail, "fadeIn", Value::Number(0.0));
    }
    values::obj(head).insert("dur", Value::Number(t - from));
    tail
}

/// `splitLayers(PM)` for explicit targets.
fn split(
    cx: &mut Cx<'_>,
    layer_ids: &[String],
    t: f64,
    new_ids: &[String],
) -> Result<(), CommandError> {
    if layer_ids.len() != new_ids.len() {
        return Err(CommandError::Invalid(
            "new_ids must pair with layer_ids".into(),
        ));
    }
    let t = values::finite(t, "time")?;
    let Some(first) = layer_ids.first() else {
        return Ok(());
    };
    let comp = cx.find(first)?.comp_id;
    let comp_dur = cx.duration(comp.as_deref());
    let mut layers = cx.layers(comp.as_deref());
    let explicit: HashSet<&str> = layer_ids.iter().map(String::as_str).collect();
    let locked_in = |layers: &[Value], l: &Value, except: &HashSet<String>| {
        values::flag(l, "lock")
            || layers::ancestors(layers, l).iter().any(|g| {
                values::flag(g, "lock")
                    && !values::text(g, "id").is_some_and(|i| except.contains(i))
            })
    };
    let tail_id_for = |id: &str| {
        layer_ids
            .iter()
            .position(|x| x == id)
            .map(|i| new_ids[i].clone())
    };
    // Group roots: explicit groups not inside another explicit group.
    let roots: Vec<String> = layers
        .iter()
        .filter(|l| {
            values::text(l, "id").is_some_and(|i| explicit.contains(i))
                && values::text(l, "type") == Some("group")
                && !layers::ancestors(&layers, l)
                    .iter()
                    .any(|g| values::text(g, "id").is_some_and(|i| explicit.contains(i)))
        })
        .filter_map(|l| values::text(l, "id").map(str::to_owned))
        .collect();
    let mut grouped: HashSet<String> = HashSet::new();
    for r in &roots {
        let one: HashSet<&str> = [r.as_str()].into_iter().collect();
        grouped.extend(layers::expand_groups(&layers, &one));
    }
    let ordinary: Vec<String> = layers
        .iter()
        .filter(|l| {
            let id = values::text(l, "id").unwrap_or_default();
            explicit.contains(id)
                && values::text(l, "type") != Some("group")
                && !grouped.contains(id)
                && !values::flag(l, "lock")
                && crosses(span(&layers, l, comp_dur), t)
        })
        .filter_map(|l| values::text(l, "id").map(str::to_owned))
        .collect();
    let mut changed = false;
    for root in &roots {
        let one: HashSet<&str> = [root.as_str()].into_iter().collect();
        let ids: HashSet<String> = layers::expand_groups(&layers, &one).into_iter().collect();
        let hierarchy: Vec<Value> = layers
            .iter()
            .filter(|l| values::text(l, "id").is_some_and(|i| ids.contains(i)))
            .cloned()
            .collect();
        let Some(root_layer) = layer_of(&layers, root) else {
            continue;
        };
        if !crosses(span(&layers, root_layer, comp_dur), t)
            || hierarchy.is_empty()
            || hierarchy.iter().any(|l| locked_in(&layers, l, &ids))
        {
            continue;
        }
        changed = true;
        let mut tail_by: Vec<(String, String)> = Vec::new();
        let mut pairs: Vec<(String, String)> = Vec::new();
        for l in &hierarchy {
            let id = values::text(l, "id").unwrap_or_default().to_owned();
            let sp = span(&layers, l, comp_dur);
            if !crosses(sp, t) {
                if sp.0 >= t {
                    tail_by.push((id.clone(), id));
                }
                continue;
            }
            let Some(index) = layers
                .iter()
                .position(|x| values::text(x, "id") == Some(id.as_str()))
            else {
                continue;
            };
            let tail_id = if id == *root {
                tail_id_for(&id).unwrap_or_else(|| new_id("L"))
            } else {
                new_id("L")
            };
            let siblings = layers.clone();
            let tail = split_pair(&mut layers[index], &tail_id, &siblings, t);
            layers.insert(index, tail);
            tail_by.push((id.clone(), tail_id.clone()));
            pairs.push((id, tail_id));
        }
        let tail_of = |id: Option<&str>| -> Option<String> {
            let id = id?;
            tail_by
                .iter()
                .find(|(s, _)| s == id)
                .map(|(_, t)| t.clone())
        };
        // Rows wholly after the cut join the tail hierarchy.
        for l in layers.iter_mut() {
            let id = values::text(l, "id").unwrap_or_default().to_owned();
            let is_after = tail_by.iter().any(|(s, t)| *s == id && *t == id);
            if !is_after {
                continue;
            }
            for field in ["group", "parent", "matteSource"] {
                let target = tail_of(values::text(l, field));
                if let Some(target) = target.filter(|x| Some(x.as_str()) != values::text(l, field))
                {
                    values::obj(l).insert(field, values::s(&target));
                }
            }
        }
        for (source, tail_id) in &pairs {
            let Some(src) = layer_of(&layers, source).cloned() else {
                continue;
            };
            let Some(tail) = layers
                .iter_mut()
                .find(|x| values::text(x, "id") == Some(tail_id.as_str()))
            else {
                continue;
            };
            let group = values::text(&src, "group").filter(|g| !g.is_empty());
            let tail_group = group.and_then(|g| {
                tail_by
                    .iter()
                    .find(|(s, t)| s == g && s != t)
                    .map(|(_, t)| t.clone())
            });
            let group_value = match (tail_group, group) {
                (Some(tg), _) => values::s(&tg),
                (None, Some(g)) if ids.contains(g) => Value::Null,
                (None, Some(g)) => values::s(g),
                (None, None) => Value::Null,
            };
            values::obj(tail).insert("group", group_value);
            let parent = values::text(&src, "parent").filter(|p| !p.is_empty());
            let tail_parent = parent
                .and_then(|p| {
                    tail_by
                        .iter()
                        .find(|(s, x)| s == p && s != x)
                        .map(|(_, x)| x.clone())
                })
                .or(parent.map(str::to_owned));
            values::obj(tail).insert(
                "parent",
                tail_parent.as_deref().map(values::s).unwrap_or(Value::Null),
            );
            if let Some(m) = values::text(&src, "matteSource") {
                let tm = tail_by
                    .iter()
                    .find(|(s, x)| s == m && s != x)
                    .map(|(_, x)| x.clone())
                    .unwrap_or_else(|| m.to_owned());
                values::obj(tail).insert("matteSource", values::s(&tm));
            }
        }
    }
    for id in &ordinary {
        let Some(index) = layers
            .iter()
            .position(|x| values::text(x, "id") == Some(id.as_str()))
        else {
            continue;
        };
        let tail_id = tail_id_for(id).unwrap_or_else(|| new_id("L"));
        let siblings = layers.clone();
        let tail = split_pair(&mut layers[index], &tail_id, &siblings, t);
        layers.insert(index, tail);
        changed = true;
    }
    if !changed {
        return Ok(());
    }
    let next = layers::normalize_group_stack(layers);
    layers::write_layers(cx, comp.as_deref(), next)
}
