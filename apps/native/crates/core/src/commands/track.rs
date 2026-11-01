//! Keyframe tracks (`prop.kf`) with the old app's semantics.
//!
//! Ports `PM.KF`, `setKeyOn`, `setKeysOn`, `PM.removeKey` and
//! `PM.normalizeKeyframes` (`legacy/core/anim.ts`, `model.ts`),
//! `temporalKeys` (`legacy/core/temporal-bridge.ts`), and the edit half of
//! `core/anim/temporal-ease.ts` (`adoptTemporalEase`, `normalizeKeyEase`,
//! `easyEase`, `setInterpolation`, `setHold`, `applyUnitCurve`,
//! `applyContinuity`, `setHandleFromPoint`). The evaluation half is
//! `pm_eval::temporal`, which these functions reuse.
//!
//! Keys are plain `Value` objects so fields the core does not know (springs,
//! extension data) survive every edit.

// `!(x > y)` is deliberate: true for NaN, as in JavaScript.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use pm_eval::temporal::{
    self, AUTO_INFLUENCE, EASY_EASE_INFLUENCE, EPSILON, EaseKeyframe, Interp, LINEAR_INFLUENCE,
    Side, TemporalEase, clamp_influence, ease_from_unit_handle, make_ease,
};
use pm_model::{Map, Value};

use super::values::{js_round, num, s, snap};

/// `typeof v === 'number' && Number.isFinite(v)`.
pub(crate) fn numeric(v: Option<&Value>) -> bool {
    v.and_then(Value::as_f64).is_some_and(f64::is_finite)
}

/// `Number(v)` for the values keys hold.
pub(crate) fn js_number(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => *n,
        Value::String(s) => pm_model::json::string_to_number(&s.to_string_lossy()),
        Value::Array(a) if a.is_empty() => 0.0,
        Value::Array(a) if a.len() == 1 => js_number(&a[0]),
        _ => f64::NAN,
    }
}

fn pair(v: [f64; 2]) -> Value {
    Value::Array(vec![Value::Number(v[0]), Value::Number(v[1])])
}

/// `PM.KF(t, v, ease)`: a key with legacy unit handles (numeric tracks are
/// converted to native ease by [`temporal_keys`]).
pub(crate) fn legacy_key(t: f64, v: Value, ease: &str, id: String) -> Value {
    let (eo, ei) = pm_eval::easing::handles(ease);
    let mut m = Map::new();
    m.insert("t", Value::Number(t));
    m.insert("v", v);
    m.insert("eo", pair(eo));
    m.insert("ei", pair(ei));
    m.insert("hold", Value::Bool(false));
    m.insert("i", s(&id));
    Value::Object(m)
}

fn ease_value(e: TemporalEase) -> Value {
    let mut m = Map::new();
    m.insert("speed", Value::Number(e.speed));
    m.insert("influence", Value::Number(e.influence));
    Value::Object(m)
}

fn interp_of(key: &Value, field: &str) -> Interp {
    Interp::from_name(key.get(field).and_then(Value::as_str).map(str::as_bytes))
}

fn is_temporal_ease(v: Option<&Value>) -> bool {
    v.is_some_and(|e| {
        e.as_object().is_some()
            && e.get("speed")
                .and_then(Value::as_f64)
                .is_some_and(f64::is_finite)
            && e.get("influence")
                .and_then(Value::as_f64)
                .is_some_and(f64::is_finite)
    })
}

fn has_native_ease(key: &Value) -> bool {
    interp_of(key, "inInterp").is_valid()
        && interp_of(key, "outInterp").is_valid()
        && is_temporal_ease(key.get("inEase"))
        && is_temporal_ease(key.get("outEase"))
}

fn read_ease(v: Option<&Value>) -> TemporalEase {
    let speed = v
        .and_then(|e| e.get("speed"))
        .map(js_number)
        .unwrap_or(f64::NAN);
    let influence = if is_temporal_ease(v) {
        v.and_then(|e| e.get("influence"))
            .map(js_number)
            .unwrap_or(LINEAR_INFLUENCE)
    } else {
        LINEAR_INFLUENCE
    };
    make_ease(speed, influence)
}

/// The key as the AE model sees it.
pub(crate) fn ease_key(key: &Value) -> EaseKeyframe {
    let v = key
        .get("v")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite());
    EaseKeyframe {
        t: key.get("t").map(js_number).unwrap_or(f64::NAN),
        v,
        in_interp: match interp_of(key, "inInterp") {
            Interp::Other => Interp::Linear,
            i => i,
        },
        out_interp: match interp_of(key, "outInterp") {
            Interp::Other => Interp::Linear,
            i => i,
        },
        in_ease: read_ease(key.get("inEase")),
        out_ease: read_ease(key.get("outEase")),
        auto_bezier: key.get("autoBezier").and_then(Value::as_bool) == Some(true),
        continuous: key.get("continuous").and_then(Value::as_bool) == Some(true),
    }
}

/// Writes the AE fields of `e` onto the stored key.
pub(crate) fn write_ease(key: &mut Value, e: &EaseKeyframe) {
    let m = super::values::obj(key);
    m.insert("inInterp", s(e.in_interp.name().unwrap_or("linear")));
    m.insert("outInterp", s(e.out_interp.name().unwrap_or("linear")));
    m.insert("inEase", ease_value(e.in_ease));
    m.insert("outEase", ease_value(e.out_ease));
    m.insert("autoBezier", Value::Bool(e.auto_bezier));
    m.insert("continuous", Value::Bool(e.continuous));
}

fn legacy_handle(v: Option<&Value>, fallback: [f64; 2]) -> [f64; 2] {
    match v.and_then(Value::as_array) {
        Some(a) if a.len() >= 2 => {
            let (x, y) = (js_number(&a[0]), js_number(&a[1]));
            if x.is_finite() && y.is_finite() {
                [x, y]
            } else {
                fallback
            }
        }
        _ => fallback,
    }
}

/// `adoptTemporalEase(keys)` then `refreshAutoBezier`: legacy `eo`/`ei`/
/// `hold` keys become native ease measured against their segments.
pub(crate) fn adopt(keys: &mut [Value]) {
    for index in 0..keys.len() {
        if has_native_ease(&keys[index]) {
            let e = ease_key(&keys[index]);
            write_ease(&mut keys[index], &e);
            continue;
        }
        let key_t = keys[index].get("t").map(js_number).unwrap_or(f64::NAN);
        let value = keys[index].get("v").map(js_number).unwrap_or(f64::NAN);
        let out_handle = legacy_handle(keys[index].get("eo"), [0.0, 0.0]);
        let in_handle = legacy_handle(keys[index].get("ei"), [1.0, 1.0]);
        let hold = keys[index].get("hold").and_then(Value::as_bool) == Some(true);
        let next = keys.get(index + 1).map(|k| {
            (
                k.get("t").map(js_number).unwrap_or(f64::NAN),
                k.get("v").map(js_number).unwrap_or(f64::NAN),
            )
        });
        let previous = index.checked_sub(1).map(|i| {
            (
                keys[i].get("t").map(js_number).unwrap_or(f64::NAN),
                keys[i].get("v").map(js_number).unwrap_or(f64::NAN),
            )
        });
        let linear = make_ease(0.0, LINEAR_INFLUENCE);
        let (out_interp, out_ease) = if hold {
            (Interp::Hold, linear)
        } else if next.is_none() || out_handle == [0.0, 0.0] {
            (Interp::Linear, linear)
        } else {
            let (nt, nv) = next.expect("checked");
            let dv = if nv.is_finite() && value.is_finite() {
                nv - value
            } else {
                0.0
            };
            (
                Interp::Bezier,
                ease_from_unit_handle(out_handle, nt - key_t, dv, Side::Out),
            )
        };
        let (in_interp, in_ease) = match previous {
            Some((pt, pv)) if in_handle != [1.0, 1.0] => {
                let dv = if pv.is_finite() && value.is_finite() {
                    value - pv
                } else {
                    0.0
                };
                (
                    Interp::Bezier,
                    ease_from_unit_handle(in_handle, key_t - pt, dv, Side::In),
                )
            }
            _ => (Interp::Linear, linear),
        };
        let continuous =
            keys[index].get("bezierMode").and_then(Value::as_str) == Some("continuous");
        let e = EaseKeyframe {
            t: key_t,
            v: value.is_finite().then_some(value),
            in_interp,
            out_interp,
            in_ease,
            out_ease,
            auto_bezier: false,
            continuous,
        };
        let key = &mut keys[index];
        write_ease(key, &e);
        if let Some(m) = key.as_object_mut() {
            m.remove("eo");
            m.remove("ei");
            m.remove("hold");
            m.remove("bezierMode");
        }
    }
    refresh_auto_bezier(keys);
}

/// `refreshAutoBezier(keys)` on stored keys.
pub(crate) fn refresh_auto_bezier(keys: &mut [Value]) {
    if !keys
        .iter()
        .any(|k| k.get("autoBezier").and_then(Value::as_bool) == Some(true))
    {
        return;
    }
    let mut eased: Vec<EaseKeyframe> = keys.iter().map(ease_key).collect();
    temporal::refresh_auto_bezier(&mut eased);
    for (key, e) in keys.iter_mut().zip(&eased) {
        if e.auto_bezier {
            write_ease(key, e);
        }
    }
}

/// `temporalKeys(keys)`: numeric tracks are kept in the native model.
pub(crate) fn temporal_keys(keys: &mut [Value]) {
    if keys.is_empty() || keys.iter().any(|k| !numeric(k.get("v"))) {
        return;
    }
    adopt(keys);
}

fn key_t(k: &Value) -> f64 {
    k.get("t").map(js_number).unwrap_or(f64::NAN)
}

/// `p.kf.sort((a, b) => a.t - b.t)` (stable; NaN comparisons keep order).
pub(crate) fn sort_keys(keys: &mut [Value]) {
    keys.sort_by(|a, b| {
        key_t(a)
            .partial_cmp(&key_t(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// The keys of a channel object (an empty list when it has none).
pub(crate) fn keys_mut(prop: &mut Value) -> &mut Vec<Value> {
    let m = super::values::obj(prop);
    if m.get("kf").and_then(Value::as_array).is_none() {
        m.insert("kf", Value::Array(Vec::new()));
    }
    m.get_mut("kf")
        .and_then(Value::as_array_mut)
        .expect("just made an array")
}

pub(crate) fn keys_of(prop: &Value) -> &[Value] {
    prop.get("kf")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// `safeFps` of `setKeyOn`.
pub(crate) fn safe_fps(fps: f64) -> f64 {
    if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    }
}

/// `setKeyOn(p, tLocal, value, ease, fps, prepare)`: updates the key within
/// half a frame of the snapped time or inserts a new one. Returns the key's
/// id and whether it was created.
pub(crate) fn set_key_on(
    prop: &mut Value,
    t_local: f64,
    value: Value,
    ease: &str,
    fps: f64,
    prepare: bool,
    new_id: impl FnOnce() -> String,
) -> (String, bool) {
    let fps = safe_fps(fps);
    let t = snap(if t_local.is_finite() { t_local } else { 0.0 }, fps).max(0.0);
    let keys = keys_mut(prop);
    let found = keys.iter().position(|k| (key_t(k) - t).abs() < 0.5 / fps);
    let (id, created) = match found {
        Some(i) => {
            super::values::obj(&mut keys[i]).insert("v", value);
            (key_id(&keys[i]), false)
        }
        None => {
            let id = new_id();
            keys.push(legacy_key(t, value, ease, id.clone()));
            sort_keys(keys);
            (id, true)
        }
    };
    if prepare {
        temporal_keys(keys);
    }
    (id, created)
}

pub(crate) fn key_id(k: &Value) -> String {
    k.get("i")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn index_of(keys: &[Value], id: &str) -> Option<usize> {
    keys.iter()
        .position(|k| k.get("i").and_then(Value::as_str) == Some(id))
}

/// The finishing pass of `setKeysOn` for keys written with a named ease:
/// hold flags, then `applyUnitCurve` (numeric) or the legacy handles.
pub(crate) fn apply_presets(keys: &mut [Value], edits: &[(String, String, Option<bool>)]) {
    temporal_keys(keys);
    for (id, ease, hold) in edits {
        let Some(index) = index_of(keys, id) else {
            continue;
        };
        if let Some(h) = hold {
            set_hold_flag(keys, index, *h);
        }
        let (eo, ei) = pm_eval::easing::handles(ease);
        if numeric(keys[index].get("v")) {
            apply_unit_curve(keys, index, [eo[0], eo[1], ei[0], ei[1]]);
        } else {
            let m = super::values::obj(&mut keys[index]);
            m.insert("eo", pair(eo));
            m.insert("ei", pair(ei));
        }
    }
}

/// `key.hold = h` through the bridge: native `outInterp` on numeric keys,
/// the legacy flag otherwise.
pub(crate) fn set_hold_flag(keys: &mut [Value], index: usize, hold: bool) {
    if has_native_ease(&keys[index]) {
        let mut e = ease_key(&keys[index]);
        if hold {
            e.out_interp = Interp::Hold;
        } else if e.out_interp == Interp::Hold {
            e.out_interp = Interp::Linear;
        }
        write_ease(&mut keys[index], &e);
    } else {
        super::values::obj(&mut keys[index]).insert("hold", Value::Bool(hold));
    }
}

/// Applies `f` to the AE view of the track and writes the keys back.
fn with_ease(keys: &mut [Value], f: impl FnOnce(&mut [EaseKeyframe])) {
    let mut eased: Vec<EaseKeyframe> = keys.iter().map(ease_key).collect();
    f(&mut eased);
    for (key, e) in keys.iter_mut().zip(&eased) {
        write_ease(key, e);
    }
}

/// `applyUnitCurve(keys, index, curve)`.
pub(crate) fn apply_unit_curve(keys: &mut [Value], index: usize, curve: [f64; 4]) {
    with_ease(keys, |k| {
        let next = k.get(index + 1).copied();
        let previous = index.checked_sub(1).map(|i| k[i]);
        let key = &mut k[index];
        let is_linear = curve[0] == curve[1] && curve[2] == curve[3];
        key.auto_bezier = false;
        key.continuous = false;
        let interp = if is_linear {
            Interp::Linear
        } else {
            Interp::Bezier
        };
        if key.out_interp != Interp::Hold {
            key.out_interp = interp;
        }
        if key.in_interp != Interp::Hold {
            key.in_interp = interp;
        }
        if is_linear {
            return;
        }
        key.out_ease = match (next, key.v) {
            (Some(n), Some(v)) if n.v.is_some() => ease_from_unit_handle(
                [curve[0], curve[1]],
                n.t - key.t,
                n.v.expect("checked") - v,
                Side::Out,
            ),
            _ => make_ease(0.0, clamp_influence(curve[0] * 100.0)),
        };
        key.in_ease = match (previous, key.v) {
            (Some(p), Some(v)) if p.v.is_some() => ease_from_unit_handle(
                [curve[2], curve[3]],
                key.t - p.t,
                v - p.v.expect("checked"),
                Side::In,
            ),
            _ => make_ease(0.0, clamp_influence((1.0 - curve[2]) * 100.0)),
        };
    });
}

/// `easyEase(keys, index, side)`.
pub(crate) fn easy_ease(keys: &mut [Value], index: usize, side: super::keyframes::EaseSide) {
    with_ease(keys, |k| {
        let key = &mut k[index];
        key.auto_bezier = false;
        key.continuous = false;
        if side != super::keyframes::EaseSide::Out {
            key.in_interp = Interp::Bezier;
            key.in_ease = make_ease(0.0, EASY_EASE_INFLUENCE);
        }
        if side != super::keyframes::EaseSide::In {
            key.out_interp = Interp::Bezier;
            key.out_ease = make_ease(0.0, EASY_EASE_INFLUENCE);
        }
    });
}

fn chord(a: &EaseKeyframe, b: &EaseKeyframe) -> f64 {
    temporal::chord_speed(a, b)
}

/// `setInterpolation(keys, index, side, type)`.
pub(crate) fn set_interpolation(
    keys: &mut [Value],
    index: usize,
    side: super::keyframes::EaseSide,
    kind: super::keyframes::Interpolation,
) {
    use super::keyframes::{EaseSide, Interpolation};
    let do_in = side != EaseSide::Out;
    let do_out = side != EaseSide::In;
    with_ease(keys, |k| {
        let previous = index.checked_sub(1).map(|i| k[i]);
        let next = k.get(index + 1).copied();
        let key = &mut k[index];
        match kind {
            Interpolation::Auto | Interpolation::Continuous => {
                let auto = kind == Interpolation::Auto;
                key.auto_bezier = auto;
                key.continuous = true;
                let speed = if auto {
                    temporal::auto_bezier_speed(previous.as_ref(), key, next.as_ref())
                } else if key.out_interp == Interp::Bezier {
                    key.out_ease.speed
                } else if key.in_interp == Interp::Bezier {
                    key.in_ease.speed
                } else if let Some(n) = next {
                    chord(key, &n)
                } else if let Some(p) = previous {
                    chord(&p, key)
                } else {
                    0.0
                };
                let influence = |e: TemporalEase| {
                    if auto || !e.influence.is_finite() {
                        AUTO_INFLUENCE
                    } else {
                        e.influence
                    }
                };
                if do_in {
                    key.in_ease = make_ease(speed, influence(key.in_ease));
                    key.in_interp = Interp::Bezier;
                }
                if do_out {
                    key.out_ease = make_ease(speed, influence(key.out_ease));
                    key.out_interp = Interp::Bezier;
                }
            }
            _ => {
                let interp = match kind {
                    Interpolation::Linear => Interp::Linear,
                    Interpolation::Hold => Interp::Hold,
                    _ => Interp::Bezier,
                };
                key.auto_bezier = false;
                if interp != Interp::Bezier {
                    key.continuous = false;
                }
                if do_in {
                    key.in_interp = interp;
                    if interp == Interp::Bezier {
                        key.in_ease = make_ease(key.in_ease.speed, AUTO_INFLUENCE);
                    }
                }
                if do_out {
                    key.out_interp = interp;
                    if interp == Interp::Bezier {
                        key.out_ease = make_ease(key.out_ease.speed, AUTO_INFLUENCE);
                    }
                }
            }
        }
    });
}

/// `setHold(keys, index, hold)`.
pub(crate) fn set_hold(keys: &mut [Value], index: usize, hold: bool) {
    with_ease(keys, |k| {
        let key = &mut k[index];
        if hold {
            key.out_interp = Interp::Hold;
            key.auto_bezier = false;
            key.continuous = false;
        } else if key.out_interp == Interp::Hold {
            key.out_interp = Interp::Linear;
        }
    });
}

/// `applyContinuity(key, edited)`.
fn apply_continuity(key: &mut EaseKeyframe, edited: Side) {
    if !key.continuous {
        return;
    }
    match edited {
        Side::In => {
            key.out_ease = make_ease(key.in_ease.speed, key.out_ease.influence);
            if key.out_interp == Interp::Linear {
                key.out_interp = Interp::Bezier;
            }
        }
        Side::Out => {
            key.in_ease = make_ease(key.out_ease.speed, key.in_ease.influence);
            if key.in_interp == Interp::Linear {
                key.in_interp = Interp::Bezier;
            }
        }
    }
}

/// A handle edit: `setHandleFromPoint` for a point, or the ease itself.
pub(crate) fn set_handle(
    keys: &mut [Value],
    index: usize,
    side: Side,
    value: &super::keyframes::HandleValue,
    split: bool,
) -> bool {
    use super::keyframes::HandleValue;
    let mut done = true;
    with_ease(keys, |k| {
        let neighbour = match side {
            Side::Out => k.get(index + 1).copied(),
            Side::In => index.checked_sub(1).map(|i| k[i]),
        };
        let key = &mut k[index];
        let ease = match value {
            HandleValue::Ease(e) => make_ease(e.speed, e.influence),
            HandleValue::Point { t, v } => {
                let Some(n) = neighbour else {
                    done = false;
                    return;
                };
                let dt = (n.t - key.t).abs();
                if !(dt > EPSILON) {
                    done = false;
                    return;
                }
                let reach = match side {
                    Side::Out => t - key.t,
                    Side::In => key.t - t,
                };
                let influence = clamp_influence(reach.abs() / dt * 100.0);
                let span = influence / 100.0 * dt;
                let key_value = key.v.unwrap_or(0.0);
                let rise = match side {
                    Side::Out => v - key_value,
                    Side::In => key_value - v,
                };
                make_ease(rise / span, influence)
            }
        };
        if key.auto_bezier {
            key.auto_bezier = false;
            key.continuous = true;
        }
        if split {
            key.continuous = false;
        }
        match side {
            Side::Out => {
                key.out_ease = ease;
                if key.out_interp == Interp::Linear {
                    key.out_interp = Interp::Bezier;
                }
            }
            Side::In => {
                key.in_ease = ease;
                if key.in_interp == Interp::Linear {
                    key.in_interp = Interp::Bezier;
                }
            }
        }
        apply_continuity(key, side);
    });
    done
}

/// `PM.removeKey(p, k)` for several ids: a channel losing its last key keeps
/// that key's value.
pub(crate) fn remove_keys(prop: &mut Value, ids: &std::collections::HashSet<&str>) -> bool {
    let before = keys_of(prop).len();
    let first = keys_of(prop).first().and_then(|k| k.get("v")).cloned();
    let keys = keys_mut(prop);
    keys.retain(|k| {
        !k.get("i")
            .and_then(Value::as_str)
            .is_some_and(|i| ids.contains(i))
    });
    let after = keys.len();
    if after == before {
        return false;
    }
    temporal_keys(keys);
    if before > 0
        && after == 0
        && let Some(v) = first
    {
        super::values::obj(prop).insert("v", v);
    }
    true
}

/// `round(x, 3)` (`PM.round`).
pub(crate) fn round3(x: f64) -> f64 {
    js_round(x * 1000.0) / 1000.0
}

/// The layer-local time of composition time `time` on a layer.
pub(crate) fn local_time(layer: &Value, time: f64) -> f64 {
    time - num(layer, "from").unwrap_or(0.0)
}
