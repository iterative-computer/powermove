//! Keyframes: replacing tracks, adding, toggling, deleting, moving, typed
//! edits, paste, easing, interpolation and Bézier handles.
//!
//! Ports `replaceKeyframes` and `setEasing` of `legacy/core/editing.ts`,
//! `PM.setKeyOn`/`setKeysOn`/`removeKey`/`applyEaseTo` of
//! `legacy/core/anim.ts`, the interpolation commands of
//! `core/anim/temporal-ease.ts` (`easyEase`, `setInterpolation`, `setHold`,
//! `applyUnitCurve`, `applyContinuity`), the timeline's key drags and
//! keyframe value dialog (`extensions/timeline/timeline.ts`,
//! `keyframe-value-dialog.ts`), and `legacy/core/keyframe-simplification.ts`.
//!
//! Keys are written in the native (After Effects) form the app adopts on
//! first evaluation: `{ t, v, i, inInterp, outInterp, inEase, outEase,
//! autoBezier, continuous }` for numeric tracks; non-numeric tracks (colors,
//! strings, switches) keep the legacy `{ t, v, i, eo, ei, hold }` handles, as
//! the old app did. Key times are layer-local seconds.

use std::collections::HashSet;

use pm_eval::temporal::{Interp, Side};
use pm_model::{Map, Value};

use super::channels::{self, Step};
use super::{Cx, new_id, properties, track, values};
use crate::error::CommandError;

/// An easing preset name (`PM.Ease.PRESETS`: `linear`, `ease`, `easeIn`,
/// `easeOut`, `easeInOut`, `quadIn` … `glide`) or a unit-square cubic
/// `[x1, y1, x2, y2]` (x clamped to 0–1, y to −4–4).
#[derive(Clone, Debug, PartialEq)]
pub enum EaseSpec {
    Preset(String),
    Curve([f64; 4]),
}

impl EaseSpec {
    pub fn linear() -> EaseSpec {
        EaseSpec::Preset("linear".into())
    }
}

/// Which side of a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EaseSide {
    In,
    Out,
    #[default]
    Both,
}

/// AE's Keyframe Interpolation choices (`setInterpolation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Bezier,
    Hold,
    /// Continuous Bézier (joined handles).
    Continuous,
    /// Auto Bézier.
    Auto,
}

/// A side's speed (value units per second) and influence (percent).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TemporalEase {
    pub speed: f64,
    pub influence: f64,
}

/// A handle edit: the side's ease, or a point in (seconds, value) space
/// where the graph editor dropped the handle (`setHandleFromPoint`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HandleValue {
    Ease(TemporalEase),
    /// Layer-local seconds and the channel's value units.
    Point {
        t: f64,
        v: f64,
    },
}

/// One copied key (`KeyframeClipboard` track entry): its composition time
/// relative to the copied range's start, and the key as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct CopiedKey {
    pub at: f64,
    pub key: Value,
}

/// One key of `replace_keyframes`.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyframeSpec {
    /// Layer-local seconds (negative becomes 0).
    pub time: f64,
    pub value: Value,
    /// Named ease (default linear); ignored when `temporal` is given.
    pub ease: Option<EaseSpec>,
    pub hold: bool,
    /// Keep this key id (native edits keep identities); unique per track and
    /// not used by another channel.
    pub id: Option<String>,
    /// Native ease fields (`inInterp`, `outEase`, …) copied as given; keeps
    /// the exact subframe time.
    pub temporal: Option<Value>,
    /// `Some(None)` removes the spring, `Some(Some(v))` sets it.
    pub spring: Option<Option<Value>>,
    /// Unit handles `[eo0, eo1, ei0, ei1]` instead of `ease`.
    pub handles: Option<[f64; 4]>,
}

impl KeyframeSpec {
    pub fn new(time: f64, value: impl Into<Value>) -> KeyframeSpec {
        KeyframeSpec {
            time,
            value: value.into(),
            ease: None,
            hold: false,
            id: None,
            temporal: None,
            spring: None,
            handles: None,
        }
    }
}

/// `replace_keyframes`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplaceKeyframes {
    pub layer_id: String,
    pub channel: String,
    pub keyframes: Vec<KeyframeSpec>,
    /// `replace` (default true): drop the existing keys first. With no keys
    /// left the channel keeps the value shown at `time`.
    pub replace: bool,
    /// `Some(None)` removes the expression, `Some(Some(e))` sets it.
    pub expression: Option<Option<String>>,
    pub preserve_hand_edits: bool,
    /// The playhead, for the value kept when the last key goes.
    pub time: f64,
}

/// What `simplify_keyframes` works on.
#[derive(Clone, Debug, PartialEq)]
pub enum SimplifyScope {
    /// Every animated property of the project.
    All,
    /// Every animated property of these layers.
    Layers(Vec<String>),
    /// These `(layer id, channel)` pairs.
    Properties(Vec<(String, String)>),
}

/// Keyframe commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum KeyframeCommand {
    /// `replace_keyframes` (`replaceKeyframes`). Label "Edit Keyframes".
    Replace(Box<ReplaceKeyframes>),
    /// `set_easing` (`setEasing`): applies `curve` to the keys (outgoing
    /// handle from `x1,y1`, incoming from `x2,y2`, hold off). Ids not found
    /// are ignored; none found: `The selected keyframes are no longer
    /// available`. At most 1000 ids. Label "Set Easing".
    SetEasing {
        key_ids: Vec<String>,
        curve: EaseSpec,
    },
    /// `simplify_keyframes` (`simplifyProperties`, `simplifyKeyframes`):
    /// drops only analytically constant interior keys, then proves the
    /// result with the real evaluator at every frame (and shutter samples);
    /// any difference keeps the track as it was. Tracks are left alone when
    /// any expression exists in the project, on locked layers, structured,
    /// shader, extension, audio and pre-comp channels, non-numeric tracks,
    /// auto/continuous Bézier keys and unknown key fields. Label "Simplify
    /// Keyframes".
    Simplify { scope: SimplifyScope },
    /// Add Keyframe at Playhead / the diamond on a channel without a key at
    /// `time`: a key holding the channel's current value (`value`, or the
    /// static value when `None` on an unanimated channel). An existing key
    /// within half a frame is left alone. Label "Add Keyframe".
    Add {
        layer_id: String,
        channel: String,
        time: f64,
        key_id: String,
        value: Option<Value>,
    },
    /// The diamond: removes the key at `time` (within half a frame) or adds
    /// one as [`KeyframeCommand::Add`] does. Removing the last key keeps its
    /// value. Label "Toggle Keyframe".
    Toggle {
        layer_id: String,
        channel: String,
        time: f64,
        key_id: String,
        value: Option<Value>,
    },
    /// Delete selected keys (`PM.removeKey`); a channel losing its last key
    /// keeps that key's value as its static value. Missing ids are ignored.
    /// Label "Delete Keyframes".
    Delete { key_ids: Vec<String> },
    /// Clear All Keyframes / Remove Animation: the channel becomes static at
    /// the value its keys give at `time` (the playhead) and loses its keys.
    /// Label "Clear Keyframes".
    ClearAll {
        layer_id: String,
        channel: String,
        time: f64,
    },
    /// Key drag and Option+←/→ nudge (`planKeyframeMove`): every key moves
    /// by the same `delta`, snapped to a frame and clamped so no key leaves
    /// the composition (keys already outside may stay); an unselected key
    /// on a destination frame of the same channel is removed. In a drag,
    /// send the total delta each step with
    /// `DocumentStore::update_gesture_from_start`, so a key removed by a
    /// passing collision comes back. Label "Move Keyframes".
    Move { key_ids: Vec<String>, delta: f64 },
    /// The keyframe value dialog: a key's time (composition seconds, snapped
    /// to a frame) and/or value (finite). A time on another key's frame is
    /// refused (`Another keyframe already sits on that frame.`), and so is a
    /// new time outside the composition (`That time falls outside the
    /// composition.`). Several keys: one `Edit` each in a transaction. Label
    /// "Edit Keyframe".
    Edit {
        key_id: String,
        time: Option<f64>,
        value: Option<Value>,
    },
    /// Paste Keyframes Here (`pasteKeyframes`, one destination track): each
    /// copied key lands at `time + at` rounded to a frame, keys outside the
    /// composition are dropped, existing keys on those frames are replaced,
    /// ids come from `new_ids` (same length as `keys`; select them after).
    /// A track of another value kind refuses (`No matching property to
    /// paste keyframes into`). Choosing the destination (retargeting, the
    /// effect a paste creates) is the timeline's. Label "Paste Keyframes".
    Paste {
        layer_id: String,
        channel: String,
        time: f64,
        keys: Vec<CopiedKey>,
        new_ids: Vec<String>,
    },
    /// F9 / Shift+F9 / Cmd+Shift+F9 (`easyEase`): Bézier at 33.3% influence,
    /// flat tangent, on `side`. Label "Easy Ease".
    EasyEase {
        key_ids: Vec<String>,
        side: EaseSide,
    },
    /// Keyframe Interpolation (`setInterpolation`). Label "Change
    /// Interpolation".
    SetInterpolation {
        key_ids: Vec<String>,
        side: EaseSide,
        interpolation: Interpolation,
    },
    /// Toggle Hold Keyframe (`setHold`). Label "Toggle Hold Keyframe".
    SetHold { key_ids: Vec<String>, hold: bool },
    /// A Bézier handle (graph editor drag, easing panel numeric entry): the
    /// side's speed and influence (`setHandleFromPoint` for a point). An auto
    /// key becomes continuous; a continuous key mirrors the speed to its
    /// other side unless `split` (Option-drag) breaks it. Label "Adjust
    /// Handle".
    SetHandle {
        key_id: String,
        /// `In` or `Out` (`Both` is treated as `Out`).
        side: EaseSide,
        value: HandleValue,
        split: bool,
    },
    /// Spring easing of the outgoing segment (`spring`: `{ stiffness,
    /// damping, mass, ... }`; `None` removes). Label "Set Spring".
    SetSpring {
        key_ids: Vec<String>,
        spring: Option<Value>,
    },
    /// A Graph Editor drag of selected keys (`planGraphKeyframeMove`): one
    /// rigid time delta, snapped to a frame and clamped so no key leaves the
    /// composition and none comes within a frame of a key it does not own
    /// (the graph never replaces keys); numeric values move by
    /// `value_delta` (rounded to 0.001). Send totals with
    /// `update_gesture_from_start`. Label "Edit curve" / "Edit keyframes".
    GraphMove {
        key_ids: Vec<String>,
        delta: f64,
        value_delta: f64,
    },
    /// The graph transform box and Option-stretch
    /// (`planGraphKeyframeScale`): key times scale about `anchor_time`
    /// (composition seconds), clamped so keys never cross each other or
    /// come within a frame of unselected neighbours, snapped to frames
    /// (a value-only scale keeps exact times); numeric values scale about
    /// `anchor_value` (rounded to 0.001); stored ease speeds follow
    /// `value_scale / time_scale` so the curve keeps its shape. Label "Scale
    /// keyframes".
    Transform {
        key_ids: Vec<String>,
        anchor_time: f64,
        time_scale: f64,
        anchor_value: f64,
        value_scale: f64,
    },
}

impl KeyframeCommand {
    pub fn label(&self) -> String {
        match self {
            KeyframeCommand::Replace(_) => "Edit Keyframes",
            KeyframeCommand::SetEasing { .. } => "Set Easing",
            KeyframeCommand::Simplify { .. } => "Simplify Keyframes",
            KeyframeCommand::Add { .. } => "Add Keyframe",
            KeyframeCommand::Toggle { .. } => "Toggle Keyframe",
            KeyframeCommand::Delete { .. } => "Delete Keyframes",
            KeyframeCommand::ClearAll { .. } => "Clear Keyframes",
            KeyframeCommand::Move { .. } => "Move Keyframes",
            KeyframeCommand::Edit { .. } => "Edit Keyframe",
            KeyframeCommand::Paste { .. } => "Paste Keyframes",
            KeyframeCommand::EasyEase { .. } => "Easy Ease",
            KeyframeCommand::SetInterpolation { .. } => "Change Interpolation",
            KeyframeCommand::SetHold { .. } => "Toggle Hold Keyframe",
            KeyframeCommand::SetHandle { .. } => "Adjust Handle",
            KeyframeCommand::SetSpring { .. } => "Set Spring",
            KeyframeCommand::GraphMove { key_ids, .. } if key_ids.len() > 1 => "Edit keyframes",
            KeyframeCommand::GraphMove { .. } => "Edit curve",
            KeyframeCommand::Transform { .. } => "Scale keyframes",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            KeyframeCommand::Replace(c) => replace_keyframes(cx, c),
            KeyframeCommand::SetEasing { key_ids, curve } => set_easing(cx, key_ids, curve),
            KeyframeCommand::Simplify { scope } => simplify(cx, scope),
            KeyframeCommand::Add {
                layer_id,
                channel,
                time,
                key_id,
                value,
            } => add_key(cx, layer_id, channel, *time, key_id, value.as_ref(), false),
            KeyframeCommand::Toggle {
                layer_id,
                channel,
                time,
                key_id,
                value,
            } => add_key(cx, layer_id, channel, *time, key_id, value.as_ref(), true),
            KeyframeCommand::Delete { key_ids } => {
                let ids: HashSet<&str> = key_ids.iter().map(String::as_str).collect();
                edit_tracks(cx, &ids, |_, prop, _| {
                    track::remove_keys(prop, &ids);
                    Ok(())
                })
            }
            KeyframeCommand::ClearAll {
                layer_id,
                channel,
                time,
            } => clear_all(cx, layer_id, channel, *time),
            KeyframeCommand::Move { key_ids, delta } => move_keys(cx, key_ids, *delta),
            KeyframeCommand::Edit {
                key_id,
                time,
                value,
            } => edit_key(cx, key_id, *time, value.as_ref()),
            KeyframeCommand::Paste {
                layer_id,
                channel,
                time,
                keys,
                new_ids,
            } => paste(cx, layer_id, channel, *time, keys, new_ids),
            KeyframeCommand::EasyEase { key_ids, side } => per_key(cx, key_ids, true, |keys, i| {
                track::easy_ease(keys, i, *side)
            }),
            KeyframeCommand::SetInterpolation {
                key_ids,
                side,
                interpolation,
            } => per_key(cx, key_ids, true, |keys, i| {
                track::set_interpolation(keys, i, *side, *interpolation)
            }),
            KeyframeCommand::SetHold { key_ids, hold } => per_key(cx, key_ids, false, |keys, i| {
                if track::numeric(keys[i].get("v")) {
                    track::set_hold(keys, i, *hold)
                } else {
                    track::set_hold_flag(keys, i, *hold)
                }
            }),
            KeyframeCommand::SetHandle {
                key_id,
                side,
                value,
                split,
            } => {
                let side = match side {
                    EaseSide::In => Side::In,
                    _ => Side::Out,
                };
                per_key(cx, std::slice::from_ref(key_id), true, |keys, i| {
                    track::set_handle(keys, i, side, value, *split);
                })
            }
            KeyframeCommand::SetSpring { key_ids, spring } => {
                per_key(cx, key_ids, false, |keys, i| {
                    let m = values::obj(&mut keys[i]);
                    match spring {
                        Some(v) => {
                            m.insert("spring", v.clone());
                        }
                        None => {
                            m.remove("spring");
                        }
                    }
                })
            }
            KeyframeCommand::GraphMove {
                key_ids,
                delta,
                value_delta,
            } => graph_edit(
                cx,
                key_ids,
                GraphEdit::Move {
                    delta: *delta,
                    value_delta: *value_delta,
                },
            ),
            KeyframeCommand::Transform {
                key_ids,
                anchor_time,
                time_scale,
                anchor_value,
                value_scale,
            } => graph_edit(
                cx,
                key_ids,
                GraphEdit::Scale {
                    anchor_time: *anchor_time,
                    time_scale: *time_scale,
                    anchor_value: *anchor_value,
                    value_scale: *value_scale,
                },
            ),
        }
    }
}

// ── evaluation ──────────────────────────────────────────────────────────────

/// `evalKfs(prop.kf, t)` (or `v` without keys): the keys' value at
/// layer-local `t`, without the channel's expression.
pub(crate) fn value_at(prop: &Value, t: f64, channel: &str) -> Value {
    let keys = track::keys_of(prop);
    if keys.is_empty() {
        return prop.get("v").cloned().unwrap_or(Value::Null);
    }
    let node = pm_model::Node::from_value(Value::Array(keys.to_vec()), 0);
    let mut springs = pm_eval::easing::SpringTables::new();
    let color = pm_eval::keyframes::interpolates_color(channel.as_bytes());
    pm_eval::keyframes::evaluate(Some(&node), t, color, &mut springs)
        .ok()
        .and_then(|v| pm_edit::channel::to_value(&v))
        .unwrap_or(Value::Null)
}

// ── finding keys ────────────────────────────────────────────────────────────

/// Runs `f` on every channel holding one of `ids` (layers refusing when
/// locked), then writes each touched layer back.
fn edit_tracks(
    cx: &mut Cx<'_>,
    ids: &HashSet<&str>,
    mut f: impl FnMut(&Value, &mut Value, &str) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    if ids.is_empty() {
        return Ok(());
    }
    for loc in cx.all_layers() {
        let layer = cx.layer_node(&loc)?.to_value();
        let hits: Vec<(String, Vec<Step>)> = channels::all(&layer)
            .into_iter()
            .filter(|(_, path)| {
                channels::get(&layer, path).is_some_and(|p| {
                    track::keys_of(p).iter().any(|k| {
                        k.get("i")
                            .and_then(Value::as_str)
                            .is_some_and(|i| ids.contains(i))
                    })
                })
            })
            .collect();
        if hits.is_empty() {
            continue;
        }
        let layer_id = values::text(&layer, "id").unwrap_or_default().to_owned();
        cx.check_lock(&loc, &layer_id)?;
        let snapshot = layer.clone();
        let mut layer = layer;
        for (channel, path) in hits {
            let prop = channels::get_mut(&mut layer, &path).expect("found");
            f(&snapshot, prop, &channel)?;
        }
        cx.put_layer(&loc, layer)?;
    }
    Ok(())
}

/// Runs `f(keys, index)` on each selected key, after `temporalKeys` when
/// `adopt` (numeric tracks in the native model). Ids not found are ignored.
fn per_key(
    cx: &mut Cx<'_>,
    key_ids: &[String],
    adopt: bool,
    mut f: impl FnMut(&mut [Value], usize),
) -> Result<(), CommandError> {
    let ids: HashSet<&str> = key_ids.iter().map(String::as_str).collect();
    let mut found = false;
    edit_tracks(cx, &ids, |_, prop, _| {
        let keys = track::keys_mut(prop);
        if adopt {
            track::temporal_keys(keys);
        }
        for i in 0..keys.len() {
            if keys[i]
                .get("i")
                .and_then(Value::as_str)
                .is_some_and(|k| ids.contains(k))
            {
                found = true;
                f(keys, i);
            }
        }
        Ok(())
    })?;
    if !found && !ids.is_empty() {
        return Err(CommandError::KeyframeNotFound(
            key_ids.first().cloned().unwrap_or_default(),
        ));
    }
    Ok(())
}

// ── commands ────────────────────────────────────────────────────────────────

/// `easingCurve(value)`: a preset or four handles, clamped.
fn easing_curve(curve: &EaseSpec) -> Result<[f64; 4], CommandError> {
    let mut c = match curve {
        EaseSpec::Preset(name) => pm_eval::easing::PRESETS
            .iter()
            .find(|(n, _)| *n == name.as_str())
            .map(|(_, p)| *p)
            .ok_or_else(|| {
                CommandError::Invalid("Easing curve must contain four handles".into())
            })?,
        EaseSpec::Curve(c) => *c,
    };
    for (i, part) in c.iter().enumerate() {
        values::finite(*part, &format!("easing handle {}", i + 1))?;
    }
    c[0] = values::clamp(c[0], 0.0, 1.0);
    c[2] = values::clamp(c[2], 0.0, 1.0);
    c[1] = values::clamp(c[1], -4.0, 4.0);
    c[3] = values::clamp(c[3], -4.0, 4.0);
    Ok(c)
}

/// `key.eo = …; key.ei = …` through the bridge accessors of
/// `temporal-bridge.ts`, then `key.hold = false`.
fn set_unit_handles(keys: &mut [Value], index: usize, eo: [f64; 2], ei: [f64; 2]) {
    if !keys.iter().all(|k| track::numeric(k.get("v"))) {
        let m = values::obj(&mut keys[index]);
        m.insert("eo", pair(eo));
        m.insert("ei", pair(ei));
        m.insert("hold", Value::Bool(false));
        return;
    }
    let mut e = track::ease_key(&keys[index]);
    let other = |offset: isize| -> Option<pm_eval::temporal::EaseKeyframe> {
        let i = index as isize + offset;
        (i >= 0)
            .then(|| keys.get(i as usize))
            .flatten()
            .map(track::ease_key)
    };
    let span = |o: Option<pm_eval::temporal::EaseKeyframe>| match o {
        Some(o) => (
            (o.t - e.t).abs(),
            (o.v.unwrap_or(f64::NAN) - e.v.unwrap_or(f64::NAN)) * (o.t - e.t).signum(),
        ),
        None => (1.0, 1.0),
    };
    let (dt, dv) = span(other(1).or_else(|| other(-1)));
    e.out_interp = Interp::Bezier;
    e.auto_bezier = false;
    e.out_ease = pm_eval::temporal::ease_from_unit_handle(eo, dt, dv, Side::Out);
    let (dt, dv) = span(other(-1).or_else(|| other(1)));
    e.in_interp = Interp::Bezier;
    e.in_ease = pm_eval::temporal::ease_from_unit_handle(ei, dt, dv, Side::In);
    track::write_ease(&mut keys[index], &e);
}

fn pair(v: [f64; 2]) -> Value {
    Value::Array(vec![Value::Number(v[0]), Value::Number(v[1])])
}

/// `setEasing(command)`.
fn set_easing(cx: &mut Cx<'_>, key_ids: &[String], curve: &EaseSpec) -> Result<(), CommandError> {
    let ids: HashSet<&str> = key_ids
        .iter()
        .filter(|id| !id.is_empty())
        .take(1000)
        .map(String::as_str)
        .collect();
    if ids.is_empty() {
        return Err(CommandError::Invalid("Select at least one keyframe".into()));
    }
    let curve = easing_curve(curve)?;
    let mut found = false;
    edit_tracks(cx, &ids, |_, prop, _| {
        let keys = track::keys_mut(prop);
        track::temporal_keys(keys);
        for i in 0..keys.len() {
            if keys[i]
                .get("i")
                .and_then(Value::as_str)
                .is_some_and(|k| ids.contains(k))
            {
                found = true;
                set_unit_handles(keys, i, [curve[0], curve[1]], [curve[2], curve[3]]);
            }
        }
        Ok(())
    })?;
    if !found {
        return Err(CommandError::KeyframeNotFound(String::new()));
    }
    Ok(())
}

/// Add Keyframe at Playhead and the diamond.
fn add_key(
    cx: &mut Cx<'_>,
    layer_id: &str,
    channel: &str,
    time: f64,
    key_id: &str,
    value: Option<&Value>,
    toggle: bool,
) -> Result<(), CommandError> {
    let (loc, mut layer, channel, path) = properties::resolve(cx, layer_id, channel)?;
    let fps = track::safe_fps(cx.fps(loc.comp_id.as_deref()));
    let local = track::local_time(&layer, values::finite(time, "time")?);
    let prop = channels::get_mut(&mut layer, &path).expect("resolved");
    let existing = track::keys_of(prop).iter().position(|k| {
        (track::js_number(k.get("t").unwrap_or(&Value::Null)) - local).abs() < 0.5 / fps
    });
    match existing {
        Some(i) if toggle => {
            let id = track::key_id(&track::keys_of(prop)[i]);
            let ids: HashSet<&str> = [id.as_str()].into_iter().collect();
            track::remove_keys(prop, &ids);
        }
        Some(_) => return Ok(()),
        None => {
            let v = match value {
                Some(v) => v.clone(),
                None => value_at(prop, local, &channel),
            };
            let (id, created) =
                track::set_key_on(prop, local, v, "linear", fps, true, || key_id.to_owned());
            if created {
                track::apply_presets(track::keys_mut(prop), &[(id, "linear".into(), None)]);
            }
        }
    }
    cx.put_layer(&loc, layer)
}

/// Clear All Keyframes: static at the value shown at `time`.
fn clear_all(
    cx: &mut Cx<'_>,
    layer_id: &str,
    channel: &str,
    time: f64,
) -> Result<(), CommandError> {
    let (loc, mut layer, channel, path) = properties::resolve(cx, layer_id, channel)?;
    let local = track::local_time(&layer, values::finite(time, "time")?);
    let prop = channels::get_mut(&mut layer, &path).expect("resolved");
    if track::keys_of(prop).is_empty() {
        return Ok(());
    }
    let v = value_at(prop, local, &channel);
    let m = values::obj(prop);
    m.insert("v", v);
    m.insert("kf", Value::Array(Vec::new()));
    cx.put_layer(&loc, layer)
}

/// Key drags and nudges (`planKeyframeMove`): one rigid delta, snapped to
/// a frame and clamped so no key leaves the composition (keys already
/// outside may stay); unselected keys on a destination frame are removed.
fn move_keys(cx: &mut Cx<'_>, key_ids: &[String], delta: f64) -> Result<(), CommandError> {
    let delta = values::finite(delta, "delta")?;
    let ids: HashSet<&str> = key_ids.iter().map(String::as_str).collect();
    // Pass 1: the clamp over every selected key.
    let mut fps = None;
    let mut min_delta = f64::NEG_INFINITY;
    let mut max_delta = f64::INFINITY;
    for loc in cx.all_layers() {
        let layer = cx.layer_node(&loc)?.to_value();
        let from = values::num(&layer, "from").unwrap_or(0.0);
        let dur = cx.duration(loc.comp_id.as_deref());
        for (_, path) in channels::all(&layer) {
            let Some(prop) = channels::get(&layer, &path) else {
                continue;
            };
            for k in track::keys_of(prop) {
                if !k
                    .get("i")
                    .and_then(Value::as_str)
                    .is_some_and(|i| ids.contains(i))
                {
                    continue;
                }
                fps.get_or_insert_with(|| track::safe_fps(cx.fps(loc.comp_id.as_deref())));
                let t = track::js_number(k.get("t").unwrap_or(&Value::Null));
                if !t.is_finite() {
                    continue;
                }
                min_delta = min_delta.max(t.min(-from) - t);
                max_delta = max_delta.min(t.max(0f64.max(dur - from)) - t);
            }
        }
    }
    let Some(rate) = fps else {
        return Ok(());
    };
    let mut delta = values::js_round(delta * rate) / rate;
    delta = delta.max(min_delta).min(max_delta);
    if delta.abs() <= 1e-12 {
        return Ok(());
    }
    edit_tracks(cx, &ids, |_, prop, _| {
        let keys = track::keys_mut(prop);
        let selected = |k: &Value| {
            k.get("i")
                .and_then(Value::as_str)
                .is_some_and(|i| ids.contains(i))
        };
        let t_of = |k: &Value| track::js_number(k.get("t").unwrap_or(&Value::Null));
        let frames: Vec<f64> = keys
            .iter()
            .filter(|k| selected(k))
            .map(|k| values::js_round((t_of(k) + delta) * rate))
            .collect();
        keys.retain(|k| selected(k) || !frames.contains(&values::js_round(t_of(k) * rate)));
        for k in keys.iter_mut() {
            if selected(k) {
                let t = t_of(k);
                values::obj(k).insert("t", Value::Number(t + delta));
            }
        }
        track::sort_keys(keys);
        track::temporal_keys(keys);
        Ok(())
    })
}

/// The keyframe value dialog.
fn edit_key(
    cx: &mut Cx<'_>,
    key_id: &str,
    time: Option<f64>,
    value: Option<&Value>,
) -> Result<(), CommandError> {
    if time.is_none() && value.is_none() {
        return Ok(());
    }
    if let Some(v) = value
        && v.as_f64().is_some()
    {
        values::finite(v.as_f64().unwrap_or(f64::NAN), "value")
            .map_err(|_| CommandError::Invalid("Enter a finite value.".into()))?;
    }
    if let Some(t) = time
        && !t.is_finite()
    {
        return Err(CommandError::Invalid("Enter a finite time.".into()));
    }
    let ids: HashSet<&str> = [key_id].into_iter().collect();
    let mut found = false;
    let info: std::collections::HashMap<String, (f64, f64)> = cx
        .all_layers()
        .into_iter()
        .filter_map(|loc| {
            let id = cx
                .layer_node(&loc)
                .ok()?
                .get("id")
                .and_then(crate::model::str_of)?
                .to_owned();
            let c = loc.comp_id.as_deref();
            Some((id, (track::safe_fps(cx.fps(c)), cx.duration(c))))
        })
        .collect();
    edit_tracks(cx, &ids, |layer, prop, _| {
        let (fps, dur) = values::text(layer, "id")
            .and_then(|id| info.get(id).copied())
            .unwrap_or((30.0, 0.0));
        let from = values::num(layer, "from").unwrap_or(0.0);
        let keys = track::keys_mut(prop);
        let Some(index) = track::index_of(keys, key_id) else {
            return Ok(());
        };
        found = true;
        if let Some(v) = value {
            values::obj(&mut keys[index]).insert("v", v.clone());
        }
        if let Some(t) = time {
            let current = track::js_number(keys[index].get("t").unwrap_or(&Value::Null));
            let next = values::snap(t - from, fps);
            let min = current.min(-from);
            let max = current.max(dur - from);
            if next < min || next > max {
                return Err(CommandError::Invalid(
                    "That time falls outside the composition.".into(),
                ));
            }
            if keys.iter().enumerate().any(|(i, k)| {
                i != index
                    && (track::js_number(k.get("t").unwrap_or(&Value::Null)) - next).abs()
                        < 0.5 / fps
            }) {
                return Err(CommandError::Invalid(
                    "Another keyframe already sits on that frame.".into(),
                ));
            }
            values::obj(&mut keys[index]).insert("t", Value::Number(next));
            track::sort_keys(keys);
        }
        track::temporal_keys(keys);
        Ok(())
    })?;
    if !found {
        return Err(CommandError::KeyframeNotFound(key_id.to_owned()));
    }
    Ok(())
}

/// `keyframeValueKind(v)`.
fn value_kind(v: Option<&Value>) -> &'static str {
    match v {
        Some(Value::Number(_)) => "number",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::String(s)) if is_hex(&s.to_string_lossy()) => "color",
        Some(Value::String(_)) => "string",
        _ => "other",
    }
}

fn is_hex(s: &str) -> bool {
    let h = s.strip_prefix('#').unwrap_or("");
    (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `pasteKeyframes` for one destination track.
fn paste(
    cx: &mut Cx<'_>,
    layer_id: &str,
    channel: &str,
    time: f64,
    copied: &[CopiedKey],
    new_ids: &[String],
) -> Result<(), CommandError> {
    if copied.len() != new_ids.len() {
        return Err(CommandError::Invalid("new_ids must pair with keys".into()));
    }
    let (loc, mut layer, _, path) = properties::resolve(cx, layer_id, channel)?;
    let comp = loc.comp_id.clone();
    let fps = cx.fps(comp.as_deref()).max(1.0);
    let duration = cx.duration(comp.as_deref()).max(0.0);
    let from = values::num(&layer, "from").unwrap_or(0.0);
    let prop = channels::get_mut(&mut layer, &path).expect("resolved");
    let current = track::keys_of(prop)
        .first()
        .and_then(|k| k.get("v"))
        .or_else(|| prop.get("v"))
        .cloned();
    let kind = value_kind(copied.first().and_then(|c| c.key.get("v")));
    if value_kind(current.as_ref()) != kind {
        return Err(CommandError::Invalid(
            "No matching property to paste keyframes into".into(),
        ));
    }
    let keys: Vec<Value> = copied
        .iter()
        .zip(new_ids)
        .filter_map(|(c, id)| {
            let at = values::js_round((c.at + time) * fps) / fps;
            (at >= -1e-9 && at <= duration + 1e-9).then(|| {
                let mut k = c.key.clone();
                let m = values::obj(&mut k);
                m.insert("i", values::s(id));
                m.insert(
                    "t",
                    Value::Number(values::js_round((at - from) * fps) / fps),
                );
                k
            })
        })
        .collect();
    if keys.is_empty() {
        return Ok(());
    }
    let frame =
        |k: &Value| values::js_round(track::js_number(k.get("t").unwrap_or(&Value::Null)) * fps);
    let frames: Vec<f64> = keys.iter().map(frame).collect();
    let list = track::keys_mut(prop);
    list.retain(|k| !frames.contains(&frame(k)));
    list.extend(keys);
    track::sort_keys(list);
    track::temporal_keys(list);
    cx.put_layer(&loc, layer)
}

/// `replaceKeyframes(command)`.
fn replace_keyframes(cx: &mut Cx<'_>, c: &ReplaceKeyframes) -> Result<(), CommandError> {
    let (loc, mut layer, channel, path) = properties::resolve(cx, &c.layer_id, &c.channel)?;
    if properties::preserves(cx, c.preserve_hand_edits)
        && properties::locked_intent(&layer, &channel)
    {
        return Err(CommandError::PreservedHandEdit {
            layer_id: c.layer_id.clone(),
            channel,
        });
    }
    let fps = cx.fps(loc.comp_id.as_deref());
    let local_now = track::local_time(&layer, c.time);
    let prop = channels::get_mut(&mut layer, &path).expect("resolved");
    let numeric = prop.get("v").and_then(Value::as_f64).is_some();
    let mut next = Vec::with_capacity(c.keyframes.len());
    for (index, key) in c.keyframes.iter().enumerate() {
        let time = values::finite(key.time, &format!("keyframe {} time", index + 1))?.max(0.0);
        let value = if numeric {
            Value::Number(values::finite(
                track::js_number(&key.value),
                &format!("keyframe {} value", index + 1),
            )?)
        } else {
            key.value.clone()
        };
        next.push((time, value, key));
    }
    if let Some(Some(e)) = &c.expression
        && !e.is_empty()
    {
        properties::check_expression(e)?;
    }
    let native = next
        .iter()
        .any(|(_, _, k)| k.temporal.is_some() || k.id.is_some());
    if native {
        let existing: Vec<Value> = track::keys_of(prop).to_vec();
        let mut out = Vec::with_capacity(next.len());
        for (time, value, key) in &next {
            let old = key.id.as_deref().and_then(|id| {
                existing
                    .iter()
                    .find(|k| k.get("i").and_then(Value::as_str) == Some(id))
            });
            let mut v = old.cloned().unwrap_or_else(|| Value::Object(Map::new()));
            let m = values::obj(&mut v);
            if let Some(Value::Object(t)) = &key.temporal {
                for (k, x) in t.iter() {
                    m.insert(k.clone(), x.clone());
                }
            }
            m.insert("t", Value::Number(*time));
            m.insert("v", value.clone());
            m.insert(
                "i",
                values::s(&key.id.clone().unwrap_or_else(|| new_id("k"))),
            );
            match &key.spring {
                Some(None) => {
                    m.remove("spring");
                }
                Some(Some(s)) => {
                    m.insert("spring", s.clone());
                }
                None => {}
            }
            if key.temporal.is_none() {
                for f in ["inInterp", "outInterp", "inEase", "outEase"] {
                    m.remove(f);
                }
                let (eo, ei) = match (key.handles, &key.ease) {
                    (Some(h), _) => ([h[0], h[1]], [h[2], h[3]]),
                    (None, Some(EaseSpec::Curve(c))) => ([c[0], c[1]], [c[2], c[3]]),
                    (None, Some(EaseSpec::Preset(p))) => pm_eval::easing::handles(p),
                    (None, None) => pm_eval::easing::handles("linear"),
                };
                m.insert("eo", pair(eo));
                m.insert("ei", pair(ei));
                m.insert("hold", Value::Bool(key.hold));
            }
            out.push(v);
        }
        let mut seen = HashSet::new();
        for k in &out {
            if !seen.insert(track::key_id(k)) {
                return Err(CommandError::Invalid(
                    "Keyframe IDs must be unique within a track".into(),
                ));
            }
        }
        // Ids may not belong to another property.
        let this = (loc.clone(), path.clone());
        for other in cx.all_layers() {
            let node = cx.layer_node(&other)?.to_value();
            for (_, p) in channels::all(&node) {
                if other == this.0 && p == this.1 {
                    continue;
                }
                if let Some(ch) = channels::get(&node, &p)
                    && track::keys_of(ch)
                        .iter()
                        .any(|k| seen.contains(&track::key_id(k)))
                {
                    return Err(CommandError::Invalid(
                        "Keyframe ID belongs to another property".into(),
                    ));
                }
            }
        }
        let mut list: Vec<Value> = if c.replace {
            Vec::new()
        } else {
            existing
                .into_iter()
                .filter(|k| {
                    let id = track::key_id(k);
                    let t = track::js_number(k.get("t").unwrap_or(&Value::Null));
                    !seen.contains(&id)
                        && !out
                            .iter()
                            .any(|n| track::js_number(n.get("t").unwrap_or(&Value::Null)) == t)
                })
                .collect()
        };
        list.extend(out);
        let fallback = prop.get("v").cloned().unwrap_or(Value::Null);
        let list = normalize_keyframes(list, &fallback);
        values::obj(prop).insert("kf", Value::Array(list));
        track::temporal_keys(track::keys_mut(prop));
    } else {
        if c.replace {
            if !track::keys_of(prop).is_empty() && next.is_empty() {
                let v = value_at(prop, local_now, &channel);
                values::obj(prop).insert("v", v);
            }
            values::obj(prop).insert("kf", Value::Array(Vec::new()));
        }
        let mut edits = Vec::new();
        for (time, value, key) in &next {
            let ease = match &key.ease {
                Some(EaseSpec::Preset(p)) => p.clone(),
                _ => "linear".to_owned(),
            };
            let (id, _) = track::set_key_on(prop, *time, value.clone(), &ease, fps, false, || {
                new_id("k")
            });
            edits.push((id, ease, Some(key.hold)));
        }
        track::apply_presets(track::keys_mut(prop), &edits);
    }
    if let Some(e) = &c.expression {
        let e = e.as_deref().filter(|e| !e.is_empty());
        values::obj(prop).insert("expr", e.map(values::s).unwrap_or(Value::Null));
    }
    cx.put_layer(&loc, layer)
}

/// `PM.normalizeKeyframes(raw, fallbackValue)`: valid keys only, sorted by
/// time (stable), duplicate ids renewed, keys within 1e-6 s collapsed (the
/// later wins).
pub(crate) fn normalize_keyframes(raw: Vec<Value>, fallback: &Value) -> Vec<Value> {
    let expected = match fallback {
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Bool(_) => "boolean",
        _ => "any",
    };
    let valid = |v: Option<&Value>| match (expected, v) {
        ("number", Some(Value::Number(n))) => n.is_finite(),
        ("string", Some(Value::String(_))) => true,
        ("boolean", Some(Value::Bool(_))) => true,
        ("any", Some(Value::Number(n))) => n.is_finite(),
        ("any", Some(Value::String(_) | Value::Bool(_))) => true,
        _ => false,
    };
    let pair_or = |v: Option<&Value>, fallback: [f64; 2]| -> Value {
        match v.and_then(Value::as_array) {
            Some(a)
                if a.len() >= 2
                    && a[0].as_f64().is_some_and(f64::is_finite)
                    && a[1].as_f64().is_some_and(f64::is_finite) =>
            {
                Value::Array(vec![a[0].clone(), a[1].clone()])
            }
            _ => pair(fallback),
        }
    };
    let mut keys: Vec<(f64, Value)> = raw
        .into_iter()
        .filter_map(|key| {
            let t = track::js_number(key.get("t")?);
            (key.as_object().is_some() && t.is_finite() && t >= 0.0 && valid(key.get("v")))
                .then_some((t, key))
        })
        .map(|(t, mut key)| {
            let native = key.get("inInterp").is_some();
            let m = values::obj(&mut key);
            m.insert("t", Value::Number(if t == 0.0 { 0.0 } else { t }));
            if m.get("i").and_then(Value::as_str).is_none_or(str::is_empty) {
                m.insert("i", values::s(&new_id("k")));
            }
            if !native {
                let eo = pair_or(m.get("eo"), [0.33, 0.0]);
                let ei = pair_or(m.get("ei"), [0.67, 1.0]);
                m.insert("eo", eo);
                m.insert("ei", ei);
                let hold = m.get("hold").and_then(Value::as_bool) == Some(true);
                m.insert("hold", Value::Bool(hold));
            }
            (t, key)
        })
        .collect();
    keys.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<Value> = Vec::with_capacity(keys.len());
    let mut ids: HashSet<String> = HashSet::new();
    for (t, mut key) in keys {
        let mut id = track::key_id(&key);
        if ids.contains(&id) {
            id = new_id("k");
            values::obj(&mut key).insert("i", values::s(&id));
        }
        ids.insert(id);
        if let Some(prev) = out.last()
            && (track::js_number(prev.get("t").unwrap_or(&Value::Null)) - t).abs() <= 1e-6
        {
            ids.remove(&track::key_id(prev));
            *out.last_mut().expect("present") = key;
        } else {
            out.push(key);
        }
    }
    out
}

// ── graph editor ────────────────────────────────────────────────────────────

enum GraphEdit {
    Move {
        delta: f64,
        value_delta: f64,
    },
    Scale {
        anchor_time: f64,
        time_scale: f64,
        anchor_value: f64,
        value_scale: f64,
    },
}

/// One key of `captureKeyframeGesture`.
struct GraphItem {
    property: usize,
    id: String,
    time: f64,
    value: Option<f64>,
    selected: bool,
    from: f64,
    min: f64,
    max: f64,
}

/// `lowerBound(times, t, upper)`.
fn lower_bound(times: &[f64], t: f64, upper: bool) -> usize {
    times.partition_point(|x| if upper { *x <= t } else { *x < t })
}

/// `graphNeighbours(items)`: per property, the unselected times around a
/// key, and the smallest gap between distinct selected times.
struct Neighbours {
    fixed: Vec<Vec<f64>>,
    gap: f64,
}

impl Neighbours {
    fn new(items: &[GraphItem], properties: usize) -> Neighbours {
        let mut fixed = vec![Vec::new(); properties];
        let mut selected = vec![Vec::new(); properties];
        for i in items.iter().filter(|i| i.time.is_finite()) {
            if i.selected {
                selected[i.property].push(i.time);
            } else {
                fixed[i.property].push(i.time);
            }
        }
        let by = |a: &f64, b: &f64| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
        for t in &mut fixed {
            t.sort_by(by);
        }
        let mut gap = f64::INFINITY;
        for times in &mut selected {
            times.sort_by(by);
            let mut next = 1;
            for i in 0..times.len() {
                next = next.max(i + 1);
                while next < times.len() && times[next] - times[i] <= 1e-9 {
                    next += 1;
                }
                if next < times.len() {
                    gap = gap.min(times[next] - times[i]);
                }
            }
        }
        Neighbours { fixed, gap }
    }

    fn before(&self, item: &GraphItem) -> Option<f64> {
        let times = &self.fixed[item.property];
        lower_bound(times, item.time, false)
            .checked_sub(1)
            .map(|i| times[i])
    }

    fn after(&self, item: &GraphItem) -> Option<f64> {
        let times = &self.fixed[item.property];
        times.get(lower_bound(times, item.time, true)).copied()
    }
}

/// The Graph Editor's move and scale, rebuilt from the tracks holding the
/// selected keys.
fn graph_edit(cx: &mut Cx<'_>, key_ids: &[String], edit: GraphEdit) -> Result<(), CommandError> {
    let ids: HashSet<&str> = key_ids.iter().map(String::as_str).collect();
    let mut items: Vec<GraphItem> = Vec::new();
    let mut properties = 0usize;
    let mut fps = None;
    for loc in cx.all_layers() {
        let layer = cx.layer_node(&loc)?.to_value();
        let from = values::num(&layer, "from").unwrap_or(0.0);
        let dur = cx.duration(loc.comp_id.as_deref());
        for (_, path) in channels::all(&layer) {
            let Some(prop) = channels::get(&layer, &path) else {
                continue;
            };
            let keys = track::keys_of(prop);
            if !keys.iter().any(|k| {
                k.get("i")
                    .and_then(Value::as_str)
                    .is_some_and(|i| ids.contains(i))
            }) {
                continue;
            }
            fps.get_or_insert_with(|| track::safe_fps(cx.fps(loc.comp_id.as_deref())));
            for k in keys {
                let t = track::js_number(k.get("t").unwrap_or(&Value::Null));
                let id = track::key_id(k);
                items.push(GraphItem {
                    property: properties,
                    selected: ids.contains(id.as_str()),
                    id,
                    time: t,
                    value: k.get("v").and_then(Value::as_f64),
                    from,
                    min: t.min(-from),
                    max: t.max(0f64.max(dur - from)),
                });
            }
            properties += 1;
        }
    }
    let Some(rate) = fps else {
        return Ok(());
    };
    let frame = 1.0 / rate;
    let neighbours = Neighbours::new(&items, properties);
    let selected: Vec<&GraphItem> = items
        .iter()
        .filter(|i| i.selected && i.time.is_finite())
        .collect();
    // id → (time, value)
    let mut moves: Vec<(String, f64, Option<f64>)> = Vec::new();
    let mut speed_ratio = 1.0;
    match edit {
        GraphEdit::Move { delta, value_delta } => {
            let mut d =
                values::js_round((if delta.is_finite() { delta } else { 0.0 }) * rate) / rate;
            let min_d = selected
                .iter()
                .map(|i| i.min - i.time)
                .fold(f64::NEG_INFINITY, f64::max);
            let max_d = selected
                .iter()
                .map(|i| i.max - i.time)
                .fold(f64::INFINITY, f64::min);
            d = min_d.max(max_d.min(d));
            if d != 0.0 {
                for m in &selected {
                    if d > 0.0 {
                        if let Some(a) = neighbours.after(m) {
                            d = d.min((a - m.time - frame).max(0.0));
                        }
                    } else if let Some(b) = neighbours.before(m) {
                        d = d.max((b - m.time + frame).min(0.0));
                    }
                }
            }
            d = values::js_round(d * rate) / rate;
            let dv = values::finite(value_delta, "value delta")?;
            for m in &selected {
                let v = m.value.map(|v| track::round3(v + dv));
                moves.push((m.id.clone(), m.time + d, v));
            }
        }
        GraphEdit::Scale {
            anchor_time,
            time_scale,
            anchor_value,
            value_scale,
        } => {
            let (mut low, mut high) = (0f64, f64::INFINITY);
            for m in &selected {
                let anchor = anchor_time - m.from;
                let span = m.time - anchor;
                let mut min = m.min;
                let mut max = m.max;
                if let Some(b) = neighbours.before(m) {
                    min = min.max(b + frame);
                }
                if let Some(a) = neighbours.after(m) {
                    max = max.min(a - frame);
                }
                if span.abs() < 1e-9 {
                    continue;
                }
                let (a, b) = ((min - anchor) / span, (max - anchor) / span);
                low = low.max(a.min(b));
                high = high.min(a.max(b));
            }
            low = low.max(frame / neighbours.gap);
            low = low.min(1.0);
            high = high.max(1.0);
            let requested = if time_scale.is_finite() {
                time_scale
            } else {
                1.0
            };
            let ts = low.max(high.min(requested));
            let vs = if value_scale.is_finite() {
                value_scale
            } else {
                1.0
            };
            for m in &selected {
                let anchor = anchor_time - m.from;
                let t = if ts == 1.0 {
                    m.time
                } else {
                    values::js_round((anchor + (m.time - anchor) * ts) * rate) / rate
                };
                let v = m
                    .value
                    .filter(|_| vs != 1.0)
                    .map(|v| anchor_value + (v - anchor_value) * vs)
                    .filter(|v| v.is_finite())
                    .map(track::round3);
                moves.push((m.id.clone(), if t == 0.0 { 0.0 } else { t }, v));
            }
            speed_ratio = if ts > 1e-6 { vs / ts } else { 1.0 };
        }
    }
    let scaled = matches!(edit, GraphEdit::Scale { .. });
    edit_tracks(cx, &ids, |_, prop, _| {
        let keys = track::keys_mut(prop);
        for k in keys.iter_mut() {
            let id = track::key_id(k);
            let Some((_, t, v)) = moves.iter().find(|(m, _, _)| *m == id) else {
                continue;
            };
            let m = values::obj(k);
            m.insert("t", Value::Number(*t));
            if let Some(v) = v {
                m.insert("v", Value::Number(*v));
            }
            if scaled && speed_ratio != 1.0 {
                for side in ["inEase", "outEase"] {
                    if let Some(e) = m.get_mut(side).and_then(Value::as_object_mut)
                        && let Some(speed) = e.get("speed").and_then(Value::as_f64)
                    {
                        e.insert("speed", Value::Number(speed * speed_ratio));
                    }
                }
            }
        }
        track::sort_keys(keys);
        Ok(())
    })
}

// ── simplify ────────────────────────────────────────────────────────────────

const KNOWN_KEY_FIELDS: [&str; 13] = [
    "i",
    "t",
    "v",
    "inInterp",
    "outInterp",
    "inEase",
    "outEase",
    "autoBezier",
    "continuous",
    "eo",
    "ei",
    "hold",
    "bezierMode",
];

/// `flat(a, b)` of `simplify-keyframes.ts`.
fn flat(a: &pm_eval::temporal::EaseKeyframe, b: &pm_eval::temporal::EaseKeyframe) -> bool {
    if a.v != b.v {
        return false;
    }
    if pm_eval::temporal::is_held_segment(a, b)
        || (a.out_interp == Interp::Linear && b.in_interp == Interp::Linear)
    {
        return true;
    }
    let value = a.v.unwrap_or(f64::NAN);
    (a.out_interp != Interp::Bezier || a.out_ease.speed == 0.0)
        && (b.in_interp != Interp::Bezier || b.in_ease.speed == 0.0)
        && value - 2.0 * value + value == 0.0
        && value - 3.0 * value + 3.0 * value - value == 0.0
}

struct SimplifyOptions {
    fps: f64,
    from: f64,
    duration: f64,
    composition_duration: f64,
    shutter: Option<f64>,
    trim_outside: bool,
}

/// `evalKfs(keys, t)` over a prepared track.
struct Evaluator {
    node: pm_model::Node,
    track: pm_eval::keyframes::PreparedTrack,
    springs: pm_eval::easing::SpringTables,
}

impl Evaluator {
    fn new(keys: &[Value]) -> Evaluator {
        let node = pm_model::Node::from_value(Value::Array(keys.to_vec()), 0);
        let track = pm_eval::keyframes::PreparedTrack::prepare(Some(&node));
        Evaluator {
            node,
            track,
            springs: pm_eval::easing::SpringTables::new(),
        }
    }

    fn at(&mut self, t: f64) -> Option<pm_eval::JsValue> {
        self.track
            .value(Some(&self.node), t, false, &mut self.springs)
            .ok()
    }
}

/// `a === b` for evaluator results.
fn strictly_equal(a: &Option<pm_eval::JsValue>, b: &Option<pm_eval::JsValue>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => pm_eval::js_sem::strict_eq(a, b),
        _ => false,
    }
}

/// `simplifyKeyframes(source, options)`: `None` keeps the source.
fn simplify_track(source: &[Value], o: &SimplifyOptions) -> Option<Vec<Value>> {
    if source.len() < 3
        || !(o.fps.is_finite() && o.fps > 0.0)
        || ![o.from, o.duration, o.composition_duration]
            .iter()
            .all(|x| x.is_finite())
        || o.duration <= 0.0
        || o.composition_duration <= 0.0
    {
        return None;
    }
    let mut previous_t = f64::NEG_INFINITY;
    for (i, key) in source.iter().enumerate() {
        let t = key.get("t").and_then(Value::as_f64).unwrap_or(f64::NAN);
        let ok = key
            .get("v")
            .and_then(Value::as_f64)
            .is_some_and(f64::is_finite)
            && t.is_finite()
            && (i == 0 || t > previous_t)
            && !values::flag(key, "autoBezier")
            && !values::flag(key, "continuous")
            && key.as_object().is_some_and(|m| {
                m.keys()
                    .all(|k| KNOWN_KEY_FIELDS.contains(&k.to_string_lossy().as_str()))
            });
        if !ok {
            return None;
        }
        previous_t = t;
    }
    let mut before = source.to_vec();
    track::adopt(&mut before);
    if before
        .iter()
        .any(|k| values::flag(k, "autoBezier") || values::flag(k, "continuous"))
    {
        return None;
    }
    let eased: Vec<_> = before.iter().map(track::ease_key).collect();
    let radius = o
        .shutter
        .filter(|s| *s != 0.0 && !s.is_nan())
        .unwrap_or(0.5)
        .max(0.5)
        / o.fps;
    let (mut first, mut last) = (0usize, before.len() - 1);
    if o.trim_outside {
        let (start, end) = (-radius, o.duration + radius);
        while first + 1 < before.len() && eased[first + 1].t < start {
            first += 1;
        }
        while last - 1 > first && eased[last - 1].t > end {
            last -= 1;
        }
    }
    let mut candidate = vec![first];
    for i in first + 1..last {
        let a = &eased[*candidate.last().expect("seeded")];
        let (b, c) = (&eased[i], &eased[i + 1]);
        if !(flat(a, b) && flat(b, c) && flat(a, c)) {
            candidate.push(i);
        }
    }
    if last > first {
        candidate.push(last);
    }
    if candidate.len() == source.len() {
        return None;
    }
    let after: Vec<Value> = candidate.iter().map(|&i| before[i].clone()).collect();
    let mut eb = Evaluator::new(&before);
    let mut ea = Evaluator::new(&after);
    let mut identical = |t: f64| strictly_equal(&eb.at(t), &ea.at(t));
    let start = if o.trim_outside {
        -radius
    } else {
        -o.from - radius
    };
    let end = if o.trim_outside {
        o.duration + radius
    } else {
        o.composition_duration - o.from + radius
    };
    let shutter = o
        .shutter
        .filter(|s| *s != 0.0 && !s.is_nan())
        .unwrap_or(0.5)
        / o.fps;
    let mut offsets = vec![0.0, -0.5 / o.fps, -0.25 / o.fps, 0.25 / o.fps, 0.5 / o.fps];
    offsets.extend((0..10).map(|s| ((f64::from(s) + 0.5) / 10.0 - 0.5) * shutter));
    let frame_start = ((start + o.from) * o.fps).floor() as i64;
    let frame_end = ((end + o.from) * o.fps).ceil() as i64;
    if frame_end - frame_start > 1_000_000 {
        return None;
    }
    for frame in frame_start..=frame_end {
        let time = frame as f64 / o.fps - o.from;
        for offset in &offsets {
            let t = time + offset;
            if t >= start && t <= end && !identical(t) {
                return None;
            }
        }
    }
    for i in 0..eased.len() {
        let time = eased[i].t;
        if time >= start && time <= end && !identical(time) {
            return None;
        }
        if let Some(next) = eased.get(i + 1) {
            for f in [0.25, 0.5, 0.75] {
                let t = time + (next.t - time) * f;
                if t >= start && t <= end && !identical(t) {
                    return None;
                }
            }
        }
    }
    Some(after)
}

/// `simplifyProperties(PM, simplificationRefs(command))`.
fn simplify(cx: &mut Cx<'_>, scope: &SimplifyScope) -> Result<(), CommandError> {
    let locs = cx.all_layers();
    let mut layers: Vec<(crate::model::LayerLoc, Value)> = Vec::with_capacity(locs.len());
    for loc in locs {
        let v = cx.layer_node(&loc)?.to_value();
        layers.push((loc, v));
    }
    let find = |id: &str| {
        layers
            .iter()
            .position(|(_, l)| values::text(l, "id") == Some(id))
    };
    let mut refs: Vec<(usize, String)> = Vec::new();
    match scope {
        SimplifyScope::All | SimplifyScope::Layers(_) => {
            if let SimplifyScope::Layers(ids) = scope
                && ids.iter().any(|id| find(id).is_none())
            {
                return Err(CommandError::Invalid(
                    "Simplification layer not found".into(),
                ));
            }
            for (i, (_, l)) in layers.iter().enumerate() {
                if let SimplifyScope::Layers(ids) = scope
                    && !ids
                        .iter()
                        .any(|id| Some(id.as_str()) == values::text(l, "id"))
                {
                    continue;
                }
                for (key, _) in channels::all(l) {
                    refs.push((i, key));
                }
            }
        }
        SimplifyScope::Properties(list) => {
            for (id, path) in list {
                let i = find(id)
                    .filter(|&i| channels::find(&layers[i].1, values::channel_key(path)).is_some())
                    .ok_or_else(|| {
                        CommandError::Invalid("Simplification property not found".into())
                    })?;
                refs.push((i, values::channel_key(path).to_owned()));
            }
        }
    }
    refs.dedup();
    let expression_consumer = layers.iter().any(|(_, l)| {
        channels::all(l).iter().any(|(_, p)| {
            channels::get(l, p)
                .and_then(|c| c.get("expr"))
                .is_some_and(Value::is_truthy)
        })
    });
    if expression_consumer {
        return Ok(());
    }
    let comp_count = match cx.root().get("comps").map(pm_model::Node::kind) {
        Some(pm_model::NodeKind::Object(m)) => m.len(),
        _ => 1,
    };
    let nested_or_mapped = comp_count > 1
        || layers.iter().any(|(_, l)| {
            values::text(l, "type") == Some("precomp")
                || l.get("d")
                    .and_then(|d| d.get("timeRemap"))
                    .is_some_and(Value::is_truthy)
                || l.get("d")
                    .and_then(|d| d.get("stretch"))
                    .is_some_and(|v| !v.is_null())
                || l.get("timeStretch").is_some_and(|v| !v.is_null())
        });
    let mut touched: Vec<usize> = Vec::new();
    for (i, path) in refs {
        let (loc, layer) = &layers[i];
        let id = values::text(layer, "id").unwrap_or_default().to_owned();
        if cx.check_lock(loc, &id).is_err() {
            continue;
        }
        let Some(steps) = channels::find(layer, &path) else {
            continue;
        };
        let prop = channels::get(layer, &steps).cloned().unwrap_or_default();
        let structured = [
            "g.",
            "mp.",
            "ta.",
            "ts.",
            "u.",
            "x.",
            "transitionIn.",
            "transitionOut.",
        ]
        .iter()
        .any(|p| path.starts_with(p));
        let t = values::text(layer, "type").unwrap_or_default();
        let supported = !structured
            && !["extension", "shader", "audio", "precomp"].contains(&t)
            && prop.get("v").and_then(Value::as_f64).is_some();
        if !supported || prop.get("expr").is_some_and(Value::is_truthy) {
            continue;
        }
        let all: Vec<&Value> = layers
            .iter()
            .filter(|(l, _)| l.comp_id == loc.comp_id)
            .map(|(_, v)| v)
            .collect();
        let parent_rig = t == "group"
            || all.iter().any(|o| {
                ["parent", "group", "matteSource"]
                    .iter()
                    .any(|f| values::text(o, f) == Some(id.as_str()))
            });
        let comp = cx.comp_node(loc.comp_id.as_deref())?;
        let comp_num = |k: &str| comp.get(k).and_then(crate::model::number_of);
        let options = SimplifyOptions {
            fps: comp_num("fps").unwrap_or(30.0),
            from: values::num(layer, "from").unwrap_or(0.0),
            duration: values::num(layer, "dur").unwrap_or(0.0),
            composition_duration: comp_num("dur").unwrap_or(0.0),
            shutter: comp_num("shutter"),
            trim_outside: !nested_or_mapped
                && !parent_rig
                && !values::flag(layer, "threeD")
                && layer
                    .get("p")
                    .and_then(|p| p.get(&path))
                    .is_some_and(Value::is_truthy),
        };
        if let Some(next) = simplify_track(track::keys_of(&prop), &options) {
            let layer = &mut layers[i].1;
            let slot = channels::get_mut(layer, &steps).expect("found");
            values::obj(slot).insert("kf", Value::Array(next));
            if !touched.contains(&i) {
                touched.push(i);
            }
        }
    }
    for i in touched {
        let (loc, layer) = layers[i].clone();
        cx.put_layer(&loc, layer)?;
    }
    Ok(())
}
