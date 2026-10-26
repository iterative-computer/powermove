//! Keyframe track evaluation: `evalKfs` (`legacy/core/anim.ts`) with the
//! dense path of `legacy/core/temporal-bridge.ts`.
//!
//! A track is the raw `kf` array from the document. Evaluation at a local
//! time `t` follows the legacy branches exactly:
//!
//! 1. Not an array, or empty: `null`.
//! 2. A non-finite `t`: the first key's value (`kf[0]?.v ?? null`).
//! 3. **Temporal path** (`temporalEvaluate`): the first and last values are
//!    numbers. Keys are adopted into the AE model (`adoptTemporalEase`: legacy
//!    `eo`/`ei`/`hold` handles become speed and influence, measured against the
//!    segments they were saved in), auto-bezier tangents refreshed, and the
//!    segment around `t` evaluated as hold, linear, spring, or bezier.
//! 4. **Discrete path**: everything else. Strings and booleans step at the key;
//!    `#RRGGBB[AA]` colors on `c.color` and `c.strokeColor` interpolate per
//!    byte along the key's legacy `eo`/`ei` handles (or the `power` preset);
//!    springs apply; numeric segments inside a mixed track use the unadopted
//!    keys' stored interpolation.
//!
//! Raw values flow through JavaScript's operators ([`crate::js_sem`]): a
//! string key in a numeric track concatenates, `"5" + 2.5` is `"52.5"`. The
//! common case (every key time and value a finite number) runs on plain
//! `f64`s with the identical operation order.
//!
//! # Allowed differences
//!
//! The legacy code mutates the live keys while evaluating (adoption rewrites
//! `eo`/`ei` into speed and influence and installs accessors) and caches per
//! array object, so a few answers depend on evaluation history. The engine is
//! a pure function of the stored JSON and gives the answer of a freshly loaded
//! project:
//!
//! * A mixed track whose keys were all numbers at an earlier evaluation keeps
//!   accessor-backed `hold`/`eo`/`ei` in JavaScript; the engine reads the stored
//!   fields.
//! * `temporalKeys` throws the first time it meets a `null` key before the
//!   first non-number value, then never again for that array and version; the
//!   engine follows the steady state (no throw).

#![allow(clippy::neg_cmp_op_on_partial_ord)]

use std::sync::{Arc, OnceLock};

use pm_model::{Node, NodeKind};

use crate::easing::{SpringConfig, SpringTables, bezier, handles, spring_lookup};
use crate::js_sem::{self, JsResult, JsThrow, JsValue};
use crate::node::{self, Got};
use crate::temporal::{
    AUTO_INFLUENCE, EPSILON, EaseKeyframe, Interp, LINEAR_INFLUENCE, Side, TemporalEase,
    auto_bezier_speed, ease_from_unit_handle, make_ease, solve_curve_param,
};

/// Whether a channel key interpolates hex colors (`c.color`, `c.strokeColor`).
pub fn interpolates_color(key: &[u8]) -> bool {
    key == b"c.color" || key == b"c.strokeColor"
}

/// A key's time as stored. Only strings and objects behave differently from
/// their numeric value (in `a.t + x`, which concatenates).
#[derive(Clone, Debug, PartialEq)]
enum KeyTime {
    Number(f64),
    Raw(JsValue),
}

impl KeyTime {
    fn from_got(t: &Got<'_>) -> (KeyTime, f64) {
        let js = t.to_js();
        match js {
            JsValue::String(_) | JsValue::Object(_) => {
                let n = js.to_number();
                (KeyTime::Raw(js), n)
            }
            other => {
                let n = other.to_number();
                (KeyTime::Number(n), n)
            }
        }
    }

    /// `ToNumber(t + x)`.
    fn plus(&self, x: f64) -> f64 {
        match self {
            KeyTime::Number(t) => t + x,
            KeyTime::Raw(raw) => js_sem::add(raw, &JsValue::Number(x)).to_number(),
        }
    }
}

/// A key as segment construction reads it (`temporalValue`).
#[derive(Clone, Debug)]
struct SegKey {
    t: KeyTime,
    tn: f64,
    v: JsValue,
    /// `numeric(v)`.
    vn: Option<f64>,
    in_interp: Interp,
    out_interp: Interp,
    in_ease: TemporalEase,
    out_ease: TemporalEase,
    auto_bezier: bool,
    spring: Option<SpringConfig>,
}

impl SegKey {
    fn ease_key(&self) -> EaseKeyframe {
        EaseKeyframe {
            t: self.tn,
            v: self.vn,
            in_interp: self.in_interp,
            out_interp: self.out_interp,
            in_ease: self.in_ease,
            out_ease: self.out_ease,
            auto_bezier: self.auto_bezier,
            continuous: false,
        }
    }
}

fn interp_of(node: Option<&Node>) -> Interp {
    Interp::from_name(node::string(node).map(|s| s.as_bytes()))
}

/// `makeEase(ease?.speed, ease?.influence)` on a raw ease member.
fn raw_ease(ease: Option<&Node>) -> TemporalEase {
    make_ease(
        node::to_number(node::member(ease, "speed")),
        node::to_number(node::member(ease, "influence")),
    )
}

fn spring_of(key: &Node) -> Option<SpringConfig> {
    let spring = node::member(Some(key), "spring");
    node::truthy(spring).then(|| SpringConfig::from_node(spring))
}

/// Reads an object key as stored (no adoption).
fn raw_seg_key(key: &Node) -> SegKey {
    let k = Some(key);
    let (t, tn) = KeyTime::from_got(&node::get(key, b"t"));
    let v = node::js_value_opt(node::member(k, "v"));
    let vn = v.is_finite_number().then(|| v.to_number());
    SegKey {
        t,
        tn,
        v,
        vn,
        in_interp: interp_of(node::member(k, "inInterp")),
        out_interp: interp_of(node::member(k, "outInterp")),
        in_ease: raw_ease(node::member(k, "inEase")),
        out_ease: raw_ease(node::member(k, "outEase")),
        auto_bezier: false,
        spring: spring_of(key),
    }
}

/// `isTemporalEase(value)`.
fn is_temporal_ease(ease: Option<&Node>) -> bool {
    matches!(
        ease.map(Node::kind),
        Some(NodeKind::Object(_) | NodeKind::Array(_))
    ) && node::to_number(node::member(ease, "speed")).is_finite()
        && node::to_number(node::member(ease, "influence")).is_finite()
}

/// `legacyHandle(value, fallback)`.
fn legacy_handle(value: Option<&Node>, fallback: [f64; 2]) -> [f64; 2] {
    if let Some(items) = node::array(value) {
        let x = node::to_number(items.first());
        let y = node::to_number(items.get(1));
        if x.is_finite() && y.is_finite() {
            return [x, y];
        }
    }
    fallback
}

fn is_object(key: &Node) -> bool {
    matches!(key.kind(), NodeKind::Object(_) | NodeKind::Array(_))
}

/// Whether `adoptTemporalEase(keys)` throws, and where: on a key that is
/// not an object (reading from `null`, or writing to a primitive in strict
/// mode).
fn adoption_error(keys: &[Node]) -> Option<JsThrow> {
    let index = keys.iter().position(|key| !is_object(key))?;
    Some(JsThrow::type_error(format!(
        "keyframe {index} is not an object"
    )))
}

/// Key `index` as `adoptTemporalEase(keys)` followed by `refreshAutoBezier`
/// leaves it, without mutating the document. Every key must be an object
/// ([`adoption_error`]). It reads only keys `index - 1` to `index + 1`:
/// adoption measures legacy handles against the segments beside the key,
/// and an auto-bezier tangent depends only on its neighbours' times and
/// values, which adoption never changes.
fn adopt_key(keys: &[Node], index: usize) -> SegKey {
    let mut seg = adopt_ease(keys, index);
    if seg.auto_bezier {
        // refreshAutoBezier
        let previous = index
            .checked_sub(1)
            .map(|i| raw_seg_key(&keys[i]).ease_key());
        let next = keys.get(index + 1).map(|k| raw_seg_key(k).ease_key());
        let speed = auto_bezier_speed(previous.as_ref(), &seg.ease_key(), next.as_ref());
        let ease = make_ease(speed, AUTO_INFLUENCE);
        if seg.in_interp != Interp::Hold {
            seg.in_interp = Interp::Bezier;
            seg.in_ease = ease;
        }
        if seg.out_interp != Interp::Hold {
            seg.out_interp = Interp::Bezier;
            seg.out_ease = ease;
        }
    }
    seg
}

/// `adoptTemporalEase` (with `normalizeKeyEase`) for key `index`.
fn adopt_ease(keys: &[Node], index: usize) -> SegKey {
    let key = &keys[index];
    let k = Some(key);
    let mut seg = raw_seg_key(key);
    let in_i = interp_of(node::member(k, "inInterp"));
    let out_i = interp_of(node::member(k, "outInterp"));
    let native = in_i.is_valid()
        && out_i.is_valid()
        && is_temporal_ease(node::member(k, "inEase"))
        && is_temporal_ease(node::member(k, "outEase"));
    if native {
        // normalizeKeyEase
        seg.in_interp = in_i;
        seg.out_interp = out_i;
        seg.auto_bezier = matches!(
            node::member(k, "autoBezier").map(Node::kind),
            Some(NodeKind::Bool(true))
        );
        return seg;
    }
    let previous = index.checked_sub(1).map(|i| &keys[i]);
    let next = keys.get(index + 1);
    let out_handle = legacy_handle(node::member(k, "eo"), [0.0, 0.0]);
    let in_handle = legacy_handle(node::member(k, "ei"), [1.0, 1.0]);
    let value = node::to_number(node::member(k, "v"));
    let hold = matches!(
        node::member(k, "hold").map(Node::kind),
        Some(NodeKind::Bool(true))
    );
    if hold {
        seg.out_interp = Interp::Hold;
        seg.out_ease = make_ease(0.0, LINEAR_INFLUENCE);
    } else if !node::truthy(next) || (out_handle[0] == 0.0 && out_handle[1] == 0.0) {
        seg.out_interp = Interp::Linear;
        seg.out_ease = make_ease(0.0, LINEAR_INFLUENCE);
    } else {
        let next = next.expect("truthy");
        let next_value = node::to_number(node::get(next, b"v").node());
        let dt = node::get(next, b"t").to_js().to_number() - node::to_number(node::member(k, "t"));
        let dv = if next_value.is_finite() && value.is_finite() {
            next_value - value
        } else {
            0.0
        };
        seg.out_interp = Interp::Bezier;
        seg.out_ease = ease_from_unit_handle(out_handle, dt, dv, Side::Out);
    }
    if !node::truthy(previous) || (in_handle[0] == 1.0 && in_handle[1] == 1.0) {
        seg.in_interp = Interp::Linear;
        seg.in_ease = make_ease(0.0, LINEAR_INFLUENCE);
    } else {
        let previous = previous.expect("truthy");
        let previous_value = node::to_number(node::get(previous, b"v").node());
        let dt =
            node::to_number(node::member(k, "t")) - node::get(previous, b"t").to_js().to_number();
        let dv = if previous_value.is_finite() && value.is_finite() {
            value - previous_value
        } else {
            0.0
        };
        seg.in_interp = Interp::Bezier;
        seg.in_ease = ease_from_unit_handle(in_handle, dt, dv, Side::In);
    }
    seg.auto_bezier = false;
    seg
}

/// `c.v` of a prepared segment: the first key's raw value.
#[derive(Clone, Debug, PartialEq)]
enum Base {
    Number(f64),
    Raw(JsValue),
}

impl Base {
    fn of(v: &JsValue) -> Base {
        match v {
            JsValue::Number(n) => Base::Number(*n),
            other => Base::Raw(other.clone()),
        }
    }

    fn js(&self) -> JsValue {
        match self {
            Base::Number(n) => JsValue::Number(*n),
            Base::Raw(v) => v.clone(),
        }
    }

    /// `c.v + x`.
    fn plus(&self, x: f64) -> JsValue {
        match self {
            Base::Number(n) => JsValue::Number(n + x),
            Base::Raw(v) => js_sem::add(v, &JsValue::Number(x)),
        }
    }

    /// `x + c.v`.
    fn plus_rev(&self, x: f64) -> JsValue {
        match self {
            Base::Number(n) => JsValue::Number(x + n),
            Base::Raw(v) => js_sem::add(&JsValue::Number(x), v),
        }
    }
}

#[derive(Clone, Debug)]
enum Mode {
    Hold,
    Linear,
    Spring(Arc<[f32]>),
    Bezier,
}

/// One segment prepared the way `temporalValue` caches it.
#[derive(Clone, Debug)]
struct Curve {
    mode: Mode,
    start: f64,
    span: f64,
    base: Base,
    d: f64,
    x1: f64,
    x2: f64,
    ay: f64,
    by: f64,
    cy: f64,
}

/// `resolvedOutEase` / `resolvedInEase` on segment keys.
fn resolved(interp: Interp, ease: TemporalEase, a: &SegKey, b: &SegKey) -> TemporalEase {
    if interp != Interp::Bezier {
        let dt = b.tn - a.tn;
        let speed = match (a.vn, b.vn) {
            (Some(va), Some(vb)) if dt > EPSILON => (vb - va) / dt,
            _ => 0.0,
        };
        return TemporalEase {
            speed,
            influence: LINEAR_INFLUENCE,
        };
    }
    make_ease(ease.speed, ease.influence)
}

impl Curve {
    /// `spring_table`: the table of a spring segment's config.
    fn new(a: &SegKey, b: &SegKey, spring_table: impl FnOnce(SpringConfig) -> Arc<[f32]>) -> Curve {
        let span = b.tn - a.tn;
        let start = a.tn;
        let mode = if a.out_interp == Interp::Hold || b.in_interp == Interp::Hold {
            Mode::Hold
        } else if a.out_interp == Interp::Linear && b.in_interp == Interp::Linear {
            Mode::Linear
        } else if let Some(cfg) = a.spring {
            Mode::Spring(spring_table(cfg))
        } else {
            Mode::Bezier
        };
        let a_vn = a.v.to_number();
        let b_vn = b.v.to_number();
        // segmentControlPoints(a, b)
        let dt = b.tn - a.tn;
        let va = a.vn.unwrap_or(0.0);
        let vb = b.vn.unwrap_or(va);
        let out = resolved(a.out_interp, a.out_ease, a, b);
        let incoming = resolved(b.in_interp, b.in_ease, a, b);
        let f_out = out.influence / 100.0;
        let f_in = incoming.influence / 100.0;
        let p1t = a.t.plus(f_out * dt);
        let p1v = va + f_out * dt * out.speed;
        let p2t = b.tn - f_in * dt;
        let p2v = vb - f_in * dt * incoming.speed;
        let by_sum = match &a.v {
            JsValue::Number(n) => (p2v - 2.0 * p1v) + n,
            raw => js_sem::add(&JsValue::Number(p2v - 2.0 * p1v), raw).to_number(),
        };
        Curve {
            mode,
            start,
            span,
            base: Base::of(&a.v),
            d: b_vn - a_vn,
            x1: (p1t - start) / span,
            x2: (p2t - start) / span,
            cy: 3.0 * (p1v - a_vn),
            by: 3.0 * by_sum,
            ay: b_vn - 3.0 * p2v + 3.0 * p1v - a_vn,
        }
    }

    #[inline]
    fn value(&self, time: f64) -> JsValue {
        match &self.mode {
            Mode::Hold => self.base.js(),
            Mode::Linear => self.base.plus(self.d * (time - self.start) / self.span),
            Mode::Spring(table) => self
                .base
                .plus(self.d * spring_lookup(table, time - self.start)),
            Mode::Bezier => {
                let u = solve_curve_param(self.x1, self.x2, (time - self.start) / self.span);
                self.base
                    .plus_rev(((self.ay * u + self.by) * u + self.cy) * u)
            }
        }
    }

    /// The value when the base is a number.
    #[inline]
    fn value_number(&self, time: f64, base: f64) -> f64 {
        match &self.mode {
            Mode::Hold => base,
            Mode::Linear => base + self.d * (time - self.start) / self.span,
            Mode::Spring(table) => base + self.d * spring_lookup(table, time - self.start),
            Mode::Bezier => {
                let u = solve_curve_param(self.x1, self.x2, (time - self.start) / self.span);
                ((self.ay * u + self.by) * u + self.cy) * u + base
            }
        }
    }
}

/// A track on the temporal path, prepared once per content revision.
///
/// Preparing reads only every key's time and value (the segment search and
/// the answers outside the keys); a segment's curve (its keys' adopted
/// easing) is made the first time a time inside it is asked for. A layer
/// first drawn at a frame reads a few segments of tracks thousands of keys
/// long, and a frame stays exact: each curve is the one preparing every
/// segment at once made.
#[derive(Clone, Debug)]
pub struct TemporalTrack {
    keys: Arc<Vec<Node>>,
    times: Vec<f64>,
    values: Vec<f64>,
    curves: Box<[OnceLock<Curve>]>,
    /// Every segment base is a number, so every answer is a number.
    numeric: bool,
}

impl TemporalTrack {
    /// `keys` must all be objects ([`adoption_error`]).
    fn new(keys: Arc<Vec<Node>>) -> TemporalTrack {
        let mut times = Vec::with_capacity(keys.len());
        let mut values = Vec::with_capacity(keys.len());
        let mut numeric = true;
        for (index, key) in keys.iter().enumerate() {
            times.push(KeyTime::from_got(&node::get(key, b"t")).1);
            let v = node::js_value_opt(node::member(Some(key), "v"));
            values.push(v.to_number());
            // A segment's base is its first key's value.
            numeric &= index + 1 == keys.len() || matches!(v, JsValue::Number(_));
        }
        let curves = (1..keys.len()).map(|_| OnceLock::new()).collect();
        TemporalTrack {
            keys,
            times,
            values,
            curves,
            numeric,
        }
    }

    /// The curve of segment `index` (between keys `index` and `index + 1`).
    #[inline]
    fn curve(&self, index: usize) -> &Curve {
        self.curves[index].get_or_init(|| {
            let a = adopt_key(&self.keys, index);
            let b = adopt_key(&self.keys, index + 1);
            Curve::new(&a, &b, crate::easing::shared_spring_table)
        })
    }

    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// Whether every answer is a number ([`TemporalTrack::value_number`]).
    pub fn is_numeric(&self) -> bool {
        self.numeric
    }

    #[inline]
    fn segment(&self, time: f64) -> Result<usize, f64> {
        let times = &self.times;
        let n = times.len();
        if time <= times[0] {
            return Err(self.values[0]);
        }
        if time >= times[n - 1] {
            return Err(self.values[n - 1]);
        }
        let mut lo = 0usize;
        let mut hi = n - 1;
        while hi - lo > 1 {
            let m = (lo + hi) >> 1;
            if times[m] <= time {
                lo = m;
            } else {
                hi = m;
            }
        }
        Ok(lo)
    }

    /// `temporalEvaluate(keys, time)` for a finite `time`.
    pub fn value(&self, time: f64) -> JsValue {
        match self.segment(time) {
            Err(v) => JsValue::Number(v),
            Ok(lo) => self.curve(lo).value(time),
        }
    }

    /// [`TemporalTrack::value`] for a numeric track (see
    /// [`TemporalTrack::is_numeric`]); NaN otherwise.
    #[inline]
    pub fn value_number(&self, time: f64) -> f64 {
        match self.segment(time) {
            Err(v) => v,
            Ok(lo) => {
                let curve = self.curve(lo);
                match curve.base {
                    Base::Number(b) => curve.value_number(time, b),
                    Base::Raw(_) => f64::NAN,
                }
            }
        }
    }
}

/// A `kf` value prepared for repeated evaluation.
#[derive(Clone, Debug)]
pub enum PreparedTrack {
    /// Not an array, or empty: every answer is `null`.
    Null,
    /// The temporal path.
    Temporal(Arc<TemporalTrack>),
    /// The temporal path, but adoption throws.
    Throws(JsThrow),
    /// The discrete path; evaluated from the keys each time.
    Discrete,
    /// `kf[0]` (or the last key) is `null`: reading its value throws.
    NullFirst,
}

impl PreparedTrack {
    /// Prepares a track (the work `temporalEvaluate` caches per version).
    pub fn prepare(kf: Option<&Node>) -> PreparedTrack {
        let keys = match kf.map(Node::kind) {
            Some(NodeKind::Array(keys)) if !keys.is_empty() => keys,
            _ => return PreparedTrack::Null,
        };
        let first = &keys[0];
        let last = &keys[keys.len() - 1];
        if node::is_nullish(Some(first)) {
            return PreparedTrack::NullFirst;
        }
        if node::number(node::get(first, b"v").node()).is_none() {
            return PreparedTrack::Discrete;
        }
        if node::is_nullish(Some(last)) {
            return PreparedTrack::NullFirst;
        }
        if node::number(node::get(last, b"v").node()).is_none() {
            return PreparedTrack::Discrete;
        }
        match adoption_error(keys) {
            None => PreparedTrack::Temporal(Arc::new(TemporalTrack::new(keys.clone()))),
            Some(e) => PreparedTrack::Throws(e),
        }
    }

    /// The temporal track, when this is one that does not throw.
    pub fn temporal(&self) -> Option<&TemporalTrack> {
        match self {
            PreparedTrack::Temporal(t) => Some(t),
            _ => None,
        }
    }

    /// `evalKfs(kf, t, interpolateColor)`; `kf` must be the node this was
    /// prepared from.
    pub fn value(
        &self,
        kf: Option<&Node>,
        t: f64,
        interpolate_color: bool,
        springs: &mut SpringTables,
    ) -> JsResult<JsValue> {
        if matches!(self, PreparedTrack::Null) {
            return Ok(JsValue::Null);
        }
        let keys = node::array(kf).unwrap_or(&[]);
        if !t.is_finite() {
            // kf[0]?.v ?? null
            let first = keys.first();
            if node::is_nullish(first) {
                return Ok(JsValue::Null);
            }
            return Ok(node::get(first.expect("present"), b"v")
                .to_js()
                .or_nullish(JsValue::Null));
        }
        match self {
            PreparedTrack::Null => Ok(JsValue::Null),
            PreparedTrack::NullFirst => Err(JsThrow::type_error(
                "Cannot read properties of null (reading 'v')",
            )),
            PreparedTrack::Throws(e) => Err(e.clone()),
            PreparedTrack::Temporal(track) => Ok(track.value(t)),
            PreparedTrack::Discrete => discrete(keys, t, interpolate_color, springs),
        }
    }
}

/// `evalKfs(kf, t, interpolateColor)` without caching.
pub fn evaluate(
    kf: Option<&Node>,
    t: f64,
    interpolate_color: bool,
    springs: &mut SpringTables,
) -> JsResult<JsValue> {
    PreparedTrack::prepare(kf).value(kf, t, interpolate_color, springs)
}

/// `/^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/i`.
pub fn is_hex_color(v: &JsValue) -> bool {
    let JsValue::String(s) = v else {
        return false;
    };
    let b = s.as_bytes();
    (b.len() == 7 || b.len() == 9) && b[0] == b'#' && b[1..].iter().all(u8::is_ascii_hexdigit)
}

fn hex_pair(s: &[u8], index: usize) -> f64 {
    let digit = |c: u8| (c as char).to_digit(16).expect("checked hex") as f64;
    digit(s[index]) * 16.0 + digit(s[index + 1])
}

/// `clamp(v, a, b)` from `legacy/core/util.ts`.
fn clamp(v: f64, a: f64, b: f64) -> f64 {
    if v < a {
        a
    } else if v > b {
        b
    } else {
        v
    }
}

/// The per-byte color mix of `evalKfs`.
pub fn mix_color(a: &[u8], b: &[u8], amount: f64) -> JsValue {
    let channel = |v: &[u8], index: usize| {
        if index == 7 && v.len() == 7 {
            255.0
        } else {
            hex_pair(v, index)
        }
    };
    let mut bytes: Vec<f64> = [1usize, 3, 5, 7]
        .iter()
        .map(|&index| {
            let start = channel(a, index);
            let end = channel(b, index);
            js_sem::math_round(clamp(start + (end - start) * amount, 0.0, 255.0))
        })
        .collect();
    if bytes[3] == 255.0 {
        bytes.pop();
    }
    let mut out = String::from("#");
    for byte in bytes {
        out.push_str(&js_sem::pad_start(
            &js_sem::number_to_string_radix(byte, 16),
            2,
            '0',
        ));
    }
    JsValue::from(out.as_str())
}

fn key_member<'a>(key: &'a Node, name: &[u8]) -> JsResult<Got<'a>> {
    node::get_checked(Some(key), name)
}

/// `Array.isArray(h) && Number.isFinite(h[0]) && Number.isFinite(h[1])`.
fn finite_pair(h: Option<&Node>) -> Option<[f64; 2]> {
    let items = node::array(h)?;
    Some([
        node::finite_number(items.first())?,
        node::finite_number(items.get(1))?,
    ])
}

/// The discrete branch of `evalKfs` for a finite `t`.
fn discrete(
    keys: &[Node],
    t: f64,
    interpolate_color: bool,
    springs: &mut SpringTables,
) -> JsResult<JsValue> {
    let n = keys.len();
    let time = JsValue::Number(t);
    let first = &keys[0];
    if js_sem::less_equal(&time, &key_member(first, b"t")?.to_js()) {
        return Ok(key_member(first, b"v")?.to_js());
    }
    let last = &keys[n - 1];
    if js_sem::greater_equal(&time, &key_member(last, b"t")?.to_js()) {
        return Ok(key_member(last, b"v")?.to_js());
    }
    let mut lo = 0usize;
    let mut hi = n - 1;
    while hi - lo > 1 {
        let m = (lo + hi) >> 1;
        if js_sem::less_equal(&key_member(&keys[m], b"t")?.to_js(), &time) {
            lo = m;
        } else {
            hi = m;
        }
    }
    let a = &keys[lo];
    let b = &keys[hi];
    let a_v = key_member(a, b"v")?.to_js();
    if key_member(a, b"hold")?.truthy() {
        return Ok(a_v);
    }
    let colors =
        interpolate_color && is_hex_color(&a_v) && is_hex_color(&key_member(b, b"v")?.to_js());
    if !colors {
        if !a_v.is_finite_number() {
            return Ok(a_v);
        }
        if !key_member(b, b"v")?.to_js().is_finite_number() {
            return Ok(a_v);
        }
    }
    let b_v = key_member(b, b"v")?.to_js();
    let mix = |amount: f64| -> JsValue {
        if colors {
            let (JsValue::String(sa), JsValue::String(sb)) = (&a_v, &b_v) else {
                unreachable!("colors are strings")
            };
            mix_color(sa.as_bytes(), sb.as_bytes(), amount)
        } else {
            let (va, vb) = (a_v.to_number(), b_v.to_number());
            JsValue::Number(va + (vb - va) * amount)
        }
    };
    let a_t = key_member(a, b"t")?.to_js();
    let span = js_sem::sub(&key_member(b, b"t")?.to_js(), &a_t);
    let u = if span <= 0.0 {
        0.0
    } else {
        (t - a_t.to_number()) / span
    };
    if let Some(cfg) = spring_of(a) {
        let amount = springs.spring(u * span, cfg);
        if amount.is_finite() {
            return Ok(mix(amount));
        }
    }
    if !colors {
        // temporalValue(kf, lo, t) on the stored, unadopted keys.
        let curve = Curve::new(&raw_seg_key(a), &raw_seg_key(b), |cfg| springs.table(cfg));
        return Ok(curve.value(t));
    }
    let (default_eo, default_ei) = handles("power");
    let eo = finite_pair(node::member(Some(a), "eo")).unwrap_or(default_eo);
    let ei = finite_pair(node::member(Some(b), "ei")).unwrap_or(default_ei);
    let amount = bezier(eo[0], eo[1], ei[0], ei[1]).eval(u);
    if amount.is_finite() {
        return Ok(mix(amount));
    }
    Ok(mix(u))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::parse;

    fn kf(text: &str) -> Node {
        Node::from_value(parse(text).unwrap(), 0)
    }

    fn ev(text: &str, t: f64, color: bool) -> JsResult<JsValue> {
        let node = kf(text);
        evaluate(Some(&node), t, color, &mut SpringTables::new())
    }

    #[test]
    fn string_middle_key_concatenates() {
        let track = r#"[{"t":0,"v":0},{"t":1,"v":"5"},{"t":2,"v":10}]"#;
        assert_eq!(ev(track, 0.5, false).unwrap(), JsValue::Number(2.5));
        assert_eq!(ev(track, 1.5, false).unwrap(), JsValue::from("52.5"));
    }

    #[test]
    fn colors_interpolate_per_byte() {
        let track = r##"[{"t":0,"v":"#000000","eo":[0,0],"ei":[1,1]},{"t":1,"v":"#ffffff80","eo":[0,0],"ei":[1,1]}]"##;
        assert_eq!(ev(track, 0.5, true).unwrap(), JsValue::from("#808080c0"));
        assert_eq!(ev(track, 0.5, false).unwrap(), JsValue::from("#000000"));
    }

    /// A track makes each segment's curve on its first use; the answers do
    /// not depend on which segments were asked for first, and are the ones
    /// a track prepared for every single answer gives.
    #[test]
    fn segments_made_on_first_use_answer_in_any_order() {
        let track = kf(r#"[
            {"t":0,"v":0,"inInterp":"bezier","outInterp":"bezier","inEase":{"speed":0,"influence":33},"outEase":{"speed":0,"influence":33},"autoBezier":true},
            {"t":1,"v":10,"inInterp":"bezier","outInterp":"bezier","inEase":{"speed":0,"influence":33},"outEase":{"speed":0,"influence":33},"autoBezier":true},
            {"t":2,"v":30,"eo":[0.4,0],"ei":[0.6,1]},
            {"t":"3","v":25,"hold":true},
            {"t":4,"v":5,"spring":{"stiffness":120,"damping":9,"mass":1}},
            {"t":5,"v":-5,"inInterp":"bezier","outInterp":"bezier","inEase":{"speed":0,"influence":33},"outEase":{"speed":0,"influence":33},"autoBezier":true},
            {"t":6,"v":40}
        ]"#);
        let times: Vec<f64> = (0..=64).map(|i| -0.5 + i as f64 * 0.109).collect();
        let fresh: Vec<JsValue> = times
            .iter()
            .map(|&t| evaluate(Some(&track), t, false, &mut SpringTables::new()).unwrap())
            .collect();
        for order in [false, true] {
            let prepared = PreparedTrack::prepare(Some(&track));
            let temporal = prepared.temporal().expect("a temporal track");
            let indices: Vec<usize> = if order {
                (0..times.len()).rev().collect()
            } else {
                (0..times.len()).collect()
            };
            for i in indices {
                assert_eq!(
                    temporal.value(times[i]),
                    fresh[i],
                    "at {} (reversed: {order})",
                    times[i]
                );
            }
        }
        // The answers of the adoption that prepared every key at once
        // (auto-bezier, legacy handles, hold, spring, a string time), bit
        // for bit, at every fourth time.
        const WHOLE: [u64; 17] = [
            0,
            0,
            4613398973615375893,
            4620147942657401728,
            4625276642904716806,
            4628267178381869934,
            4629110819272958837,
            4628559021649363989,
            4627747222013583037,
            4627730092099895296,
            4627730092099895296,
            13843364069718884352,
            13840338453834760192,
            13826450269815070064,
            4626420888944297992,
            4630826316843712512,
            4630826316843712512,
        ];
        for (n, bits) in WHOLE.iter().enumerate() {
            assert_eq!(
                fresh[n * 4],
                JsValue::Number(f64::from_bits(*bits)),
                "at {}",
                times[n * 4]
            );
        }
    }

    #[test]
    fn empty_and_non_finite() {
        assert_eq!(ev("[]", 0.0, false).unwrap(), JsValue::Null);
        assert_eq!(ev("{}", 0.0, false).unwrap(), JsValue::Null);
        assert_eq!(
            ev(r#"[{"t":0,"v":3}]"#, f64::NAN, false).unwrap(),
            JsValue::Number(3.0)
        );
        assert_eq!(ev(r#"[null]"#, f64::NAN, false).unwrap(), JsValue::Null);
        assert!(ev(r#"[null]"#, 0.0, false).is_err());
        assert!(ev(r#"[{"t":0,"v":1},5,{"t":2,"v":3}]"#, 1.0, false).is_err());
    }
}
