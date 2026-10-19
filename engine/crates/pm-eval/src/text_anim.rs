//! Text animators and range styles (`legacy/core/text-animation.ts`).
//!
//! Each enabled animator selects characters, words, or lines and pushes them
//! away from their resting pose by its property values, scaled by a per-unit
//! weight in 0…1 (Back and Spring easing overshoot it; waves swing −1…1):
//!
//! - **stagger**: time-based; every unit eases between the animated values
//!   and rest, one after another, in the chosen order;
//! - **range**: After Effects style; start, end, and offset choose the
//!   affected units (square, ramp, triangle, or round shapes, softness);
//! - **wave**: a continuous ripple.
//!
//! Animators without a `mode` predate the modes: they are range selectors
//! whose unit lives in the `unit` channel.
//!
//! Units come from the level 1 text layout (`pm_text::compat::glyph_layout`,
//! the port of `PM.textLayout`); grapheme segmentation is
//! `Intl.Segmenter`-identical ([`pm_text::compat::graphemes`]).

use pm_model::{JsString, Node};
use pm_text::compat::{GlyphLayout, Unit};

use crate::evaluator::Content;
use crate::js_sem::{self, JsResult, JsValue};
use crate::math;
use crate::node;
use crate::scope::{Scope, js_max, js_min};
use crate::transform::Mat2d;

/// `TEXT_ANIMATOR_PROPERTIES`: offsets are added at full weight; scale and
/// opacity are target percentages.
pub const PROPERTIES: [&str; 9] = [
    "opacity", "x", "y", "scale", "rotation", "skew", "tracking", "blur", "color",
];

/// An animator's mode (`animatorMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Stagger,
    Range,
    Wave,
}

impl Mode {
    /// `animatorMode(animator)`: `stagger` or `wave` when set, otherwise `range`.
    pub fn of(animator: Option<&Node>) -> Mode {
        let mode = node::member(animator, "mode");
        if node::is_str(mode, "stagger") {
            Mode::Stagger
        } else if node::is_str(mode, "wave") {
            Mode::Wave
        } else {
            Mode::Range
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Stagger => "stagger",
            Mode::Range => "range",
            Mode::Wave => "wave",
        }
    }
}

/// What an animator selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Characters,
    Words,
    Lines,
}

impl UnitKind {
    pub fn name(self) -> &'static str {
        match self {
            UnitKind::Characters => "characters",
            UnitKind::Words => "words",
            UnitKind::Lines => "lines",
        }
    }

    fn of_js(v: &JsValue) -> Option<UnitKind> {
        match v {
            JsValue::String(s) => match s.as_str()? {
                "characters" => Some(UnitKind::Characters),
                "words" => Some(UnitKind::Words),
                "lines" => Some(UnitKind::Lines),
                _ => None,
            },
            _ => None,
        }
    }

    /// `layout[unit]`.
    pub fn units(self, layout: &GlyphLayout) -> &[Unit] {
        match self {
            UnitKind::Characters => &layout.characters,
            UnitKind::Words => &layout.words,
            UnitKind::Lines => &layout.lines,
        }
    }
}

/// The selection order of a stagger or range animator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Forward,
    Reverse,
    Center,
    Edges,
    Random,
}

impl Order {
    fn of(animator: Option<&Node>) -> Order {
        let o = node::member(animator, "order");
        if node::is_str(o, "reverse") {
            Order::Reverse
        } else if node::is_str(o, "center") {
            Order::Center
        } else if node::is_str(o, "edges") {
            Order::Edges
        } else if node::is_str(o, "random") {
            Order::Random
        } else {
            Order::Forward
        }
    }
}

/// The shape of a range selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Square,
    RampUp,
    RampDown,
    Triangle,
    Round,
}

impl Shape {
    fn of(animator: Option<&Node>) -> Shape {
        let s = node::member(animator, "shape");
        if node::is_str(s, "rampUp") {
            Shape::RampUp
        } else if node::is_str(s, "rampDown") {
            Shape::RampDown
        } else if node::is_str(s, "triangle") {
            Shape::Triangle
        } else if node::is_str(s, "round") {
            Shape::Round
        } else {
            Shape::Square
        }
    }
}

/// `num(value, fallback)`: `Number(value)` when finite, else `fallback`.
pub fn num(value: &JsValue, fallback: f64) -> f64 {
    let n = value.to_number();
    if n.is_finite() { n } else { fallback }
}

fn num_node(value: Option<&Node>, fallback: f64) -> f64 {
    let n = node::to_number(value);
    if n.is_finite() { n } else { fallback }
}

/// `Math.max(0, Math.min(1, value))`.
pub fn clamp01(v: f64) -> f64 {
    js_max(0.0, js_min(1.0, v))
}

fn get<'c>(values: &'c Content, key: &str) -> &'c JsValue {
    static UNDEFINED: JsValue = JsValue::Undefined;
    values.get(&JsString::from(key)).unwrap_or(&UNDEFINED)
}

/// A deterministic shuffle so `random` order is stable across frames and
/// exports (`randomRanks`): the rank of each unit.
pub fn random_ranks(count: usize, seed: f64) -> Vec<usize> {
    // `(Math.floor(seed) >>> 0) || 1`
    let mut s: u32 = {
        let f = seed.floor();
        let u = to_uint32(f);
        if u == 0 { 1 } else { u }
    };
    let mut next = || {
        s = s.wrapping_add(0x6D2B_79F5);
        let mut t = s;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    };
    let mut order: Vec<usize> = (0..count).collect();
    let mut i = count;
    while i > 1 {
        i -= 1;
        let j = (next() * (i as f64 + 1.0)).floor() as usize;
        order.swap(i, j);
    }
    let mut ranks = vec![0; count];
    for (rank, unit) in order.into_iter().enumerate() {
        ranks[unit] = rank;
    }
    ranks
}

/// ECMAScript `ToUint32` of a number.
fn to_uint32(x: f64) -> u32 {
    if !x.is_finite() {
        return 0;
    }
    let t = x.trunc();
    let m = t.rem_euclid(4_294_967_296.0);
    m as u32
}

/// `unitRank(order, index, count, seed)`: a unit's position in the
/// animator's sequence. Center and edges rank by distance from the middle,
/// so symmetric units share a rank.
pub fn unit_rank(order: Order, index: usize, count: usize, seed: f64) -> f64 {
    let mid = (count as f64 - 1.0) / 2.0;
    let i = index as f64;
    match order {
        Order::Reverse => count as f64 - 1.0 - i,
        Order::Center => (i - mid).abs(),
        Order::Edges => mid - (i - mid).abs(),
        Order::Random => random_ranks(count, seed)
            .get(index)
            .map_or(i, |&r| r as f64),
        Order::Forward => i,
    }
}

fn rank_span(order: Order, count: usize) -> f64 {
    let c = count as f64;
    if matches!(order, Order::Center | Order::Edges) {
        js_max(0.0, (c - 1.0) / 2.0)
    } else {
        js_max(0.0, c - 1.0)
    }
}

/// `animatorWeight(p, rank, count, shape)`: the range selector's weight for
/// a unit at `rank` (0 is the first unit in the chosen order).
pub fn animator_weight(p: &Content, rank: f64, count: usize, shape: Shape) -> f64 {
    let position = (rank + 0.5) / js_max(1.0, count as f64) * 100.0 - num(get(p, "offset"), 0.0);
    let s = num(get(p, "start"), 0.0);
    let e = num(get(p, "end"), 100.0);
    let start = js_min(s, e);
    let end = js_max(s, e);
    let span = end - start;
    if matches!(shape, Shape::RampUp | Shape::RampDown) {
        let u = if span > 0.0 {
            clamp01((position - start) / span)
        } else if position >= start {
            1.0
        } else {
            0.0
        };
        return if shape == Shape::RampUp { u } else { 1.0 - u };
    }
    if position < start || position > end {
        return 0.0;
    }
    if matches!(shape, Shape::Triangle | Shape::Round) {
        let u = if span > 0.0 {
            (position - start) / span
        } else {
            0.5
        };
        return if shape == Shape::Triangle {
            1.0 - (u * 2.0 - 1.0).abs()
        } else {
            math::sin(core::f64::consts::PI * u)
        };
    }
    let feather = js_max(0.0, num(get(p, "smoothness"), 0.0)) / 100.0 * span / 2.0;
    if feather == 0.0 || feather.is_nan() {
        return 1.0;
    }
    let weight = clamp01(js_min(
        (position - start) / feather,
        (end - position) / feather,
    ));
    weight * weight * (3.0 - 2.0 * weight)
}

/// The spring stagger easing uses (`SPRING`).
const SPRING: crate::easing::SpringConfig = crate::easing::SpringConfig {
    damping: 11.0,
    stiffness: 170.0,
    mass: 1.0,
};

/// `easeProgress(PM, easing, progress)`: eased progress for a stagger unit.
pub fn ease_progress(
    springs: &mut crate::easing::SpringTables,
    easing: Option<&Node>,
    progress: f64,
) -> f64 {
    let t = clamp01(progress);
    if t <= 0.0 || t >= 1.0 {
        return t;
    }
    if node::is_str(easing, "spring") {
        let settle = crate::easing::spring_duration(SPRING);
        return springs.spring(t * settle, SPRING);
    }
    if let Some(name) = node::string(easing).and_then(|s| s.as_str())
        && crate::easing::PRESETS.iter().any(|(n, _)| *n == name)
    {
        return crate::easing::preset_curve(name).eval(t);
    }
    if node::is_str(easing, "linear") {
        return t;
    }
    1.0 - math::pow(1.0 - t, 3.0)
}

/// `staggerLength(values, count, order)`: seconds from the animator's start
/// until every unit has settled.
pub fn stagger_length(values: &Content, count: usize, order: Order) -> f64 {
    js_max(0.0, num(get(values, "duration"), 0.5))
        + js_max(0.0, num(get(values, "stagger"), 0.0)) * rank_span(order, count)
}

/// `staggerWindow(values, count, order)`: start, duration, cascade, end.
pub fn stagger_window(values: &Content, count: usize, order: Order) -> [f64; 4] {
    let start = num(get(values, "delay"), 0.0);
    let duration = js_max(0.0, num(get(values, "duration"), 0.5));
    let cascade = js_max(0.0, num(get(values, "stagger"), 0.0)) * rank_span(order, count);
    [start, duration, cascade, start + duration + cascade]
}

/// JavaScript's `\s` (WhiteSpace and LineTerminator).
fn is_space_cp(c: u32) -> bool {
    c <= 0xFFFF && pm_text::blink::chars::is_js_space(c as u16)
}

/// `countTextUnits(lines, unit)`: unit counts for wrapped lines, segmented
/// exactly as the text layout does.
pub fn count_text_units(lines: &[JsString], unit: UnitKind) -> usize {
    match unit {
        UnitKind::Lines => lines.iter().filter(|l| l.utf16_len() > 0).count(),
        UnitKind::Words => lines
            .iter()
            .map(|l| {
                let mut count = 0;
                let mut in_word = false;
                for c in l.code_points() {
                    let space = is_space_cp(c);
                    if !space && !in_word {
                        count += 1;
                    }
                    in_word = !space;
                }
                count
            })
            .sum(),
        UnitKind::Characters => lines
            .iter()
            .map(|l| {
                let units: Vec<u16> = l.utf16_units().collect();
                pm_text::compat::graphemes(&units)
                    .into_iter()
                    .filter(|r| {
                        // `/^\s+$/u`: every code point is a space.
                        let seg = JsString::from_utf16_units(&units[r.clone()]);
                        !(seg.utf16_len() > 0 && seg.code_points().all(is_space_cp))
                    })
                    .count()
            })
            .sum(),
    }
}

/// Animator or style items with their evaluated channels.
type Controls<'d> = Vec<(&'d Node, Content)>;

/// One evaluated animator (`TextAnimatorState`).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatorState {
    pub id: JsValue,
    pub mode: Mode,
    pub unit: UnitKind,
    /// Evaluated channel values.
    pub values: Content,
    /// Selection weight per unit, in layout order.
    pub weights: Vec<f64>,
}

/// One range style: the stored item with its channels evaluated.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleState {
    /// `{ ...item, p }`: the item's members with `p` replaced.
    pub item: Content,
    pub p: Content,
}

/// Evaluated text controls before layout: enabled animators with their
/// values, and the resolved range styles.
pub struct TextControls<'d> {
    animators: Controls<'d>,
    pub styles: Vec<StyleState>,
}

/// `textAnimationState`'s answer.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TextAnimation {
    pub animators: Vec<AnimatorState>,
    pub styles: Vec<StyleState>,
}

/// A glyph with its animated pose (`animatedGlyphs`).
#[derive(Clone, Debug, PartialEq)]
pub struct PosedGlyph {
    /// The layout unit (`x` already moved by tracking).
    pub unit: Unit,
    /// Resolved typography: the content plus every range style covering it.
    pub style: Content,
    /// Relative to the glyph's baseline origin (`x`, `y + baseline`); scale,
    /// rotation, and skew pivot around the center of the glyph's unit.
    pub matrix: Mat2d,
    pub opacity: f64,
    pub blur: f64,
    pub color: Option<JsString>,
    pub tracking: f64,
    pub tracking_shift: f64,
}

impl PosedGlyph {
    /// `isIdentityGlyph(glyph)`.
    pub fn is_identity(&self) -> bool {
        let m = &self.matrix;
        m[0] == 1.0
            && m[1] == 0.0
            && m[2] == 0.0
            && m[3] == 1.0
            && m[4] == 0.0
            && m[5] == 0.0
            && self.opacity == 1.0
            && (self.blur == 0.0 || self.blur.is_nan())
            && self.color.is_none()
            && (self.tracking_shift == 0.0 || self.tracking_shift.is_nan())
    }
}

/// `parseColor(value)` on `String(value || '').trim()`.
fn parse_color(text: &str) -> Option<[f64; 3]> {
    let text = pm_model::json::js_trim(text);
    if let Some(hex) = text.strip_prefix('#')
        && (3..=8).contains(&hex.len())
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        let hex: String = if hex.len() <= 4 {
            hex[..3].chars().flat_map(|c| [c, c]).collect()
        } else {
            hex.to_owned()
        };
        let part = |a: usize, b: usize| {
            let s = hex.get(a..b.min(hex.len())).unwrap_or("");
            js_sem::parse_int(&JsString::from(s), 16)
        };
        return Some([part(0, 2), part(2, 4), part(4, 6)]);
    }
    // /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i
    let lower = text.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))?;
    let rest = &text[text.len() - rest.len()..];
    let mut chars = rest.char_indices().peekable();
    let skip = |chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, sep: bool| {
        let mut n = 0;
        while let Some(&(_, c)) = chars.peek() {
            let ok = if sep {
                is_space_cp(c as u32) || c == ','
            } else {
                is_space_cp(c as u32)
            };
            if !ok {
                break;
            }
            chars.next();
            n += 1;
        }
        n
    };
    let take = |chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>| {
        let mut s = String::new();
        while let Some(&(_, c)) = chars.peek() {
            if c.is_ascii_digit() || c == '.' {
                s.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if s.is_empty() { None } else { Some(s) }
    };
    skip(&mut chars, false);
    let a = take(&mut chars)?;
    if skip(&mut chars, true) == 0 {
        return None;
    }
    let b = take(&mut chars)?;
    if skip(&mut chars, true) == 0 {
        return None;
    }
    let c = take(&mut chars)?;
    let n = |s: &str| num(&JsValue::from(s), 0.0);
    Some([n(&a), n(&b), n(&c)])
}

/// `mixColor(from, to, weight)`.
pub fn mix_color(from: &JsString, to: &JsString, weight: f64) -> JsString {
    let w = clamp01(weight);
    // `w <= 0` is false for NaN, which reaches the channel math below.
    if w <= 0.0 {
        return from.clone();
    }
    let (Some(a), Some(b)) = (
        parse_color(&from.to_string_lossy()),
        parse_color(&to.to_string_lossy()),
    ) else {
        return if w >= 0.5 { to.clone() } else { from.clone() };
    };
    let channel = |i: usize| {
        let v = js_sem::math_round(a[i] + (b[i] - a[i]) * w);
        js_sem::pad_start(&js_sem::number_to_string_radix(v, 16), 2, '0')
    };
    JsString::from(format!("#{}{}{}", channel(0), channel(1), channel(2)).as_str())
}

/// `multiply(m, n)` of `text-animation.ts` (the same product as `PM.mul`).
fn multiply(m: &Mat2d, n: &Mat2d) -> Mat2d {
    crate::transform::mul(m, n)
}

impl<'d> Scope<'_, 'd> {
    /// `resolvedTextContent(layer, time)` (`legacy/gl/raster.ts`): the
    /// resolved content with `boxWidth`, `boxHeight`, and `paragraph` as
    /// numbers and every `fontAxis.*` channel evaluated.
    pub fn resolved_text_content(&mut self, layer: &'d Node, time: f64) -> JsResult<Content> {
        let mut d = self.ev.resolve_content(self.comp, layer, time)?;
        let stored = node::member(Some(layer), "d");
        let value = |scope: &mut Self, d: &Content, key: &str| -> JsResult<f64> {
            let source = d.get(&JsString::from(key)).cloned().unwrap_or_default();
            match &source {
                JsValue::Object(v) if !matches!(**v, pm_model::Value::Array(_)) => {
                    let n = scope
                        .channel_p(layer, node::member(stored, key), time, &format!("c.{key}"))?
                        .to_number();
                    Ok(crate::scope::or0(n))
                }
                other => Ok(crate::scope::or0(other.to_number())),
            }
        };
        let box_width = js_max(0.0, value(self, &d, "boxWidth")?);
        let box_height = js_max(0.0, value(self, &d, "boxHeight")?);
        let keys: Vec<JsString> = d.keys().cloned().collect();
        for key in keys {
            if !key.as_bytes().starts_with(b"fontAxis.") {
                continue;
            }
            let JsValue::Object(v) = &d[&key] else {
                continue;
            };
            let has_kf = matches!(&**v, pm_model::Value::Object(m) if matches!(m.get_key(&JsString::from("kf")), Some(pm_model::Value::Array(_))));
            if !has_kf {
                continue;
            }
            let k = key.to_string_lossy();
            let n = self
                .channel_p(layer, node::member(stored, &k), time, &format!("c.{k}"))?
                .to_number();
            d.insert(key, JsValue::Number(n));
        }
        d.insert(JsString::from("boxWidth"), JsValue::Number(box_width));
        d.insert(JsString::from("boxHeight"), JsValue::Number(box_height));
        d.insert(JsString::from("paragraph"), JsValue::Bool(box_width > 0.0));
        Ok(d)
    }

    /// `textControlValues(PM, layer, time)`: enabled animators and every
    /// range style, each with its channels evaluated.
    fn text_control_values(
        &mut self,
        layer: &'d Node,
        time: f64,
    ) -> JsResult<(Controls<'d>, Controls<'d>)> {
        let d = node::get_checked(Some(layer), b"d")?;
        let mut out: [Vec<(&'d Node, Content)>; 2] = [Vec::new(), Vec::new()];
        for (k, kind) in ["animators", "styles"].iter().enumerate() {
            let list = d.node().and_then(|d| node::member(Some(d), kind));
            let items = if node::truthy(list) {
                node::array(list).unwrap_or(&[])
            } else {
                &[]
            };
            let prefix = if k == 0 { "ta" } else { "ts" };
            for item in items {
                if k == 0
                    && matches!(
                        node::member(Some(item), "enabled").map(Node::kind),
                        Some(pm_model::NodeKind::Bool(false))
                    )
                {
                    continue;
                }
                let id = node::to_js_string(node::member(Some(item), "id")).to_string_lossy();
                let mut p = Content::new();
                let pn = node::member(Some(item), "p");
                if node::truthy(pn)
                    && let Some(pn) = pn
                {
                    for (key, prop) in node::entries(pn) {
                        let path = format!("{prefix}.{id}.{}", key.to_string_lossy());
                        let v = self.channel_p(layer, prop.node(), time, &path)?;
                        p.insert(key, v);
                    }
                }
                out[k].push((item, p));
            }
        }
        let [a, s] = out;
        Ok((a, s))
    }

    /// `unitWeights(PM, layer, animator, values, count, time)`.
    pub fn unit_weights(
        &mut self,
        layer: &'d Node,
        animator: &'d Node,
        values: &Content,
        count: usize,
        time: f64,
    ) -> Vec<f64> {
        let a = Some(animator);
        let mode = Mode::of(a);
        let offset = js_max(0.0, num_node(node::member(a, "unitOffset"), 0.0).floor()) as usize;
        let total_raw = num_node(node::member(a, "unitTotal"), 0.0).floor();
        let total_raw = if total_raw == 0.0 || total_raw.is_nan() {
            0.0
        } else {
            total_raw
        };
        let total = js_max(js_max((count + offset) as f64, total_raw), 1.0) as usize;
        let amount = num(get(values, "amount"), 100.0) / 100.0;
        let order = Order::of(a);
        let seed = num_node(node::member(a, "seed"), 1.0);
        let local = time - num_node(node::member(Some(layer), "from"), 0.0);
        let mut weights = Vec::with_capacity(count);
        for i in 0..count {
            let rank = unit_rank(order, offset + i, total, seed);
            let w = match mode {
                Mode::Stagger => {
                    let duration = js_max(0.0, num(get(values, "duration"), 0.5));
                    let elapsed = local
                        - num(get(values, "delay"), 0.0)
                        - rank * js_max(0.0, num(get(values, "stagger"), 0.0));
                    let progress = if duration > 0.0 {
                        elapsed / duration
                    } else if elapsed >= 0.0 {
                        1.0
                    } else {
                        0.0
                    };
                    let eased =
                        ease_progress(self.ev.springs(), node::member(a, "easing"), progress);
                    if node::is_str(node::member(a, "direction"), "out") {
                        eased
                    } else {
                        1.0 - eased
                    }
                }
                Mode::Wave => math::sin(
                    2.0 * core::f64::consts::PI
                        * (local * num(get(values, "speed"), 1.0)
                            - rank * num(get(values, "spread"), 10.0) / 100.0),
                ),
                Mode::Range => {
                    let span = rank_span(order, total);
                    let normalized = if span > 0.0 {
                        rank / span * (total as f64 - 1.0)
                    } else {
                        rank
                    };
                    animator_weight(values, normalized, total, Shape::of(a))
                }
            };
            weights.push(w * amount);
        }
        weights
    }

    /// `textAnimationState(PM, layer, time, layout)`: every enabled
    /// animator's values and per-unit weights, and the range styles.
    pub fn text_animation_state(
        &mut self,
        layer: &'d Node,
        time: f64,
        layout: &GlyphLayout,
    ) -> JsResult<TextAnimation> {
        let controls = self.text_controls(layer, time)?;
        Ok(self.text_animation_weights(layer, time, controls, layout))
    }

    /// The channel half of [`Self::text_animation_state`]: every control
    /// evaluated (in the same order) and the range styles resolved, before
    /// any layout exists. Level 2 shapes with the styles first.
    pub fn text_controls(&mut self, layer: &'d Node, time: f64) -> JsResult<TextControls<'d>> {
        let (animators, styles) = self.text_control_values(layer, time)?;
        let mut out = Vec::with_capacity(styles.len());
        for (item, values) in styles {
            let mut content = Content::new();
            if let Some(n) =
                Some(item).filter(|n| matches!(n.kind(), pm_model::NodeKind::Object(_)))
            {
                for (k, v) in node::entries(n) {
                    content.insert(k, v.to_js());
                }
            }
            let p_value = pm_model::Value::Object(
                values
                    .iter()
                    .filter_map(|(k, v)| v.to_json().map(|j| (k.clone(), j)))
                    .collect(),
            );
            content.insert(JsString::from("p"), JsValue::Object(Box::new(p_value)));
            out.push(StyleState {
                item: content,
                p: values,
            });
        }
        Ok(TextControls {
            animators,
            styles: out,
        })
    }

    /// The layout half of [`Self::text_animation_state`]: each animator's
    /// per-unit weights over `layout`.
    pub fn text_animation_weights(
        &mut self,
        layer: &'d Node,
        time: f64,
        controls: TextControls<'d>,
        layout: &GlyphLayout,
    ) -> TextAnimation {
        let mut out = TextAnimation {
            animators: Vec::with_capacity(controls.animators.len()),
            styles: controls.styles,
        };
        for (item, values) in controls.animators {
            let raw_unit = node::js_value_opt(node::member(Some(item), "unit"));
            let unit = UnitKind::of_js(&raw_unit).unwrap_or_else(|| {
                match UnitKind::of_js(get(&values, "unit")) {
                    Some(u @ (UnitKind::Words | UnitKind::Lines)) => u,
                    _ => UnitKind::Characters,
                }
            });
            let count = unit_count(layout, unit);
            let weights = self.unit_weights(layer, item, &values, count, time);
            out.animators.push(AnimatorState {
                id: node::js_value_opt(node::member(Some(item), "id")),
                mode: Mode::of(Some(item)),
                unit,
                values,
                weights,
            });
        }
        out
    }

    /// `animatedGlyphs(PM, layer, time, d, layout, state)`: glyphs with their
    /// animated pose, tracking applied along each line.
    pub fn animated_glyphs(
        &mut self,
        d: &Content,
        layout: &GlyphLayout,
        state: &TextAnimation,
    ) -> Vec<PosedGlyph> {
        self.posed_glyphs(d, layout, state, false)
    }

    /// [`Self::animated_glyphs`] as `compat` level 2 poses them: a pure
    /// translation is the authored offset itself (no pivot round trip), so
    /// integer selector movement stays exact. What the level 2 renderer
    /// draws and what `textAnimation` reports for level 2 layers.
    pub fn level2_glyphs(
        &mut self,
        d: &Content,
        layout: &GlyphLayout,
        state: &TextAnimation,
    ) -> Vec<PosedGlyph> {
        self.posed_glyphs(d, layout, state, true)
    }

    fn posed_glyphs(
        &mut self,
        d: &Content,
        layout: &GlyphLayout,
        state: &TextAnimation,
        level2: bool,
    ) -> Vec<PosedGlyph> {
        let size = js_max(1.0, num(get(d, "size"), 16.0));
        let pivot_y = -size * 0.35;
        let mut glyphs: Vec<PosedGlyph> = layout
            .characters
            .iter()
            .map(|glyph| {
                let mut style = d.clone();
                let source = JsValue::Number(glyph.source_start as f64);
                for range in &state.styles {
                    let start = range
                        .item
                        .get(&JsString::from("start"))
                        .cloned()
                        .unwrap_or_default();
                    let end = range
                        .item
                        .get(&JsString::from("end"))
                        .cloned()
                        .unwrap_or_default();
                    if js_sem::greater_equal(&source, &start) && js_sem::less_than(&source, &end) {
                        for (k, v) in &range.p {
                            style.insert(k.clone(), v.clone());
                        }
                    }
                }
                let mut matrix: Mat2d = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                let (mut opacity, mut blur, mut tracking) = (1.0f64, 0.0f64, 0.0f64);
                let mut color: Option<JsString> = None;
                for animator in &state.animators {
                    let index = match animator.unit {
                        UnitKind::Lines => Some(glyph.line_unit.unwrap_or(glyph.line)),
                        UnitKind::Words => glyph.word,
                        UnitKind::Characters => Some(glyph.index),
                    };
                    let w = index
                        .and_then(|i| animator.weights.get(i))
                        .copied()
                        .unwrap_or(0.0);
                    if w == 0.0 || w.is_nan() {
                        continue;
                    }
                    let p = &animator.values;
                    let unit = match animator.unit {
                        UnitKind::Characters => Some(glyph),
                        u => index.and_then(|i| u.units(layout).get(i)),
                    };
                    let px = match unit {
                        Some(u) => u.x + u.w / 2.0 - glyph.x,
                        None => glyph.w / 2.0,
                    };
                    let r = math::deg_to_rad(num(get(p, "rotation"), 0.0) * w);
                    let k = math::tan(math::deg_to_rad(js_max(
                        -80.0,
                        js_min(80.0, num(get(p, "skew"), 0.0) * w),
                    )));
                    let s = 1.0 + (num(get(p, "scale"), 100.0) / 100.0 - 1.0) * w;
                    let cos = math::cos(r) * s;
                    let sin = math::sin(r) * s;
                    let mut local: Mat2d = [cos, sin, cos * -k - sin, sin * -k + cos, 0.0, 0.0];
                    local[4] =
                        px + num(get(p, "x"), 0.0) * w - (local[0] * px + local[2] * pivot_y);
                    local[5] =
                        pivot_y + num(get(p, "y"), 0.0) * w - (local[1] * px + local[3] * pivot_y);
                    if level2 && local[..4] == [1.0, 0.0, 0.0, 1.0] {
                        local[4] = num(get(p, "x"), 0.0) * w;
                        local[5] = num(get(p, "y"), 0.0) * w;
                    }
                    matrix = multiply(&local, &matrix);
                    opacity *= 1.0 + (num(get(p, "opacity"), 100.0) / 100.0 - 1.0) * w;
                    blur += js_max(0.0, num(get(p, "blur"), 0.0)) * w.abs();
                    tracking += num(get(p, "tracking"), 0.0) * w;
                    let pc = get(p, "color");
                    if !matches!(pc, JsValue::Undefined | JsValue::Null) && *pc != JsValue::from("")
                    {
                        let base = color
                            .clone()
                            .unwrap_or_else(|| get(&style, "color").to_js_string());
                        color = Some(mix_color(&base, &pc.to_js_string(), w));
                    }
                }
                PosedGlyph {
                    unit: glyph.clone(),
                    style,
                    matrix,
                    opacity: clamp01(opacity),
                    blur,
                    color,
                    tracking,
                    tracking_shift: 0.0,
                }
            })
            .collect();
        // Tracking widens the gap after each unit, pushing the rest of the line.
        let align = match get(d, "align") {
            JsValue::String(s) if s.as_str() == Some("center") => 0.5,
            JsValue::String(s) if s.as_str() == Some("right") => 1.0,
            _ => 0.0,
        };
        let mut lines: Vec<usize> = Vec::new();
        for g in &glyphs {
            if !lines.contains(&g.unit.line) {
                lines.push(g.unit.line);
            }
        }
        for line in lines {
            let mut shift = 0.0;
            let mut last = None;
            for g in glyphs.iter_mut().filter(|g| g.unit.line == line) {
                g.tracking_shift = shift;
                shift += g.tracking;
                last = Some(g.tracking);
            }
            let lead = shift - last.unwrap_or(0.0);
            for g in glyphs.iter_mut().filter(|g| g.unit.line == line) {
                g.tracking_shift -= lead * align;
                g.unit.x += g.tracking_shift;
            }
        }
        glyphs
    }
}

/// `unitCount(layout, unit)`: the number of units, or 1 word or line for
/// text with characters but none of that unit.
pub fn unit_count(layout: &GlyphLayout, unit: UnitKind) -> usize {
    let n = unit.units(layout).len();
    if n > 0 {
        n
    } else if unit == UnitKind::Characters {
        0
    } else if !layout.characters.is_empty() {
        1
    } else {
        0
    }
}
