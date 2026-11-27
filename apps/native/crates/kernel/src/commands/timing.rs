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

use pm_model::{Document, Map, Node, Value};

use super::{Cx, channels, comps, layers, new_id, track, values};
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
    /// cues stay in place. Keys are re-based to keep their composition times
    /// unchanged. Applies to the listed unlocked layers, including groups.
    /// Send absolute times in a drag. Label "Trim Layer".
    TrimIn { layer_ids: Vec<String>, time: f64 },
    /// Drag the Out edge / Option+`]`: the layer ends at `time`, at least
    /// one frame long. Applies to the listed unlocked layers, including
    /// groups. Label "Trim Layer".
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
    /// A Ripple Trim, Roll or Slide made in the Tracks timeline, where a
    /// clip's track is the lane it shows on. `lanes` gives that lane for the
    /// layers of the levels the edit reaches (relative to their group, as a
    /// layer's stored `track` is): each unlocked one whose stored `track`
    /// differs is written first, so the edit's neighbours are the ones on
    /// its track and no clip changes track with it. A ripple then closes or
    /// opens time on the edited clips' own tracks only (Premiere without
    /// sync lock). Any other command runs as it is. Label: the command's.
    InTracks {
        lanes: Vec<(String, u32)>,
        command: Box<TimingCommand>,
    },
    /// Speed/Duration: a video or audio clip plays at `speed` (1 = normal,
    /// above 0.05) from the same source In; its length scales so it shows the
    /// same media (never past the media's end). With `ripple`, the later
    /// clips on its track move by the change, so no gap opens and nothing is
    /// covered (inside `InTracks`, its track is the lane it shows on). An
    /// animated speed or a remapped clip refuses. `keep_pitch` sets how its
    /// audio sounds at that speed (`d.pitch`: kept, or following the speed
    /// as tape would); `None` leaves it. Label "Change Speed".
    SetSpeed {
        layer_id: String,
        speed: f64,
        ripple: bool,
        keep_pitch: Option<bool>,
    },
    /// Writes each unlocked layer's lane in the Tracks timeline as its
    /// stored `track` where they differ (see `InTracks`), first in a
    /// transaction whose edit (a pro edit's drag) decides tracks from them.
    /// Label "Edit Clips" (it travels with the edit it serves).
    PinTracks { lanes: Vec<(String, u32)> },
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
            TimingCommand::InTracks { command, .. } => return command.label(),
            TimingCommand::PinTracks { .. } => "Edit Clips",
            TimingCommand::SetSpeed { .. } => "Change Speed",
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
            } => ripple_trim(cx, layer_ids, *edge, *time, false),
            TimingCommand::InTracks { lanes, command } => {
                pin_tracks(cx, lanes)?;
                let comp = lanes
                    .iter()
                    .find_map(|(id, _)| cx.find(id).ok())
                    .map(|loc| loc.comp_id);
                let before: std::collections::HashMap<String, (f64, f64)> = comp
                    .as_ref()
                    .map(|c| {
                        cx.layers(c.as_deref())
                            .iter()
                            .map(|l| {
                                (
                                    values::text(l, "id").unwrap_or_default().to_owned(),
                                    ends(l),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                match command.as_ref() {
                    TimingCommand::RippleTrim {
                        layer_ids,
                        edge,
                        time,
                    } => ripple_trim(cx, layer_ids, *edge, *time, true)?,
                    other => other.plan(cx)?,
                }
                match comp {
                    Some(comp) => stack_follows_tracks(cx, comp.as_deref(), &before),
                    None => Ok(()),
                }
            }
            TimingCommand::PinTracks { lanes } => pin_tracks(cx, lanes),
            TimingCommand::SetSpeed {
                layer_id,
                speed,
                ripple,
                keep_pitch,
            } => set_speed(cx, layer_id, *speed, *ripple, *keep_pitch),
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

/// Source-backed layers use the same local-time + source-trim model.
/// Precomps must retain their nested composition time when the In edge moves.
fn is_timed(layer: &Value) -> bool {
    matches!(
        values::text(layer, "type"),
        Some("audio" | "video" | "precomp")
    )
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
    if matches!(values::text(layer, "type"), Some("video" | "precomp")) {
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

/// Preserve the source position at the cut, including animated playback speed.
fn preserve_source_time(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    source: &Value,
    next: &mut Value,
    at: f64,
) -> Result<(), CommandError> {
    if is_timed(source) {
        let (_, _, time) = source_times(cx, comp, source, [at; 3])?;
        // A remapped sourceTime channel already keeps its comp times via cut_head.
        if !source
            .get("d")
            .and_then(|d| d.get("timeRemap"))
            .is_some_and(|v| {
                if values::is_property(v) {
                    v.get("v").is_some_and(Value::is_truthy)
                } else {
                    v.is_truthy()
                }
            })
        {
            set_content(next, "trim", Value::Number(time));
        }
    }
    Ok(())
}

/// A split precomp is independently editable. Reuse Duplicate Composition's
/// naming, layer/key ID renewal and internal link remapping in this transaction.
/// Media and precomps nested inside the source keep their existing references.
fn duplicate_precomp_source(
    cx: &mut Cx<'_>,
    source: &Value,
    tail: &mut Value,
) -> Result<(), CommandError> {
    if values::text(source, "type") == Some("precomp") {
        if let Some(id) = source.get("d").and_then(|d| values::text(d, "comp")) {
            let copy_id = new_id("C");
            comps::duplicate(cx, id, &copy_id)?;
            set_content(tail, "comp", values::s(&copy_id));
        }
    }
    Ok(())
}

/// Apply one strip-trim sample in a private scratch document. Each command
/// sees the earlier writes, including track pinning and legacy group spans.
pub fn preview_strip_trim(
    root: &Node,
    revision: u64,
    origin: super::Origin,
    commands: &[super::Command],
) -> Result<Document, CommandError> {
    let mut cx = Cx::new_at(root, origin, revision);
    for command in commands {
        match command {
            super::Command::Timing(
                timing @ (TimingCommand::TrimIn { .. }
                    | TimingCommand::TrimOut { .. }
                    | TimingCommand::PinTracks { .. }),
            ) => timing.plan(&mut cx)?,
            _ => return Err(CommandError::Invalid("Expected a strip trim.".into())),
        }
    }
    Ok(cx.into_document())
}

/// Trim only the requested layers at this time, skipping locked layers.
fn trim(cx: &mut Cx<'_>, layer_ids: &[String], time: f64, start: bool) -> Result<(), CommandError> {
    let time = values::finite(time, "time")?;
    let locations = (layer_ids.len() > 1).then(|| crate::model::find_layers(cx.root(), layer_ids));
    let mut targets = Vec::with_capacity(layer_ids.len());
    for id in layer_ids {
        let loc = match &locations {
            Some(locations) => locations.get(id.as_str()).cloned()
                .ok_or_else(|| CommandError::LayerNotFound(id.clone()))?,
            None => cx.find(id)?,
        };
        if cx.check_lock(&loc, id).is_err() {
            continue;
        }
        let layer = cx.layer_node(&loc)?.to_value();
        let stored_from = values::num(&layer, "from").unwrap_or(0.0);
        let stored_dur = values::num(&layer, "dur").unwrap_or(0.0);
        let out_of_stored = stored_dur <= 0.
            || (start && time >= stored_from + stored_dur)
            || (!start && time <= stored_from);
        let (from, dur) = if values::text(&layer, "type") == Some("group") && out_of_stored {
            let comp = loc.comp_id.as_deref();
            member_span(&cx.layers(comp), &layer).unwrap_or_else(|| {
                if stored_dur > 0. {
                    (stored_from, stored_dur)
                } else {
                    (0.0, cx.duration(comp))
                }
            })
        } else {
            (stored_from, stored_dur)
        };
        targets.push((loc, layer, from, dur));
    }
    for (loc, layer, from, dur) in targets {
        let fps = cx.fps(loc.comp_id.as_deref());
        if (start && time >= from + dur) || (!start && time <= from) {
            return Err(CommandError::Invalid(
                "Trim refused: the playhead must leave at least one frame in the clip.".into(),
            ));
        }
        if start {
            let mut nf = time.min(from + dur - 1.0 / fps);
            if is_timed(&layer) {
                nf = nf.max(earliest_start(&layer));
            }
            let mut next = cut_head(&layer, layer.clone(), nf);
            values::obj(&mut next).insert("dur", Value::Number(from + dur - nf));
            preserve_source_time(cx, loc.comp_id.as_deref(), &layer, &mut next, nf)?;
            cx.put_layer(&loc, next)?;
        } else {
            // Never past the media's end at the clip's speed.
            let longest = media_limit(cx, &layer).unwrap_or(f64::INFINITY);
            let duration = Value::Number((time - from).min(longest).max(1.0 / fps));
            if values::text(&layer, "type") == Some("group")
                && (values::num(&layer, "dur").unwrap_or(0.) <= 0.
                    || values::num(&layer, "from").unwrap_or(0.) != from)
            {
                let mut next = cut_head(&layer, layer.clone(), from);
                values::obj(&mut next).insert("dur", duration);
                cx.put_layer(&loc, next)?;
            } else {
                let mut path = loc.path();
                path.push("dur".into());
                cx.set(path, duration)?;
            }
        }
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

/// Extent of a group's non-group descendants, or `None` when the group is empty.
fn member_span(layers: &[Value], layer: &Value) -> Option<(f64, f64)> {
    let one: HashSet<&str> = values::text(layer, "id").into_iter().collect();
    let ids = layers::expand_groups(layers, &one);
    let children: Vec<&Value> = ids
        .iter()
        .filter_map(|id| layer_of(layers, id))
        .filter(|l| values::text(l, "type") != Some("group"))
        .collect();
    if children.is_empty() {
        return None;
    }
    let from = children
        .iter()
        .map(|l| values::num(l, "from").unwrap_or(0.0))
        .fold(f64::INFINITY, f64::min);
    let end = children
        .iter()
        .map(|l| values::num(l, "from").unwrap_or(0.0) + values::num(l, "dur").unwrap_or(0.0))
        .fold(f64::NEG_INFINITY, f64::max);
    Some((from, end - from))
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
fn split_pair(head: &mut Value, tail_id: &str, sibling_names: &[String], t: f64) -> Value {
    let mut tail = cut_head(
        head,
        layers::clone_layer_with_names(head, tail_id, sibling_names),
        t,
    );
    let from = values::num(head, "from").unwrap_or(0.0);
    if values::text(head, "type") == Some("captions")
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
    if values::text(head, "type") == Some("audio") {
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
    use std::collections::HashMap;
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
    let mut index_by_id: HashMap<String, usize> = HashMap::with_capacity(layers.len() * 2);
    for (i, l) in layers.iter().enumerate() {
        if let Some(id) = values::text(l, "id") {
            index_by_id.insert(id.to_owned(), i);
        }
    }
    let is_locked = |layers: &[Value],
                     index_by_id: &HashMap<String, usize>,
                     l: &Value,
                     except: &HashSet<String>| {
        if values::flag(l, "lock") {
            return true;
        }
        let mut cur = values::text(l, "group").filter(|g| !g.is_empty());
        let mut seen = HashSet::new();
        while let Some(gid) = cur {
            if !seen.insert(gid) {
                break;
            }
            let Some(&gi) = index_by_id.get(gid) else {
                break;
            };
            let g = &layers[gi];
            if values::flag(g, "lock") && !except.contains(gid) {
                return true;
            }
            cur = values::text(g, "group").filter(|p| !p.is_empty());
        }
        false
    };
    let has_explicit_ancestor =
        |layers: &[Value], index_by_id: &HashMap<String, usize>, l: &Value| {
            let mut cur = values::text(l, "group").filter(|g| !g.is_empty());
            let mut seen = HashSet::new();
            while let Some(gid) = cur {
                if !seen.insert(gid) {
                    break;
                }
                if explicit.contains(gid) {
                    return true;
                }
                let Some(&gi) = index_by_id.get(gid) else {
                    break;
                };
                cur = values::text(&layers[gi], "group").filter(|p| !p.is_empty());
            }
            false
        };
    let tail_id_map: HashMap<&str, &str> = layer_ids
        .iter()
        .zip(new_ids.iter())
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let tail_id_for = |id: &str| tail_id_map.get(id).map(|s| (*s).to_owned());

    // Precompute group spans once from the initial layer tree so large groups
    // never rescan `layers` per child.
    let mut group_children: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, l) in layers.iter().enumerate() {
        if let Some(gid) = values::text(l, "group").filter(|g| !g.is_empty()) {
            group_children.entry(gid.to_owned()).or_default().push(i);
        }
    }
    let compute_span = |layers: &[Value],
                        group_children: &HashMap<String, Vec<usize>>,
                        layer: &Value|
     -> (f64, f64) {
        if values::text(layer, "type") != Some("group") {
            return (
                values::num(layer, "from").unwrap_or(f64::NAN),
                values::num(layer, "dur").unwrap_or(f64::NAN),
            );
        }
        let mut stack = vec![values::text(layer, "id").unwrap_or_default()];
        let mut seen = HashSet::new();
        let mut from = f64::INFINITY;
        let mut end = f64::NEG_INFINITY;
        let mut any = false;
        while let Some(gid) = stack.pop() {
            if !seen.insert(gid) {
                continue;
            }
            let Some(children) = group_children.get(gid) else {
                continue;
            };
            for &ci in children {
                let child = &layers[ci];
                if values::text(child, "type") == Some("group") {
                    if let Some(cid) = values::text(child, "id") {
                        stack.push(cid);
                    }
                } else {
                    let f = values::num(child, "from").unwrap_or(0.0);
                    let d = values::num(child, "dur").unwrap_or(0.0);
                    from = from.min(f);
                    end = end.max(f + d);
                    any = true;
                }
            }
        }
        if !any {
            (0.0, comp_dur)
        } else {
            (from, end - from)
        }
    };

    // Group roots: explicit groups not inside another explicit group.
    let roots: Vec<String> = layers
        .iter()
        .filter(|l| {
            values::text(l, "id").is_some_and(|i| explicit.contains(i))
                && values::text(l, "type") == Some("group")
                && !has_explicit_ancestor(&layers, &index_by_id, l)
        })
        .filter_map(|l| values::text(l, "id").map(str::to_owned))
        .collect();
    let expand_group_fast = |root: &str,
                             layers: &[Value],
                             group_children: &HashMap<String, Vec<usize>>|
     -> HashSet<String> {
        let mut ids = HashSet::new();
        let mut stack = vec![root.to_owned()];
        while let Some(id) = stack.pop() {
            if !ids.insert(id.clone()) {
                continue;
            }
            if let Some(children) = group_children.get(&id) {
                for &ci in children {
                    if let Some(cid) = values::text(&layers[ci], "id") {
                        stack.push(cid.to_owned());
                    }
                }
            }
        }
        ids
    };
    let mut grouped: HashSet<String> = HashSet::new();
    let mut root_sets: Vec<(String, HashSet<String>)> = Vec::with_capacity(roots.len());
    for r in &roots {
        let set = expand_group_fast(r, &layers, &group_children);
        grouped.extend(set.iter().cloned());
        root_sets.push((r.clone(), set));
    }
    let ordinary: Vec<String> = layers
        .iter()
        .filter(|l| {
            let id = values::text(l, "id").unwrap_or_default();
            explicit.contains(id)
                && values::text(l, "type") != Some("group")
                && !grouped.contains(id)
                && !values::flag(l, "lock")
                && crosses(compute_span(&layers, &group_children, l), t)
        })
        .filter_map(|l| values::text(l, "id").map(str::to_owned))
        .collect();

    let mut sibling_names: Vec<String> = layers
        .iter()
        .map(|l| values::text(l, "name").unwrap_or_default().to_owned())
        .collect();
    let mut tails_before: HashMap<usize, Vec<Value>> = HashMap::new();
    let mut changed = false;
    let mut transition_tails = Vec::new();
    let mut root_rewires: Vec<(HashSet<String>, HashMap<String, String>, Vec<(String, String)>)> =
        Vec::new();

    for (root, ids) in root_sets {
        let hierarchy_indices: Vec<usize> = layers
            .iter()
            .enumerate()
            .filter_map(|(i, l)| {
                values::text(l, "id")
                    .is_some_and(|id| ids.contains(id))
                    .then_some(i)
            })
            .collect();
        let Some(&root_index) = index_by_id.get(&root) else {
            continue;
        };
        if !crosses(
            compute_span(&layers, &group_children, &layers[root_index]),
            t,
        ) || hierarchy_indices.is_empty()
            || hierarchy_indices
                .iter()
                .any(|&i| is_locked(&layers, &index_by_id, &layers[i], &ids))
        {
            continue;
        }
        changed = true;
        let mut tail_by: HashMap<String, String> = HashMap::with_capacity(hierarchy_indices.len());
        let mut pairs: Vec<(String, String)> = Vec::with_capacity(hierarchy_indices.len());
        for &index in &hierarchy_indices {
            let id = values::text(&layers[index], "id")
                .unwrap_or_default()
                .to_owned();
            let sp = compute_span(&layers, &group_children, &layers[index]);
            if !crosses(sp, t) {
                if sp.0 >= t {
                    tail_by.insert(id.clone(), id);
                }
                continue;
            }
            let tail_id = if id == root {
                tail_id_for(&id).unwrap_or_else(|| new_id("L"))
            } else {
                new_id("L")
            };
            let source = layers[index].clone();
            let mut tail = split_pair(&mut layers[index], &tail_id, &sibling_names, t);
            preserve_source_time(cx, comp.as_deref(), &source, &mut tail, t)?;
            duplicate_precomp_source(cx, &source, &mut tail)?;
            sibling_names.push(values::text(&tail, "name").unwrap_or_default().to_owned());
            tails_before.entry(index).or_default().push(tail);
            tail_by.insert(id.clone(), tail_id.clone());
            transition_tails.push((id.clone(), tail_id.clone()));
            pairs.push((id, tail_id));
        }
        root_rewires.push((ids, tail_by, pairs));
    }
    for id in &ordinary {
        let Some(&index) = index_by_id.get(id) else {
            continue;
        };
        let tail_id = tail_id_for(id).unwrap_or_else(|| new_id("L"));
        let source = layers[index].clone();
        let mut tail = split_pair(&mut layers[index], &tail_id, &sibling_names, t);
        preserve_source_time(cx, comp.as_deref(), &source, &mut tail, t)?;
        duplicate_precomp_source(cx, &source, &mut tail)?;
        sibling_names.push(values::text(&tail, "name").unwrap_or_default().to_owned());
        tails_before.entry(index).or_default().push(tail);
        transition_tails.push((id.clone(), tail_id));
        changed = true;
    }
    if !changed {
        return Ok(());
    }

    // Materialize the combined layer list in one O(N) pass instead of repeated Vec::insert.
    let extra: usize = tails_before.values().map(Vec::len).sum();
    let mut combined = Vec::with_capacity(layers.len() + extra);
    for (i, layer) in layers.into_iter().enumerate() {
        if let Some(tails) = tails_before.remove(&i) {
            // Multiple insertions at the same index previously pushed each new tail
            // at the front (`insert(index, tail)`), so reverse matches that order.
            for tail in tails.into_iter().rev() {
                combined.push(tail);
            }
        }
        combined.push(layer);
    }
    layers = combined;

    // Reindex once and apply group/parent/matte rewiring in O(N).
    index_by_id.clear();
    for (i, l) in layers.iter().enumerate() {
        if let Some(id) = values::text(l, "id") {
            index_by_id.insert(id.to_owned(), i);
        }
    }
    for (ids, tail_by, pairs) in root_rewires {
        // Rows wholly after the cut join the tail hierarchy.
        for (src_id, mapped_id) in &tail_by {
            if src_id != mapped_id {
                continue;
            }
            let Some(&idx) = index_by_id.get(src_id) else {
                continue;
            };
            let l = &mut layers[idx];
            for field in ["group", "parent", "matteSource"] {
                let target = values::text(l, field)
                    .and_then(|v| tail_by.get(v))
                    .cloned();
                if let Some(target) = target.filter(|x| Some(x.as_str()) != values::text(l, field))
                {
                    values::obj(l).insert(field, values::s(&target));
                }
            }
        }
        for (source, tail_id) in &pairs {
            let Some(&src_idx) = index_by_id.get(source) else {
                continue;
            };
            let Some(&tail_idx) = index_by_id.get(tail_id) else {
                continue;
            };
            let group = values::text(&layers[src_idx], "group")
                .filter(|g| !g.is_empty())
                .map(str::to_owned);
            let parent = values::text(&layers[src_idx], "parent")
                .filter(|p| !p.is_empty())
                .map(str::to_owned);
            let matte = values::text(&layers[src_idx], "matteSource").map(str::to_owned);

            let tail_group = group
                .as_deref()
                .and_then(|g| tail_by.get(g).filter(|t| t.as_str() != g).cloned());
            let group_value = match (tail_group, group.as_deref()) {
                (Some(tg), _) => values::s(&tg),
                (None, Some(g)) if ids.contains(g) => Value::Null,
                (None, Some(g)) => values::s(g),
                (None, None) => Value::Null,
            };
            let tail_parent = parent
                .as_deref()
                .and_then(|p| tail_by.get(p).filter(|x| x.as_str() != p).cloned())
                .or(parent);
            let tail_matte = matte.map(|m| {
                tail_by
                    .get(&m)
                    .filter(|x| x.as_str() != m)
                    .cloned()
                    .unwrap_or(m)
            });

            let tail = &mut layers[tail_idx];
            values::obj(tail).insert("group", group_value);
            values::obj(tail).insert(
                "parent",
                tail_parent.as_deref().map(values::s).unwrap_or(Value::Null),
            );
            if let Some(tm) = tail_matte {
                values::obj(tail).insert("matteSource", values::s(&tm));
            }
        }
    }
    super::transition_edges::after_split(&mut layers, &transition_tails)?;
    let next = layers::normalize_group_stack(layers);
    layers::write_layers(cx, comp.as_deref(), next)
}

/// Edits closer than this meet (abutting layers, a playhead on an edge).
const EDGE_EPS: f64 = 1e-6;
/// The track a ripple keeps to (its track id and index), or `None` when it
/// ripples across tracks.
type RippleTrack = Option<(Option<String>, f64)>;

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
                && area(l) == area(layer)
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

/// The playback rate of a clip's media (`speed`, audio's too), or `None`
/// when time remapping or an animated speed decides which frame shows.
fn media_rate(layer: &Value) -> Option<f64> {
    let d = layer.get("d")?;
    let remapped = d.get("timeRemap").is_some_and(|r| {
        if values::is_property(r) {
            r.get("v").is_some_and(Value::is_truthy) || !track::keys_of(r).is_empty()
        } else {
            r.is_truthy()
        }
    });
    let speed = d.get("speed");
    let animated = speed.filter(|s| values::is_property(s)).is_some_and(|s| {
        !track::keys_of(s).is_empty() || s.get("expr").is_some_and(Value::is_truthy)
    });
    if remapped || animated {
        return None;
    }
    Some(
        content_number(layer, "speed")
            .filter(|s| *s > 0.0 && s.is_finite())
            .unwrap_or(1.0),
    )
}

/// The longest a video or audio clip may run before its media ends, at its
/// speed from its source In; `None` without a known media length (or when
/// its frames come from remapping).
fn media_limit(cx: &Cx<'_>, layer: &Value) -> Option<f64> {
    if !is_timed(layer) {
        return None;
    }
    let rate = media_rate(layer)?;
    let asset = layer.get("d").and_then(|d| values::text(d, "asset"))?;
    let length = cx
        .root()
        .get("assets")
        .and_then(|a| a.get(asset))
        .and_then(|a| a.get("dur"))
        .and_then(crate::model::number_of)
        .filter(|n| n.is_finite() && *n > 0.0)?;
    Some(((length - trim_of(layer)) / rate).max(0.0))
}

/// Speed/Duration (see `TimingCommand::SetSpeed`).
fn set_speed(
    cx: &mut Cx<'_>,
    id: &str,
    speed: f64,
    ripple: bool,
    keep_pitch: Option<bool>,
) -> Result<(), CommandError> {
    let speed = values::finite(speed, "speed")?;
    if !(0.05..=100.0).contains(&speed) {
        return Err(CommandError::Invalid(
            "Choose a speed between 5% and 10000%.".into(),
        ));
    }
    let (comp, layer, _) = editable(cx, id)?;
    if !is_timed(&layer) {
        return Err(CommandError::Invalid(
            "Speed changes a video or audio clip.".into(),
        ));
    }
    let Some(previous) = media_rate(&layer) else {
        return Err(CommandError::Invalid(
            "The clip's speed is animated or remapped: change it in Properties.".into(),
        ));
    };
    let mut next = layer.clone();
    if let Some(keep) = keep_pitch {
        let pitch = if keep { "preserve" } else { "follow" };
        values::obj(&mut next)
            .get_mut("d")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| CommandError::Invalid("The clip has no source content.".into()))?
            .insert("pitch", Value::from(pitch));
    }
    if (previous - speed).abs() < 1e-12 {
        return put(cx, next);
    }
    let frame = 1.0 / cx.fps(comp.as_deref());
    let (from, end) = ends(&layer);
    set_content(&mut next, "speed", Value::Number(speed));
    let mut dur = ((end - from) * previous / speed).max(frame);
    if let Some(limit) = media_limit(cx, &next) {
        dur = dur.min(limit.max(frame));
    }
    let track = track_of(&layer);
    put(cx, with_dur(next, dur))?;
    if ripple {
        let delta = from + dur - end;
        let layers = cx.layers(comp.as_deref());
        for other in &layers {
            let oid = values::text(other, "id").unwrap_or_default();
            if oid == id
                || values::text(other, "type") == Some("group")
                || locked_here(&layers, other)
                || track_of(other) != track
            {
                continue;
            }
            let (ofrom, _) = ends(other);
            if ofrom >= end - EDGE_EPS {
                let mut path = cx.find(oid)?.path();
                path.push("from".into());
                cx.set(path, Value::Number(ofrom + delta))?;
            }
        }
    }
    Ok(())
}

/// The Tracks timeline area a layer's lanes count in: audio, captions or
/// video (each numbers its lanes from 0).
fn area(layer: &Value) -> &'static str {
    match values::text(layer, "type") {
        Some("audio") => "audio",
        Some("captions") => "captions",
        _ => "video",
    }
}

/// A layer's track key: its group, area and stored `track` (Tracks
/// timeline): V1, A1 and the first caption lane are different tracks.
fn track_of(layer: &Value) -> (Option<String>, f64) {
    (
        values::text(layer, "group")
            .map(|g| format!("{g}\u{1f}{}", area(layer)))
            .or_else(|| Some(area(layer).to_owned())),
        values::num(layer, "track").unwrap_or(0.0),
    )
}

/// Writes each unlocked layer's lane as its stored `track` where they differ.
/// The stack is never touched here: a trim, roll, slide or slip changes no
/// layer's place in it (the timeline's view state holds the lanes it shows).
fn pin_tracks(cx: &mut Cx<'_>, lanes: &[(String, u32)]) -> Result<(), CommandError> {
    for (id, lane) in lanes {
        let Ok(loc) = cx.find(id) else { continue };
        let Ok((_, layer)) = cx.layer(id) else {
            continue;
        };
        let lane = f64::from(*lane);
        if values::num(&layer, "track").unwrap_or(0.0) == lane || cx.check_lock(&loc, id).is_err() {
            continue;
        }
        let mut path = loc.path();
        path.push("track".into());
        cx.set(path, Value::Number(lane))?;
    }
    Ok(())
}

/// In Tracks the track decides stacking (PM-512). After a Tracks edit, a
/// clip it retimed takes the lowest free track from its own over its new
/// time, as the timeline packs it; then the clips it overlaps on other tracks
/// of its area and group (and theirs, onward) are restacked as little as they
/// need, so a clip on a higher track is drawn above every clip it overlaps on
/// a lower one. No other clip changes track and every other layer keeps its
/// place. A restack that would move or pass a locked, 3D, group or track
/// matte layer, or a layer of another area or group, is left out (as a
/// Tracks move is).
fn stack_follows_tracks(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    before: &std::collections::HashMap<String, (f64, f64)>,
) -> Result<(), CommandError> {
    let mut layers = cx.layers(comp);
    let id = |l: &Value| values::text(l, "id").unwrap_or_default().to_owned();
    let group = |l: &Value| values::text(l, "type") == Some("group");
    let key = |l: &Value| (values::text(l, "group").map(str::to_owned), area(l));
    let track = |l: &Value| values::num(l, "track").unwrap_or(0.0).max(0.0).floor();
    let overlap = |a: (f64, f64), b: (f64, f64)| a.0 < b.1 - 1e-5 && b.0 < a.1 - 1e-5;
    let edited: Vec<usize> = (0..layers.len())
        .filter(|&i| !group(&layers[i]) && before.get(&id(&layers[i])) != Some(&ends(&layers[i])))
        .collect();
    if edited.is_empty() {
        return Ok(());
    }
    let mut changed = false;
    // The timeline places a retimed clip after the clips that keep their
    // time, from the bottom of the stack up.
    for &i in edited.iter().rev() {
        let mut t = track(&layers[i]);
        while layers.iter().enumerate().any(|(j, o)| {
            j != i
                && !group(o)
                && (j > i || !edited.contains(&j))
                && key(o) == key(&layers[i])
                && track(o) == t
                && overlap(held_span(o), held_span(&layers[i]))
        }) {
            t += 1.0;
        }
        if t != track(&layers[i]) {
            values::obj(&mut layers[i]).insert("track", Value::Number(t));
            changed = true;
        }
    }
    let linked = |layers: &[Value], a: usize, b: usize| {
        !group(&layers[a])
            && !group(&layers[b])
            && key(&layers[a]) == key(&layers[b])
            && track(&layers[a]) != track(&layers[b])
            && overlap(ends(&layers[a]), ends(&layers[b]))
    };
    let mut run = edited.clone();
    let mut k = 0;
    while k < run.len() {
        let a = run[k];
        for b in 0..layers.len() {
            if !run.contains(&b) && linked(&layers, a, b) {
                run.push(b);
            }
        }
        k += 1;
    }
    run.sort_unstable();
    // Top first: each clip after every clip on a higher track it overlaps,
    // otherwise in its present order.
    let above = |a: usize, b: usize| linked(&layers, a, b) && track(&layers[a]) > track(&layers[b]);
    let mut order = Vec::with_capacity(run.len());
    let mut left = run.clone();
    while let Some(pick) = left
        .iter()
        .position(|&a| !left.iter().any(|&b| above(b, a)))
    {
        order.push(left.remove(pick));
    }
    if order.len() == run.len() && order != run {
        let moved: Vec<usize> = run
            .iter()
            .zip(&order)
            .filter(|(slot, from)| slot != from)
            .map(|(slot, _)| *slot)
            .collect();
        let matte = |l: &Value| l.get("matteMode").is_some_and(|m| *m != Value::Null);
        let blocked = (moved[0]..=moved[moved.len() - 1]).any(|j| {
            let l = &layers[j];
            locked_here(&layers, l)
                || values::flag(l, "threeD")
                || group(l)
                || matte(l)
                || layers.get(j + 1).is_some_and(matte)
                || key(l) != key(&layers[run[0]])
        });
        if !blocked {
            let original = layers.clone();
            for (slot, from) in run.iter().zip(&order) {
                layers[*slot] = original[*from].clone();
            }
            changed = true;
        }
    }
    if changed {
        layers::write_layers(cx, comp, layers)
    } else {
        Ok(())
    }
}

/// The time a clip holds its track alone: less what its transitions let a
/// neighbour on its track overlap (the Tracks timeline's packing).
fn held_span(layer: &Value) -> (f64, f64) {
    let (from, end) = ends(layer);
    let audio = values::text(layer, "type") == Some("audio");
    let side = |field: &str, fade: &str| {
        let length = if audio {
            layer.get("d").and_then(|d| values::num(d, fade))
        } else {
            layer
                .get(field)
                .filter(|t| values::text(t, "type").is_some())
                .and_then(|t| values::num(t, "dur"))
        };
        length.filter(|d| d.is_finite()).unwrap_or(0.0).max(0.0)
    };
    let (a, b) = (
        from + side("transitionIn", "fadeIn"),
        end - side("transitionOut", "fadeOut"),
    );
    if a <= b {
        (a, b)
    } else {
        let mid = (from + end) / 2.0;
        (mid, mid)
    }
}

fn ripple_trim(
    cx: &mut Cx<'_>,
    ids: &[String],
    edge: TimeEdge,
    time: f64,
    by_track: bool,
) -> Result<(), CommandError> {
    let time = values::finite(time, "time")?;
    let (comp, targets) = leaves(cx, ids)?;
    let frame = 1.0 / cx.fps(comp.as_deref());
    let mut removed: Vec<(f64, f64, RippleTrack)> = vec![];
    // An edge dragged outward: (the layer's old end, the time it gained,
    // its track when the ripple keeps to tracks).
    let mut grown: Vec<(String, f64, f64, RippleTrack)> = vec![];
    for layer in targets {
        let (from, end) = ends(&layer);
        let track = by_track.then(|| track_of(&layer));
        let id = values::text(&layer, "id").unwrap_or_default().to_owned();
        match edge {
            TimeEdge::In if time < from - EDGE_EPS => {
                // More of the head shows; the layer keeps its start.
                let t = time.max(first_start(&layer));
                if from - t > EDGE_EPS {
                    let mut longer = cut_head(&layer, layer.clone(), t);
                    values::obj(&mut longer).insert("from", Value::Number(from));
                    put(cx, longer)?;
                    grown.push((id, end, from - t, track));
                }
            }
            TimeEdge::Out if time > end + EDGE_EPS => {
                let t = time.min(from + media_limit(cx, &layer).unwrap_or(f64::INFINITY));
                if t - end > EDGE_EPS {
                    put(cx, with_dur(layer, t - from))?;
                    grown.push((id, end, t - end, track));
                }
            }
            _ if time <= from + EDGE_EPS || time >= end - EDGE_EPS => {}
            TimeEdge::In => {
                let t = time.min(end - frame);
                removed.push((from, t, track));
                put(cx, cut_head(&layer, layer.clone(), t))?;
            }
            TimeEdge::Out => {
                let t = time.max(from + frame);
                removed.push((t, end, track));
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
            if values::text(layer, "type") == Some("group") || locked_here(&layers, layer) {
                continue;
            }
            let (from, _) = ends(layer);
            // A grown target moves on by the others' growth before it, never
            // by its own.
            let shift: f64 = grown
                .iter()
                .filter(|(g, pivot, _, track)| {
                    g != id
                        && from >= pivot - EDGE_EPS
                        && track.as_ref().is_none_or(|t| *t == track_of(layer))
                })
                .map(|(_, _, gained, _)| gained)
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
/// With a track on a span (a ripple kept to tracks), only layers on that
/// track move for it.
fn close_up(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    mut removed: Vec<(f64, f64, RippleTrack)>,
) -> Result<(), CommandError> {
    removed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut spans: Vec<(f64, f64, RippleTrack)> = vec![];
    for (start, end, track) in removed {
        match spans.iter_mut().rev().find(|s| s.2 == track) {
            Some(last) if start <= last.1 + EDGE_EPS => last.1 = last.1.max(end),
            _ => spans.push((start, end, track)),
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
            .filter(|(_, end, track)| {
                from >= end - EDGE_EPS && track.as_ref().is_none_or(|t| *t == track_of(layer))
            })
            .map(|(start, end, _)| end - start)
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
            let (mut lo, mut hi) = (
                from + frame,
                from + media_limit(cx, &layer).unwrap_or(f64::INFINITY),
            );
            if let Some(next) = &next {
                hi = hi.min(ends(next).1 - frame);
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
            let (mut lo, mut hi) = (first_start(&layer), end - frame);
            if let Some(previous) = &previous {
                lo = lo.max(ends(previous).0 + frame);
                // The clip before grows: never past its media's end.
                if let Some(limit) = media_limit(cx, previous) {
                    hi = hi.min(ends(previous).0 + limit);
                }
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
        // The clip before grows as this one moves on: never past its media.
        if let Some(limit) = media_limit(cx, p) {
            hi = hi.min(ends(p).0 + limit - from);
        }
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

/// Static speed changes keep the in-point and ripple only the current track.
/// The caller writes the resized target in the same transaction/undo step.
pub(super) fn speed_ripple(
    cx: &mut Cx<'_>,
    layer: &Value,
    old_duration: f64,
) -> Result<(), CommandError> {
    let project = cx.project.unwrap_or_default();
    let tracks = cx
        .state
        .and_then(|s| s.ui.get(project))
        .is_some_and(|ui| ui.timeline.get("mode").and_then(Value::as_str) == Some("tracks"));
    if !tracks {
        return Ok(());
    }
    let id = values::text(layer, "id").unwrap_or_default();
    let loc = cx.find(id)?;
    let from = values::num(layer, "from").unwrap_or(0.);
    let delta = values::num(layer, "dur").unwrap_or(old_duration) - old_duration;
    if delta.abs() < EDGE_EPS {
        return Ok(());
    }
    let track = track_of(layer);
    for other in cx.layers(loc.comp_id.as_deref()) {
        if values::text(&other, "id") == Some(id)
            || track_of(&other) != track
            || values::text(&other, "type") == Some("group")
        {
            continue;
        }
        let start = values::num(&other, "from").unwrap_or(0.);
        if start < from + old_duration - EDGE_EPS {
            continue;
        }
        let other_id = values::text(&other, "id").unwrap_or_default();
        let other_loc = cx.find(other_id)?;
        cx.check_lock(&other_loc, other_id)?;
        let mut moved = other;
        values::obj(&mut moved).insert("from", Value::Number(start + delta));
        cx.put_layer(&other_loc, moved)?;
    }
    Ok(())
}
