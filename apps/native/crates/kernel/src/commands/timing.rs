//! Layer timing: strip moves, trims, `[`/`]`, slip, split and stagger.
//!
//! Ports the timeline's strip gestures (`extensions/timeline/timeline.ts`),
//! its split command (`extensions/timeline/index.ts`: Cmd+Shift+D / Cmd+B),
//! the layer-timing shortcuts of `extensions/keymap-default/keymap.ts`, the
//! quick stagger (Cmd+Option drag), and the agent's video edits
//! (`legacy/assistant/video-editing.ts`: split, trim, slip), the inspector's
//! Time buttons (`extensions/inspector/retiming.ts`: time remap, freeze
//! frame, reverse), and the keyboard edits editors expect from Premiere,
//! Final Cut and After Effects: ripple trim, roll, slide, lift and extract.
//!
//! Key times are layer-local, so moving a layer moves its animation with it.
//! Group strips act on their members; caption cues stay put in composition
//! time. The ripple, roll, slide and range edits keep what remains of a layer
//! where it was in composition time (its media and its keys), as a cut does.

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
    /// Ripple trim to `time` (Premiere Q/W): each layer whose span holds
    /// `time` loses the part before it (`In`) or after it (`Out`), and the
    /// time removed closes up; a `time` beyond the edge lengthens the layer
    /// (its start stays) and later layers move on by the time gained. What remains of a layer keeps its media and
    /// keys; every later unlocked layer moves back by the removed time that
    /// ends at or before its start; a layer that overlaps a removed part
    /// stays. Groups stand for their members; locked layers are skipped.
    /// Label "Ripple Trim".
    RippleTrim {
        layer_ids: Vec<String>,
        edge: TimeEdge,
        time: f64,
    },
    /// Roll the edit at a layer's `edge` to `time` (Premiere E, Final Cut
    /// Shift-X): the layer's edge moves, and the layer that abuts it on the
    /// same track and group takes the other side, so nothing after moves.
    /// Without a neighbour it is a trim. Clamped so each side keeps a frame
    /// and media keep their source start. Label "Roll Edit".
    Roll {
        layer_id: String,
        edge: TimeEdge,
        time: f64,
    },
    /// Slide by `delta` seconds (Premiere Option-comma/period): the layer
    /// moves with its keys and media; the layer ending where it starts grows
    /// or shrinks, the one starting where it ends gives or takes its head.
    /// Clamped at the neighbours' limits; refuses when it cannot move.
    /// Label "Slide Layer".
    Slide { layer_id: String, delta: f64 },
    /// Lift (`close: false`) or Extract (`close: true`) `start..end`
    /// (After Effects Edit ▸ Lift/Extract Work Area): the layers' parts
    /// inside the range go (a layer across the whole range is split, its
    /// tail a fresh layer above it); Extract moves those layers' later
    /// parts back by the range's length. Groups stand for their members;
    /// locked layers are skipped. Label "Lift Work Area" / "Extract Work
    /// Area".
    RemoveRange {
        layer_ids: Vec<String>,
        start: f64,
        end: f64,
        close: bool,
    },
    /// Time remapping (`enableTimeRemap`): video and pre-comp layers get
    /// `d.timeRemap` on and a `d.sourceTime` channel from their source time
    /// at the layer's ends (`Remap`), the reverse (`Reverse`), or one hold
    /// key of the source time at `time` (`Freeze`). Labels "Enable Time
    /// Remapping", "Time-Reverse Layer", "Freeze Frame".
    Retime {
        layer_ids: Vec<String>,
        mode: RetimeMode,
        time: f64,
    },
}

/// Which end of a layer an edit acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeEdge {
    In,
    Out,
}

/// The inspector's three Time buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetimeMode {
    Remap,
    Freeze,
    Reverse,
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
            TimingCommand::RippleTrim { .. } => "Ripple Trim",
            TimingCommand::Roll { .. } => "Roll Edit",
            TimingCommand::Slide { .. } => "Slide Layer",
            TimingCommand::RemoveRange { close: false, .. } => "Lift Work Area",
            TimingCommand::RemoveRange { close: true, .. } => "Extract Work Area",
            TimingCommand::Retime { mode, .. } => match mode {
                RetimeMode::Remap => "Enable Time Remapping",
                RetimeMode::Freeze => "Freeze Frame",
                RetimeMode::Reverse => "Time-Reverse Layer",
            },
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
            TimingCommand::RippleTrim {
                layer_ids,
                edge,
                time,
            } => ripple_trim(cx, layer_ids, *edge, *time),
            TimingCommand::Roll {
                layer_id,
                edge,
                time,
            } => roll(cx, layer_id, *edge, *time),
            TimingCommand::Slide { layer_id, delta } => slide(cx, layer_id, *delta),
            TimingCommand::RemoveRange {
                layer_ids,
                start,
                end,
                close,
            } => remove_range(cx, layer_ids, *start, *end, *close),
            TimingCommand::Retime {
                layer_ids,
                mode,
                time,
            } => retime(cx, layer_ids, *mode, *time),
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
pub(super) fn start_patch(layer: &mut Value, source: &Value, next_from: f64) {
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

/// `layer` (a copy of `source`) from `t` on: it starts at `t`, keeps its
/// end, and shows what `source` showed there (its keys move back by the
/// offset, `rebaseTailAnimation`; media continue, `startPatch`). `t` may lie
/// before `source`'s start, which uncovers more of its head.
fn cut_head(source: &Value, mut layer: Value, t: f64) -> Value {
    let from = values::num(source, "from").unwrap_or(0.0);
    let end = from + values::num(source, "dur").unwrap_or(0.0);
    let offset = t - from;
    {
        let m = values::obj(&mut layer);
        m.insert("from", Value::Number(t));
        m.insert("dur", Value::Number(end - t));
    }
    if offset.is_finite() && offset != 0.0 {
        for (_, path) in channels::all(&layer) {
            if let Some(keys) = channels::get_mut(&mut layer, &path)
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
    start_patch(&mut layer, source, t);
    layer
}

/// `splitLayerClone(PM, layer, tail, time)`: edits the head in place and
/// returns the tail.
fn split_pair(head: &mut Value, tail_id: &str, siblings: &[Value], t: f64) -> Value {
    let source = head.clone();
    let mut tail = cut_head(&source, layers::clone_layer(&source, tail_id, siblings), t);
    let from = values::num(&source, "from").unwrap_or(0.0);
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

/// Edits closer than this meet (abutting layers, a playhead on an edge).
const EDGE_EPS: f64 = 1e-6;

fn ends(layer: &Value) -> (f64, f64) {
    let from = values::num(layer, "from").unwrap_or(0.0);
    (from, from + values::num(layer, "dur").unwrap_or(0.0))
}

fn locked_here(layers: &[Value], layer: &Value) -> bool {
    values::flag(layer, "lock")
        || layers::ancestors(layers, layer)
            .iter()
            .any(|g| values::flag(g, "lock"))
}

/// The unlocked, non-group layers `ids` name (a group standing for its
/// members), in stack order, and their composition. All must share one.
fn leaves(cx: &Cx<'_>, ids: &[String]) -> Result<(Option<String>, Vec<Value>), CommandError> {
    let Some(first) = ids.first() else {
        return Ok((None, vec![]));
    };
    let comp = cx.find(first)?.comp_id;
    for id in ids {
        if cx.find(id)?.comp_id != comp {
            return Err(CommandError::Invalid(
                "Choose layers of one composition.".into(),
            ));
        }
    }
    let layers = cx.layers(comp.as_deref());
    let named: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let wanted = layers::expand_groups(&layers, &named);
    let picked = layers
        .iter()
        .filter(|l| {
            values::text(l, "id").is_some_and(|id| wanted.iter().any(|w| w == id))
                && values::text(l, "type") != Some("group")
                && !locked_here(&layers, l)
        })
        .cloned()
        .collect();
    Ok((comp, picked))
}

/// The unlocked layer on `layer`'s track and group whose other edge meets
/// `at`: the one ending there (`before`) or starting there.
fn abutting(layers: &[Value], layer: &Value, at: f64, before: bool) -> Option<Value> {
    let track = |l: &Value| values::num(l, "track").unwrap_or(0.0);
    layers
        .iter()
        .find(|l| {
            let (from, end) = ends(l);
            values::text(l, "id") != values::text(layer, "id")
                && values::text(l, "type") != Some("group")
                && values::text(l, "group") == values::text(layer, "group")
                && track(l) == track(layer)
                && !locked_here(layers, l)
                && ((if before { end } else { from }) - at).abs() < EDGE_EPS
        })
        .cloned()
}

fn put(cx: &mut Cx<'_>, layer: Value) -> Result<(), CommandError> {
    let id = values::text(&layer, "id").unwrap_or_default().to_owned();
    let loc = cx.find(&id)?;
    cx.put_layer(&loc, layer)
}

fn with_dur(mut layer: Value, dur: f64) -> Value {
    values::obj(&mut layer).insert("dur", Value::Number(dur));
    layer
}

/// The earliest a layer's In may reach: its media's first frame.
fn first_start(layer: &Value) -> f64 {
    if is_timed(layer) {
        earliest_start(layer)
    } else {
        f64::NEG_INFINITY
    }
}

fn ripple_trim(
    cx: &mut Cx<'_>,
    ids: &[String],
    edge: TimeEdge,
    time: f64,
) -> Result<(), CommandError> {
    let time = values::finite(time, "time")?;
    let (comp, targets) = leaves(cx, ids)?;
    let frame = 1.0 / cx.fps(comp.as_deref());
    let mut removed: Vec<(f64, f64)> = vec![];
    // An edge dragged outward: (the layer's old end, the time it gained).
    let mut grown: Vec<(String, f64, f64)> = vec![];
    for layer in targets {
        let (from, end) = ends(&layer);
        let id = values::text(&layer, "id").unwrap_or_default().to_owned();
        match edge {
            TimeEdge::In if time < from - EDGE_EPS => {
                // More of the head shows; the layer keeps its start.
                let t = time.max(first_start(&layer));
                if from - t > EDGE_EPS {
                    let mut longer = cut_head(&layer, layer.clone(), t);
                    values::obj(&mut longer).insert("from", Value::Number(from));
                    put(cx, longer)?;
                    grown.push((id, end, from - t));
                }
            }
            TimeEdge::Out if time > end + EDGE_EPS => {
                put(cx, with_dur(layer, time - from))?;
                grown.push((id, end, time - end));
            }
            _ if time <= from + EDGE_EPS || time >= end - EDGE_EPS => {}
            TimeEdge::In => {
                let t = time.min(end - frame);
                removed.push((from, t));
                put(cx, cut_head(&layer, layer.clone(), t))?;
            }
            TimeEdge::Out => {
                let t = time.max(from + frame);
                removed.push((t, end));
                put(cx, with_dur(layer, t - from))?;
            }
        }
    }
    if removed.is_empty() && grown.is_empty() {
        return Err(CommandError::Invalid(
            "Put the playhead inside a selected layer to ripple trim it.".into(),
        ));
    }
    if !grown.is_empty() {
        // Later unlocked layers move on by the time gained before them.
        let layers = cx.layers(comp.as_deref());
        for layer in &layers {
            let id = values::text(layer, "id").unwrap_or_default();
            if values::text(layer, "type") == Some("group")
                || locked_here(&layers, layer)
                || grown.iter().any(|(g, ..)| g == id)
            {
                continue;
            }
            let (from, _) = ends(layer);
            let shift: f64 = grown
                .iter()
                .filter(|(_, pivot, _)| from >= pivot - EDGE_EPS)
                .map(|(.., gained)| gained)
                .sum();
            if shift > 0.0 {
                let mut path = cx.find(id)?.path();
                path.push("from".into());
                cx.set(path, Value::Number(from + shift))?;
            }
        }
    }
    close_up(cx, comp.as_deref(), removed)
}

/// Every unlocked layer moves back by the removed time that ends at or
/// before its start (overlapping removed spans count once).
fn close_up(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    mut removed: Vec<(f64, f64)>,
) -> Result<(), CommandError> {
    removed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut spans: Vec<(f64, f64)> = vec![];
    for (start, end) in removed {
        match spans.last_mut() {
            Some(last) if start <= last.1 + EDGE_EPS => last.1 = last.1.max(end),
            _ => spans.push((start, end)),
        }
    }
    let layers = cx.layers(comp);
    for layer in &layers {
        if values::text(layer, "type") == Some("group") || locked_here(&layers, layer) {
            continue;
        }
        let (from, _) = ends(layer);
        let shift: f64 = spans
            .iter()
            .filter(|(_, end)| from >= end - EDGE_EPS)
            .map(|(start, end)| end - start)
            .sum();
        if shift > 0.0 {
            let id = values::text(layer, "id").unwrap_or_default();
            let mut path = cx.find(id)?.path();
            path.push("from".into());
            cx.set(path, Value::Number(from - shift))?;
        }
    }
    Ok(())
}

fn editable(cx: &Cx<'_>, id: &str) -> Result<(Option<String>, Value, Vec<Value>), CommandError> {
    let (loc, layer) = cx.layer(id)?;
    cx.check_lock(&loc, id)?;
    if values::text(&layer, "type") == Some("group") {
        return Err(CommandError::Invalid(
            "Choose a layer inside the group.".into(),
        ));
    }
    let layers = cx.layers(loc.comp_id.as_deref());
    Ok((loc.comp_id, layer, layers))
}

fn roll(cx: &mut Cx<'_>, id: &str, edge: TimeEdge, time: f64) -> Result<(), CommandError> {
    let time = values::finite(time, "time")?;
    let (comp, layer, layers) = editable(cx, id)?;
    let frame = 1.0 / cx.fps(comp.as_deref());
    let (from, end) = ends(&layer);
    match edge {
        TimeEdge::Out => {
            let next = abutting(&layers, &layer, end, false);
            let (mut lo, mut hi) = (from + frame, f64::INFINITY);
            if let Some(next) = &next {
                hi = ends(next).1 - frame;
                lo = lo.max(first_start(next));
            }
            let t = time.clamp(lo, hi.max(lo));
            put(cx, with_dur(layer, t - from))?;
            if let Some(next) = next {
                put(cx, cut_head(&next, next.clone(), t))?;
            }
        }
        TimeEdge::In => {
            let previous = abutting(&layers, &layer, from, true);
            let (mut lo, hi) = (first_start(&layer), end - frame);
            if let Some(previous) = &previous {
                lo = lo.max(ends(previous).0 + frame);
            }
            let t = time.clamp(lo.min(hi), hi);
            put(cx, cut_head(&layer, layer.clone(), t))?;
            if let Some(previous) = previous {
                let start = ends(&previous).0;
                put(cx, with_dur(previous, t - start))?;
            }
        }
    }
    Ok(())
}

fn slide(cx: &mut Cx<'_>, id: &str, delta: f64) -> Result<(), CommandError> {
    let delta = values::finite(delta, "delta")?;
    let (comp, layer, layers) = editable(cx, id)?;
    let frame = 1.0 / cx.fps(comp.as_deref());
    let (from, end) = ends(&layer);
    let previous = abutting(&layers, &layer, from, true);
    let next = abutting(&layers, &layer, end, false);
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    if let Some(p) = &previous {
        lo = lo.max(ends(p).0 + frame - from);
    }
    if let Some(n) = &next {
        hi = hi.min(ends(n).1 - frame - end);
        lo = lo.max(first_start(n) - end);
    }
    let delta = delta.clamp(lo.min(0.0), hi.max(0.0));
    if delta.abs() < EDGE_EPS {
        return Err(CommandError::Invalid(
            "The layer cannot slide further.".into(),
        ));
    }
    let mut moved = layer;
    values::obj(&mut moved).insert("from", Value::Number(from + delta));
    put(cx, moved)?;
    if let Some(p) = previous {
        let (start, _) = ends(&p);
        put(cx, with_dur(p, from + delta - start))?;
    }
    if let Some(n) = next {
        put(cx, cut_head(&n, n.clone(), end + delta))?;
    }
    Ok(())
}

/// A new cut at a layer's end: as split leaves the head.
fn cut_tail(mut layer: Value, t: f64) -> Value {
    let (from, _) = ends(&layer);
    values::obj(&mut layer).insert("transitionOut", Value::Null);
    if values::text(&layer, "type") == Some("audio") {
        set_content(&mut layer, "fadeOut", Value::Number(0.0));
    }
    with_dur(layer, t - from)
}

fn remove_range(
    cx: &mut Cx<'_>,
    ids: &[String],
    start: f64,
    end: f64,
    close: bool,
) -> Result<(), CommandError> {
    let (start, end) = (values::finite(start, "start")?, values::finite(end, "end")?);
    if end - start < EDGE_EPS {
        return Err(CommandError::Invalid("Set a work area first.".into()));
    }
    let (_, targets) = leaves(cx, ids)?;
    // A layer across the whole range is split at its end first: its tail
    // is a fresh layer, and the head then only crosses the start.
    let mut chosen: Vec<String> = vec![];
    for layer in &targets {
        let id = values::text(layer, "id").unwrap_or_default().to_owned();
        let (from, stop) = ends(layer);
        if from < start - EDGE_EPS && stop > end + EDGE_EPS {
            let tail = new_id("L");
            split(
                cx,
                std::slice::from_ref(&id),
                end,
                std::slice::from_ref(&tail),
            )?;
            chosen.push(tail);
        }
        chosen.push(id);
    }
    let gap = end - start;
    let mut gone: Vec<String> = vec![];
    let mut changed = false;
    for id in &chosen {
        let (_, layer) = cx.layer(id)?;
        let (from, stop) = ends(&layer);
        if stop <= start + EDGE_EPS {
            continue;
        }
        if from >= end - EDGE_EPS {
            if close {
                let mut moved = layer;
                values::obj(&mut moved).insert("from", Value::Number(from - gap));
                put(cx, moved)?;
                changed = true;
            }
            continue;
        }
        changed = true;
        if from >= start - EDGE_EPS && stop <= end + EDGE_EPS {
            gone.push(id.clone());
        } else if from < start {
            put(cx, cut_tail(layer, start))?;
        } else {
            let mut tail = cut_head(&layer, layer.clone(), end);
            values::obj(&mut tail).insert("transitionIn", Value::Null);
            if values::text(&tail, "type") == Some("audio") {
                set_content(&mut tail, "fadeIn", Value::Number(0.0));
            }
            if close {
                values::obj(&mut tail).insert("from", Value::Number(start));
            }
            put(cx, tail)?;
        }
    }
    if !changed {
        return Err(CommandError::Invalid(
            "No chosen layer is inside the work area.".into(),
        ));
    }
    if !gone.is_empty() {
        super::LayerCommand::Delete { layer_ids: gone }.plan(cx)?;
    }
    Ok(())
}

/// A linear key, as `setKeyOn(.., 'linear', fps)` makes one.
fn linear_key(t: f64, v: f64, hold: bool) -> Value {
    let mut key = Map::new();
    key.insert("i", values::s(&new_id("k")));
    key.insert("t", Value::Number(t));
    key.insert("v", Value::Number(v));
    key.insert(
        "eo",
        Value::Array(vec![Value::Number(0.0), Value::Number(0.0)]),
    );
    key.insert(
        "ei",
        Value::Array(vec![Value::Number(1.0), Value::Number(1.0)]),
    );
    if hold {
        key.insert("hold", Value::Bool(true));
    }
    Value::Object(key)
}

fn retime(
    cx: &mut Cx<'_>,
    ids: &[String],
    mode: RetimeMode,
    time: f64,
) -> Result<(), CommandError> {
    let time = values::finite(time, "time")?;
    for id in ids {
        let (loc, layer) = cx.layer(id)?;
        if !matches!(values::text(&layer, "type"), Some("video" | "precomp")) {
            return Err(CommandError::Invalid(
                "Time remapping needs a video or pre-comp layer.".into(),
            ));
        }
        cx.check_lock(&loc, id)?;
        let fps = cx.fps(loc.comp_id.as_deref());
        let frame = 1.0 / fps;
        let (from, end) = ends(&layer);
        let dur = end - from;
        let (start, last, now) = source_times(
            cx,
            loc.comp_id.as_deref(),
            &layer,
            [from, from + (dur - frame).max(0.0), time],
        )?;
        let keys = match mode {
            RetimeMode::Freeze => vec![linear_key(((time - from) * fps).round() / fps, now, true)],
            RetimeMode::Remap | RetimeMode::Reverse => {
                let (a, b) = if mode == RetimeMode::Reverse {
                    (last, start)
                } else {
                    (start, last)
                };
                vec![
                    linear_key(0.0, a, false),
                    linear_key((dur - frame).max(frame), b, false),
                ]
            }
        };
        let base = if mode == RetimeMode::Freeze {
            now
        } else {
            start
        };
        let mut source_time = values::property(Value::Number(base));
        values::obj(&mut source_time).insert("kf", Value::Array(keys));
        for (key, value) in [
            ("timeRemap", values::property(Value::Bool(true))),
            ("sourceTime", source_time),
        ] {
            let mut path = loc.path();
            path.extend(["d".into(), key.into()]);
            cx.set(path, value)?;
        }
    }
    Ok(())
}

/// `sourceTime(PM, layer, t)` at three times: the remapped channel when
/// remapping is on, else elapsed time through the (possibly animated) speed
/// plus the trim.
fn source_times(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    layer: &Value,
    at: [f64; 3],
) -> Result<(f64, f64, f64), CommandError> {
    let from = values::num(layer, "from").unwrap_or(0.0);
    let d = layer.get("d");
    let channel = |key: &str| d.and_then(|d| d.get(key));
    let remapped = channel("sourceTime").is_some_and(values::is_property);
    let varying = channel("speed").is_some_and(|s| {
        values::is_property(s)
            && (s
                .get("kf")
                .and_then(Value::as_array)
                .is_some_and(|k| !k.is_empty())
                || s.get("expr")
                    .and_then(Value::as_str)
                    .is_some_and(|e| !e.is_empty()))
    });
    let fps = cx.fps(comp);
    let node = pm_model::Node::from_value(layer.clone(), 0);
    let times = cx.kernel(comp, |edit| {
        edit.read_with(&node, |scope, layer| {
            let d = layer.get("d");
            let field = |key: &str| d.and_then(|d| d.get(key));
            let mut one = |t: f64| -> pm_eval::JsResult<f64> {
                if remapped
                    && scope
                        .evaluated(layer, field("timeRemap"), t, "c.timeRemap")?
                        .truthy()
                {
                    let v = scope
                        .evaluated(layer, field("sourceTime"), t, "c.sourceTime")?
                        .to_number();
                    return Ok(if v.is_nan() { 0.0 } else { v });
                }
                let trim = scope.evaluated(layer, field("trim"), t, "c.trim")?;
                let trim = if trim.truthy() { trim.to_number() } else { 0.0 };
                let local = t - from;
                if !varying {
                    let speed = scope.evaluated(layer, field("speed"), t, "c.speed")?;
                    let speed =
                        if matches!(speed, pm_eval::JsValue::Undefined | pm_eval::JsValue::Null) {
                            1.0
                        } else {
                            speed.to_number()
                        };
                    return Ok(local * speed + trim);
                }
                // Integrate a varying rate at midpoints of fixed steps.
                let step = 1.0 / (fps.max(30.0) * 2.0);
                let end = local.max(0.0);
                let full = (end / step).floor();
                let mut sum = 0.0;
                let mut i = 0.0;
                while i < full {
                    sum += scope
                        .evaluated(layer, field("speed"), from + (i + 0.5) * step, "c.speed")?
                        .to_number()
                        * step;
                    i += 1.0;
                }
                let rest = end - full * step;
                let tail = scope
                    .evaluated(
                        layer,
                        field("speed"),
                        from + full * step + rest / 2.0,
                        "c.speed",
                    )?
                    .to_number();
                Ok(sum + tail * rest + trim)
            };
            Ok((one(at[0])?, one(at[1])?, one(at[2])?))
        })
    })?;
    Ok(times)
}
