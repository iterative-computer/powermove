//! Replays `tests/vectors/edit-vectors.json.gz`, written by
//! `tests/vectors/generate-edit-vectors.ts` from today's TypeScript edits
//! under bun (JavaScriptCore): each kernel runs on the same document, time,
//! and arguments, its operations are applied to the document, and the
//! layers it leaves are compared with the layers the TypeScript edit left.
//!
//! Numbers compare within plan section 25's evaluation tolerance
//! (`1e-9 + 1e-9·|b|`; transcendental functions are `libm` here and the
//! platform's in JavaScriptCore), rotations also modulo 360 (amendment 20),
//! numbers inside rewritten expressions the same way. Key times a kernel
//! created or moved are on the tick grid (section 5, rule 4 with amendment
//! 1): they may differ from the TypeScript float sum by the rounding of its
//! terms (`KEY_TIME_TICKS`) and, on NTSC compositions stored with a float
//! rate, by the float grid's drift. Snapping never counts as a mismatch.
//! Values carry no such allowance: a bake samples its own keys at their
//! float times and moves them to the tick grid only when it is done
//! (`groups::ungroup`). Ids an edit creates
//! (`i`, `id` absent from the input) are renamed by order of first
//! appearance on both sides. Messages of user-facing `Error`s must match;
//! engine-internal `TypeError`s only need to throw. The font anchor's cases
//! (`captureFontAnchor`, `anchorReference`/`anchorMoved` with `anchor:
//! "font"`) and measured split text measure on the generator's synthetic
//! canvas (`Synth`). A camera or light reparented into a space that moves
//! with it (the generator marks it `selfSpace`) is compared like every other
//! case: the legacy aim there reads the camera's keyed channels through the
//! evaluation cache the pose write left stale, and the kernel reproduces
//! that read (`pose::reparent`); those cases are also counted on their own.
//! Cases a kernel leaves to the host (three.js bounds of 3D objects and model
//! groups) or that measure text without that canvas (the vectors carry no
//! fonts) are counted apart.
//!
//! `PM_EDIT_VECTORS=<file>` replays another set (a `PM_EDIT_SCALE` run, or
//! the corpus set from `PM_EDIT_CORPUS`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::sync::Arc;

use pm_edit::align::{self, Mode};
use pm_edit::anchor::{self, Kind};
use pm_edit::split_text::{self, Mode as SplitMode};
use pm_edit::{Edit, IdSource, comp_bounds, groups, materials, pose};
use pm_eval::bounds::{HOST_ONLY, NoHost};
use pm_eval::space3d::Rect;
use pm_eval::text_geom::{MeasuredLine, TextMeasure};
use pm_eval::{ExpressionEngine, JsThrow, JsValue};
use pm_model::json::{Value, parse};
use pm_model::{Document, JsString};
use pm_text::compat::{GlyphLayout, Measure, TextContent, glyph_layout, text_source_lines};

/// A decoded value (the generator's encoding, or a document value).
#[derive(Clone, Debug, PartialEq)]
enum V {
    U,
    Z,
    N(f64),
    S(String),
    B(bool),
    A(Vec<V>),
    O(BTreeMap<String, V>),
    Throw(String),
}

fn hexf(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

fn get<'v>(v: &'v Value, k: &str) -> Option<&'v Value> {
    v.as_object().and_then(|m| m.get(k))
}

fn dec(v: &Value) -> V {
    let m = v.as_object().expect("an encoded value");
    let (k, x) = m.iter().next().unwrap();
    match k.as_str().unwrap() {
        "u" => V::U,
        "z" => V::Z,
        "n" => V::N(hexf(x)),
        "s" => V::S(x.as_js_string().unwrap().to_string_lossy()),
        "b" => V::B(matches!(x, Value::Bool(true))),
        "a" => V::A(x.as_array().unwrap().iter().map(dec).collect()),
        "o" => V::O(
            x.as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.to_string_lossy(), dec(v)))
                .collect(),
        ),
        "throw" => V::Throw(x.as_js_string().unwrap().to_string_lossy()),
        other => panic!("unknown encoding {other}"),
    }
}

/// A document value as the generator encodes it.
fn of(v: &Value) -> V {
    match v {
        Value::Null => V::Z,
        Value::Bool(b) => V::B(*b),
        Value::Number(n) => V::N(*n),
        Value::String(s) => V::S(s.to_string_lossy()),
        Value::Array(a) => V::A(a.iter().map(of).collect()),
        Value::Object(m) => V::O(
            m.iter()
                .map(|(k, v)| (k.to_string_lossy(), of(v)))
                .collect(),
        ),
    }
}

fn num(case: &Value, k: &str) -> f64 {
    hexf(get(case, k).unwrap())
}

fn usize_of(v: &Value) -> usize {
    v.as_f64().unwrap() as usize
}

fn indexes(case: &Value, k: &str) -> Vec<usize> {
    get(case, k)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(usize_of)
        .collect()
}

/// Text measurement the vectors cannot answer (they carry no fonts): notes
/// that a kernel asked, so the case is counted apart.
#[derive(Default)]
struct NoText {
    asked: bool,
}

impl TextMeasure for NoText {
    fn measure_lines(&mut self, _: &TextContent) -> Vec<MeasuredLine> {
        self.asked = true;
        Vec::new()
    }
    fn plain_width(&mut self, _: &TextContent, _: &str) -> f64 {
        self.asked = true;
        0.0
    }
    fn glyph_layout(&mut self, _: &TextContent) -> GlyphLayout {
        self.asked = true;
        GlyphLayout::default()
    }
    fn spaced_width(&mut self, _: &TextContent, _: &str) -> f64 {
        self.asked = true;
        0.0
    }
}

/// The generator's synthetic canvas (`synthWidth`, `synthInk`): widths and
/// ink boxes from the font, letter spacing, and text, over the real wrapping
/// and glyph units (`pm_text::compat`, the port of `text-layout.ts`). The
/// font-anchor and measured split-text cases measure with it.
struct Synth;

/// `synthFont`: the size and the per-unit width factor of `fontStr(d)`.
fn synth_font(d: &TextContent) -> (f64, f64) {
    // `fontStr`: `${d.weight || 500} ${d.size}px "${variable || d.font || 'SF Pro Display'}"`.
    let family = d
        .font
        .clone()
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| "SF Pro Display".to_owned());
    let weight = d
        .weight
        .filter(|w| *w != 0.0 && !w.is_nan())
        .unwrap_or(500.0);
    let size = d.size.expect("the vectors' text layers have a size");
    let italic = if d.italic.unwrap_or(false) { 1.05 } else { 1.0 };
    let k = size / 16.0
        * italic
        * (1.0 + (weight - 400.0) / 4000.0)
        * (1.0 + family.encode_utf16().count() as f64 / 100.0);
    (size, k)
}

/// JavaScript's code points of UTF-16 text (`for (const ch of text)`): lone
/// surrogates count as themselves.
fn code_points(text: &[u16]) -> Vec<u32> {
    char::decode_utf16(text.iter().copied())
        .map(|c| c.map_or_else(|e| u32::from(e.unpaired_surrogate()), u32::from))
        .collect()
}

/// `synthWidth`, with the letter spacing `(d.tracking || 0) px` when
/// `spaced`.
fn synth_width(d: &TextContent, spaced: bool, text: &[u16]) -> f64 {
    let (_, k) = synth_font(d);
    let ls = if spaced {
        d.tracking
            .filter(|t| *t != 0.0 && !t.is_nan())
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let mut w = 0.0;
    let mut n = 0.0;
    for cp in code_points(text) {
        w += f64::from((cp % 13) + 4) * k;
        n += 1.0;
    }
    w + ls * n
}

/// `synthInk`: the ink box from the pen origin, in `float`.
fn synth_ink(d: &TextContent, width: f64, text: &[u16]) -> [f64; 4] {
    let cps = code_points(text);
    // `/\S/u`: JavaScript's white space is in the BMP.
    if cps
        .iter()
        .all(|&c| c < 0x10000 && pm_text::blink::chars::is_js_space(c as u16))
    {
        return [0.0; 4];
    }
    let (size, k) = synth_font(d);
    let sum: u64 = cps.iter().map(|&c| u64::from(c)).sum();
    let f = |v: f64| f64::from(v as f32);
    [
        f((f64::from(cps[0] % 5) - 2.0) * k / 4.0),
        f(-(0.72 * size + (sum % 7) as f64 * k / 8.0)),
        f(width - (f64::from(cps[cps.len() - 1] % 5) - 2.0) * k / 4.0),
        f(0.18 * size + (sum % 3) as f64 * k / 8.0),
    ]
}

/// A measuring context configured for `d` (letter spacing when `spaced`).
struct SynthContext<'d> {
    d: &'d TextContent,
    spaced: bool,
}

impl Measure for SynthContext<'_> {
    fn width(&mut self, text: &[u16]) -> f64 {
        synth_width(self.d, self.spaced, text)
    }
}

impl TextMeasure for Synth {
    fn measure_lines(&mut self, d: &TextContent) -> Vec<MeasuredLine> {
        let mut context = SynthContext { d, spaced: true };
        text_source_lines(d, &mut context, d.line_height())
            .into_iter()
            .map(|line| {
                let width = synth_width(d, true, &line.text);
                let ink = synth_ink(d, width, &line.text);
                MeasuredLine {
                    ink,
                    anchor_ink: ink,
                    text: String::from_utf16_lossy(&line.text),
                    width,
                }
            })
            .collect()
    }
    fn plain_width(&mut self, d: &TextContent, text: &str) -> f64 {
        synth_width(d, false, &text.encode_utf16().collect::<Vec<_>>())
    }
    fn glyph_layout(&mut self, d: &TextContent) -> GlyphLayout {
        glyph_layout(d, &mut SynthContext { d, spaced: true })
    }
    fn spaced_width(&mut self, d: &TextContent, text: &str) -> f64 {
        synth_width(d, true, &text.encode_utf16().collect::<Vec<_>>())
    }
}

/// What a kernel run produced.
enum Outcome {
    /// The layers by composition after the operations, and the kernel's own
    /// answer.
    Done {
        layers: BTreeMap<String, Vec<V>>,
        answer: Option<V>,
    },
    Threw(JsThrow),
    /// Left to the host (three.js) or to text measurement.
    Skipped(&'static str),
}

fn comps_layers(doc: &Value) -> BTreeMap<String, Vec<V>> {
    let mut out = BTreeMap::new();
    if let Some(comps) = get(doc, "comps").and_then(Value::as_object) {
        for (id, comp) in comps.iter() {
            let layers = get(comp, "layers")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(of).collect())
                .unwrap_or_default();
            out.insert(id.to_string_lossy(), layers);
        }
    }
    out
}

fn rect_v(r: &Rect) -> V {
    V::O(BTreeMap::from([
        ("x0".to_owned(), V::N(r.x0)),
        ("x1".to_owned(), V::N(r.x1)),
        ("y0".to_owned(), V::N(r.y0)),
        ("y1".to_owned(), V::N(r.y1)),
    ]))
}

fn run_case(doc: &Document, comp: &str, case: &Value) -> Outcome {
    let expressions: Arc<dyn ExpressionEngine> = Arc::new(pm_wasm::expr::PmExpressions);
    // Created ids must not collide with the document's (the comparison
    // renames ids the input lacks).
    let mut edit = Edit::new(doc, comp, comp, Some(expressions), IdSource::new("#", 1)).unwrap();
    let kind = get(case, "kind").unwrap().as_str().unwrap();
    let t = num(case, "t");
    let mut text = NoText::default();
    let mut host = NoHost;
    let layer = || usize_of(get(case, "layer").unwrap());
    // Cases measured on the generator's synthetic canvas.
    let font = get(case, "anchor").and_then(Value::as_str) == Some("font");
    let measured = matches!(get(case, "measured"), Some(Value::Bool(true)));
    let result: Result<Option<V>, JsThrow> = match kind {
        "reparent" => {
            let parent = get(case, "parent")
                .and_then(Value::as_f64)
                .map(|p| p as usize);
            pose::reparent(&mut edit, layer(), parent, t).map(|()| None)
        }
        "moveToGroup" => {
            let target = get(case, "target")
                .and_then(Value::as_f64)
                .map(|p| p as usize);
            groups::move_to_group(&mut edit, &indexes(case, "roots"), target, t).map(|()| None)
        }
        "ungroup" => groups::ungroup(&mut edit, &indexes(case, "groups"), t).map(|ids| {
            Some(V::A(
                ids.iter()
                    .map(|id| match id {
                        JsValue::String(s) => V::S(s.to_string_lossy()),
                        JsValue::Undefined => V::U,
                        JsValue::Null => V::Z,
                        other => V::S(other.to_js_string().to_string_lossy()),
                    })
                    .collect(),
            ))
        }),
        "groupPlacement" => {
            let group = usize_of(get(case, "group").unwrap());
            groups::group_placement(&mut edit, group, t, &mut text, &mut host).map(|_| None)
        }
        "align" => {
            let mode = Mode::parse(get(case, "mode").unwrap().as_str().unwrap()).unwrap();
            align::align(
                &mut edit,
                &indexes(case, "layers"),
                mode,
                t,
                &mut text,
                &mut host,
            )
            .map(|moves| {
                Some(V::A(
                    moves
                        .iter()
                        .map(|m| {
                            V::O(BTreeMap::from([
                                ("channel".to_owned(), V::S(m.channel.to_owned())),
                                ("layer".to_owned(), V::N(m.layer as f64)),
                                ("value".to_owned(), V::N(m.value)),
                            ]))
                        })
                        .collect(),
                ))
            })
        }
        "captureSizeAnchor" => {
            let fields: Vec<JsString> = get(case, "fields")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_js_string().unwrap().clone())
                .collect();
            anchor::capture_size_anchor(&mut edit, layer(), &fields, t).map(|()| None)
        }
        "captureFontAnchor" => {
            let fields: Vec<JsString> = get(case, "fields")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_js_string().unwrap().clone())
                .collect();
            anchor::capture_font_anchor(&mut edit, layer(), &fields, t, &mut Synth).map(|()| None)
        }
        "anchorReference" if font => {
            anchor::anchor_reference(&mut edit, layer(), Kind::Font, t, &mut Synth)
                .map(|r| Some(r.as_ref().map_or(V::Z, rect_v)))
        }
        "anchorReference" => anchor::anchor_reference(&mut edit, layer(), Kind::Size, t, &mut text)
            .map(|r| Some(r.as_ref().map_or(V::Z, rect_v))),
        "anchorMoved" => {
            let b: Vec<f64> = get(case, "bounds")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(hexf)
                .collect();
            let bounds = Rect {
                x0: b[0],
                y0: b[1],
                x1: b[2],
                y1: b[3],
            };
            let kind = if font { Kind::Font } else { Kind::Size };
            anchor::anchor_moved(&mut edit, layer(), kind, t, &bounds).map(|()| None)
        }
        "leaveAttributes" => {
            comp_bounds::leave_attributes(&mut edit, layer(), t, &mut text, &mut host).map(|b| {
                Some(V::O(BTreeMap::from([
                    ("ax".to_owned(), V::N(b.ax)),
                    ("ay".to_owned(), V::N(b.ay)),
                    ("h".to_owned(), V::N(b.h)),
                    ("w".to_owned(), V::N(b.w)),
                ])))
            })
        }
        "syncPeers" => {
            let channel = get(case, "channel").unwrap().as_str().unwrap();
            materials::synchronize_peers(&mut edit, layer(), channel).map(|_| None)
        }
        "splitText" if measured => {
            let mode = SplitMode::parse(get(case, "mode").unwrap().as_str().unwrap()).unwrap();
            split_text::split_text(&mut edit, &indexes(case, "sources"), mode, t, &mut Synth).map(
                |plans| {
                    Some(V::A(
                        plans
                            .iter()
                            .flat_map(|p| p.children.iter().map(of))
                            .collect(),
                    ))
                },
            )
        }
        "splitText" => {
            let mode = SplitMode::parse(get(case, "mode").unwrap().as_str().unwrap()).unwrap();
            let sources = indexes(case, "sources");
            let layouts: Vec<GlyphLayout> =
                serde_json::from_str(get(case, "layouts").unwrap().as_str().unwrap()).unwrap();
            let offsets: Vec<(f64, f64)> = get(case, "offsets")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|o| {
                    let o = o.as_array().unwrap();
                    (hexf(&o[0]), hexf(&o[1]))
                })
                .collect();
            let by_source: HashMap<usize, (f64, f64)> =
                sources.iter().copied().zip(offsets).collect();
            split_text::split_layouts(&mut edit, &sources, mode, t, &layouts, &mut |_, s| {
                Ok(by_source[&s])
            })
            .map(|plans| {
                Some(V::A(
                    plans
                        .iter()
                        .flat_map(|p| p.children.iter().map(of))
                        .collect(),
                ))
            })
        }
        other => panic!("unknown case {other}"),
    };
    match result {
        Err(e) if e.kind == HOST_ONLY => Outcome::Skipped("host"),
        _ if text.asked => Outcome::Skipped("text"),
        Err(e) => Outcome::Threw(e),
        Ok(answer) => {
            let ops = edit.finish();
            let mut after = doc.to_value();
            pm_model::ops::apply_ops(&mut after, &ops).expect("kernel operations apply");
            Outcome::Done {
                layers: comps_layers(&after.unwrap()),
                answer,
            }
        }
    }
}

/// One tick of the native time grid (`pm_time::TICKS_PER_SECOND`).
const TICK: f64 = 1.0 / 705_600_000.0;

/// Ticks a created or moved key time may be from the TypeScript float sum:
/// pm-edit rounds each term of `t + from − to` (or `time − from`) to the
/// tick grid, half a tick each, at most three terms (rule 4 with amendment
/// 1), plus the float error of the TypeScript sum.
const KEY_TIME_TICKS: f64 = 2.0;

#[derive(Default)]
struct Stats {
    numbers: usize,
    exact: usize,
    wrapped: usize,
    /// Key times a kernel created or moved that differ from the TypeScript
    /// float sum by the tick rounding (`KEY_TIME_TICKS`), or on an NTSC
    /// composition stored with a float rate by the float grid's drift.
    ticked: usize,
    /// The largest such difference on exact rates, in ticks.
    max_ticks: f64,
    /// The composition and layer start of the key times being compared:
    /// the float grid's relative drift from the exact rate (0 for exact
    /// rates), and the layer's `from`.
    drift: f64,
    from: f64,
    max_error: f64,
    messages: usize,
    internal: usize,
    failures: Vec<String>,
}

/// Renames ids absent from the input (`i`, `id`) by first appearance.
struct Ids<'k> {
    known: &'k HashSet<String>,
    names: HashMap<String, String>,
}

impl Ids<'_> {
    fn name(&mut self, key: &str, s: &str) -> String {
        if (key == "i" || key == "id") && !self.known.contains(s) {
            let n = self.names.len();
            return self
                .names
                .entry(s.to_owned())
                .or_insert_with(|| format!("#created{n}"))
                .clone();
        }
        s.to_owned()
    }
}

/// Numbers in an expression string, and the text around them.
fn split_numbers(s: &str) -> (Vec<String>, Vec<f64>) {
    let bytes = s.as_bytes();
    let (mut texts, mut numbers) = (Vec::new(), Vec::new());
    let (mut i, mut last) = (0, 0);
    while i < bytes.len() {
        let start = i;
        let rest = &s[i..];
        let special = ["NaN", "-Infinity", "Infinity"]
            .into_iter()
            .find(|w| rest.starts_with(w));
        let mut j = i;
        if let Some(w) = special {
            j += w.len();
        } else {
            if bytes[j] == b'-'
                && j + 1 < bytes.len()
                && (bytes[j + 1].is_ascii_digit() || bytes[j + 1] == b'.')
            {
                j += 1;
            }
            let digits = j;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            if j == digits || (j == digits + 1 && bytes[digits] == b'.') {
                i += 1;
                continue;
            }
            if j < bytes.len() && (bytes[j] == b'e' || bytes[j] == b'E') {
                let mut k = j + 1;
                if k < bytes.len() && (bytes[k] == b'+' || bytes[k] == b'-') {
                    k += 1;
                }
                if k < bytes.len() && bytes[k].is_ascii_digit() {
                    while k < bytes.len() && bytes[k].is_ascii_digit() {
                        k += 1;
                    }
                    j = k;
                }
            }
        }
        texts.push(s[last..start].to_owned());
        numbers.push(s[start..j].parse::<f64>().unwrap_or(match &s[start..j] {
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            _ => f64::NAN,
        }));
        last = j;
        i = j;
    }
    texts.push(s[last..].to_owned());
    (texts, numbers)
}

impl Stats {
    fn number(&mut self, a: f64, b: f64, angle: bool) -> bool {
        self.numbers += 1;
        if a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()) || (a == 0.0 && b == 0.0) {
            self.exact += 1;
            return true;
        }
        if !a.is_finite() || !b.is_finite() {
            return false;
        }
        let e = (a - b).abs();
        if e <= 1e-9 + 1e-9 * b.abs() {
            self.max_error = self.max_error.max(e / (1.0 + b.abs()));
            return true;
        }
        // A rotation at the atan2 branch cut (amendment 20), and the turns a
        // bake adds to keep keys continuous from there.
        if angle && ((e / 360.0).round() * 360.0 - e).abs() <= 1e-9 + 1e-9 * b.abs() {
            self.wrapped += 1;
            return true;
        }
        false
    }

    /// Whether a key time differs from the TypeScript one only by the tick
    /// rounding of its terms, or (NTSC compositions stored with a float
    /// rate) by the float grid's drift (section 5, rule 4, amendment 1).
    fn key_time_moved(&self, x: f64, y: f64) -> bool {
        let e = (x - y).abs();
        let base = 1e-9 + 1e-9 * y.abs();
        if e <= base + KEY_TIME_TICKS * TICK {
            return true;
        }
        let scale = x.abs() + y.abs() + self.from.abs() + 1.0;
        self.drift > 0.0 && e <= 2.0 * self.drift * scale + base + KEY_TIME_TICKS * TICK
    }

    fn same(
        &mut self,
        a: &V,
        b: &V,
        key: &str,
        ours: &mut Ids<'_>,
        want: &mut Ids<'_>,
        path: &mut String,
    ) -> bool {
        let ok = self.compare(a, b, key, ours, want, path);
        if !ok && !matches!((a, b), (V::A(_), V::A(_)) | (V::O(_), V::O(_))) {
            let short = |v: &V| {
                let s = format!("{v:?}");
                if s.len() > 160 {
                    format!("{}…", &s[..160])
                } else {
                    s
                }
            };
            path.push_str(&format!(": ours {}, want {}", short(a), short(b)));
        }
        ok
    }

    fn compare(
        &mut self,
        a: &V,
        b: &V,
        key: &str,
        ours: &mut Ids<'_>,
        want: &mut Ids<'_>,
        path: &mut String,
    ) -> bool {
        match (a, b) {
            (V::N(x), V::N(y)) => {
                if self.number(
                    *x,
                    *y,
                    key.starts_with("rotation") || path.contains("rotation"),
                ) {
                    return true;
                }
                if key == "t" && path.contains(".kf[") && self.key_time_moved(*x, *y) {
                    self.ticked += 1;
                    if self.drift == 0.0 {
                        self.max_ticks = self.max_ticks.max((x - y).abs() / TICK);
                    }
                    return true;
                }
                false
            }
            (V::S(x), V::S(y)) => {
                let (x, y) = (ours.name(key, x), want.name(key, y));
                if x == y {
                    return true;
                }
                if key == "expr" {
                    let ((tx, nx), (ty, ny)) = (split_numbers(&x), split_numbers(&y));
                    return tx == ty
                        && nx.len() == ny.len()
                        && nx.iter().zip(&ny).all(|(p, q)| self.number(*p, *q, false));
                }
                false
            }
            (V::A(x), V::A(y)) => {
                x.len() == y.len()
                    && x.iter().zip(y).enumerate().all(|(i, (p, q))| {
                        let len = path.len();
                        path.push_str(&format!("[{i}]"));
                        let ok = self.same(p, q, key, ours, want, path);
                        if ok {
                            path.truncate(len);
                        }
                        ok
                    })
            }
            (V::O(x), V::O(y)) => {
                if x.len() != y.len() || x.keys().zip(y.keys()).any(|(p, q)| p != q) {
                    path.push_str(&format!(
                        " keys {:?} vs {:?}",
                        x.keys().collect::<Vec<_>>(),
                        y.keys().collect::<Vec<_>>()
                    ));
                    return false;
                }
                x.iter().zip(y).all(|((k, p), (_, q))| {
                    let len = path.len();
                    path.push('.');
                    path.push_str(k);
                    let ok = self.same(p, q, k, ours, want, path);
                    if ok {
                        path.truncate(len);
                    }
                    ok
                })
            }
            (x, y) => x == y,
        }
    }
}

fn known_ids(v: &Value, out: &mut HashSet<String>) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| known_ids(x, out)),
        Value::Object(m) => {
            for (k, x) in m.iter() {
                if let (Some("i" | "id"), Some(s)) = (k.as_str(), x.as_js_string()) {
                    out.insert(s.to_string_lossy());
                } else {
                    known_ids(x, out);
                }
            }
        }
        _ => {}
    }
}

/// The layers the TypeScript edit left: the changed layers of `out` over the
/// input's, composition by composition.
fn expected_layers(before: &BTreeMap<String, Vec<V>>, out: &V) -> BTreeMap<String, Vec<V>> {
    let mut result = before.clone();
    let V::O(changed) = out else { panic!("layers") };
    for (comp, entry) in changed {
        let V::O(entry) = entry else {
            panic!("comp entry")
        };
        let V::A(ids) = &entry["ids"] else {
            panic!("ids")
        };
        let V::A(pairs) = &entry["changed"] else {
            panic!("changed")
        };
        let was = before.get(comp).cloned().unwrap_or_default();
        let id_of = |l: &V| match l {
            V::O(m) => m.get("id").cloned(),
            _ => None,
        };
        let mut layers: Vec<V> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let same_place = was.get(i).filter(|l| id_of(l).as_ref() == Some(id));
                same_place
                    .or_else(|| was.iter().find(|l| id_of(l).as_ref() == Some(id)))
                    .cloned()
                    .unwrap_or(V::U)
            })
            .collect();
        for pair in pairs {
            let V::A(pair) = pair else { panic!("pair") };
            let V::N(i) = pair[0] else { panic!("index") };
            layers[i as usize] = pair[1].clone();
        }
        result.insert(comp.clone(), layers);
    }
    result
}

/// The relative drift of a composition's stored float rate from its exact
/// rate (29.97 against 30000/1001: about 1e-6; 0 for exact rates).
fn drift(canonical: &Value, comp: &str) -> f64 {
    let Some(c) = get(canonical, "comps").and_then(|m| get(m, comp)) else {
        return 0.0;
    };
    match pm_model::IngestRate::of_composition(c) {
        Ok(r) => {
            let exact = r.rate.num() as f64 / r.rate.den() as f64;
            let fps = get(c, "fps").and_then(Value::as_f64).unwrap_or(exact);
            (fps / exact - 1.0).abs()
        }
        Err(_) => 0.0,
    }
}

fn load() -> Value {
    let path = std::env::var("PM_EDIT_VECTORS").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/vectors/edit-vectors.json.gz"
        )
        .to_owned()
    });
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut text = String::new();
    flate2::read::GzDecoder::new(&bytes[..])
        .read_to_string(&mut text)
        .unwrap();
    parse(&text).unwrap()
}

fn document(text: &str) -> (Document, String, Value) {
    let project = parse(text).unwrap();
    let comp = get(&project, "compId")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let canonical = pm_model::canonical::canonical_project(project);
    (Document::new(Some(canonical.clone()), 1), comp, canonical)
}

#[test]
fn edits_match_typescript() {
    let vectors = load();
    let mut stats = Stats::default();
    let mut cases: BTreeMap<String, usize> = BTreeMap::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    for entry in get(&vectors, "docs").unwrap().as_array().unwrap() {
        let base = get(entry, "doc").unwrap().as_str().unwrap();
        let base_doc = document(base);
        for case in get(entry, "cases").unwrap().as_array().unwrap() {
            let kind = get(case, "kind").unwrap().as_str().unwrap().to_owned();
            let own;
            let (doc, comp, canonical) = match get(case, "doc").and_then(Value::as_str) {
                Some(text) => {
                    own = document(text);
                    (&own.0, &own.1, &own.2)
                }
                None => (&base_doc.0, &base_doc.1, &base_doc.2),
            };
            let want = dec(get(case, "out").unwrap());
            let outcome = run_case(doc, comp, case);
            let what = || {
                let mut c = case.clone();
                if let Value::Object(m) = &mut c {
                    m.remove_key(&JsString::from("out"));
                    m.remove_key(&JsString::from("doc"));
                    m.remove_key(&JsString::from("layouts"));
                    m.remove_key(&JsString::from("selfSpace"));
                }
                format!("{kind} {}", pm_model::json::to_canonical_string(&c))
            };
            // Cases measured on the synthetic canvas count apart.
            let label = if get(case, "anchor").and_then(Value::as_str) == Some("font") {
                format!("{kind}(font)")
            } else if matches!(get(case, "measured"), Some(Value::Bool(true))) {
                format!("{kind}(measured)")
            } else {
                kind.clone()
            };
            *cases.entry(label).or_default() += 1;
            let mut known = HashSet::new();
            known_ids(canonical, &mut known);
            let (mut ours, mut theirs) = (
                Ids {
                    known: &known,
                    names: HashMap::new(),
                },
                Ids {
                    known: &known,
                    names: HashMap::new(),
                },
            );
            // The reparents whose legacy aim reads the camera's own transform
            // through a stale cache (`selfSpace`, from the generator's own walk
            // of `parent3D`) are compared like the rest, and counted apart.
            if matches!(get(case, "selfSpace"), Some(Value::Bool(true))) {
                *cases.entry(format!("{kind}(selfSpace)")).or_default() += 1;
            }
            let mut path = String::new();
            let ok = match (outcome, &want) {
                (Outcome::Skipped(why), _) => {
                    *skipped.entry(format!("{kind}:{why}")).or_default() += 1;
                    true
                }
                // The generator's bun harness has no `PM.raster` (canvas):
                // layers whose TypeScript bounds rasterize (vector paths on
                // a shape) have no TypeScript answer there.
                (_, V::Throw(message)) if message.starts_with("PM.raster is not a function") => {
                    *skipped.entry(format!("{kind}:no-canvas")).or_default() += 1;
                    true
                }
                (Outcome::Threw(e), V::Throw(message)) => {
                    if e.kind == "Error" {
                        stats.messages += 1;
                        path = format!("message {:?} vs {message:?}", e.message);
                        e.message == *message
                    } else {
                        stats.internal += 1;
                        true
                    }
                }
                (Outcome::Threw(e), _) => {
                    path = format!("threw {}: {}", e.kind, e.message);
                    false
                }
                (Outcome::Done { .. }, V::Throw(message)) => {
                    path = format!("TypeScript threw {message:?}");
                    false
                }
                (Outcome::Done { layers, answer }, want) => {
                    let before = comps_layers(canonical);
                    let (want_layers, want_answer) = match kind.as_str() {
                        "align" | "anchorReference" | "splitText" => (None, Some(want.clone())),
                        "ungroup" | "leaveAttributes" => {
                            let V::O(m) = want else {
                                panic!("{kind} output")
                            };
                            let answer = m.get("ids").or_else(|| m.get("bounds")).cloned();
                            (Some(expected_layers(&before, &m["layers"])), answer)
                        }
                        _ => (Some(expected_layers(&before, want)), None),
                    };
                    let mut ok = true;
                    if let (Some(a), Some(b)) = (&answer, &want_answer) {
                        path.push_str("answer");
                        ok &= stats.same(a, b, "", &mut ours, &mut theirs, &mut path);
                    }
                    if ok {
                        path.clear();
                        // Kernels that only answer leave the document alone.
                        let want_layers = want_layers.unwrap_or_else(|| before.clone());
                        if layers.keys().ne(want_layers.keys()) {
                            ok = false;
                            path = format!(
                                "compositions {:?} vs {:?}",
                                layers.keys().collect::<Vec<_>>(),
                                want_layers.keys().collect::<Vec<_>>()
                            );
                        }
                        for ((comp_id, ours_layers), (_, want_layers)) in
                            layers.iter().zip(&want_layers)
                        {
                            if !ok {
                                break;
                            }
                            // Material peers carry keys across compositions:
                            // either grid's drift applies.
                            stats.drift = drift(canonical, comp_id).max(drift(canonical, comp));
                            if ours_layers.len() != want_layers.len() {
                                ok = false;
                                path = format!(
                                    ".{comp_id}: {} layers vs {}",
                                    ours_layers.len(),
                                    want_layers.len()
                                );
                                break;
                            }
                            for (i, (a, b)) in ours_layers.iter().zip(want_layers).enumerate() {
                                stats.from = match b {
                                    V::O(m) => match m.get("from") {
                                        Some(V::N(f)) => *f,
                                        _ => 0.0,
                                    },
                                    _ => 0.0,
                                };
                                path = format!(".{comp_id}[{i}]");
                                if !stats.same(a, b, "", &mut ours, &mut theirs, &mut path) {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                    }
                    ok
                }
            };
            // `PM_EDIT_DEBUG=<comp>/<layer>/<member>/…` prints that value of
            // both sides for the first mismatch.
            if !ok
                && stats.failures.is_empty()
                && let Ok(debug) = std::env::var("PM_EDIT_DEBUG")
                && let Outcome::Done { layers, .. } = run_case(doc, comp, case)
            {
                let pick = |v: &V, path: &str| -> V {
                    let mut v = v.clone();
                    for part in path.split('/') {
                        v = match (&v, part.parse::<usize>()) {
                            (V::A(a), Ok(i)) => a[i].clone(),
                            (V::O(m), _) => m[part].clone(),
                            _ => V::U,
                        };
                    }
                    v
                };
                let path = debug;
                let ours_v = V::O(layers.into_iter().map(|(k, v)| (k, V::A(v))).collect());
                eprintln!("DEBUG ours {:?}", pick(&ours_v, &path));
                let before = comps_layers(canonical);
                let want_l = match &want {
                    V::O(m) if m.contains_key("layers") => expected_layers(&before, &m["layers"]),
                    w => expected_layers(&before, w),
                };
                let want_v = V::O(want_l.into_iter().map(|(k, v)| (k, V::A(v))).collect());
                eprintln!("DEBUG want {:?}", pick(&want_v, &path));
            }
            if !ok && stats.failures.len() < 30 {
                stats.failures.push(format!("{}: {path}", what()));
            }
        }
    }
    eprintln!(
        "edit vectors: {} cases {cases:?}; {} numbers ({} bit-identical, {} angles equal modulo 360, {} created or moved key times on the tick grid (at most {:.2} ticks off outside the NTSC drift), max relative error {:.3e}); {} error messages identical, {} internal throws; skipped {skipped:?}",
        cases.values().sum::<usize>(),
        stats.numbers,
        stats.exact,
        stats.wrapped,
        stats.ticked,
        stats.max_ticks,
        stats.max_error,
        stats.messages,
        stats.internal,
    );
    for f in &stats.failures {
        eprintln!("  {f}");
    }
    assert!(
        stats.failures.is_empty(),
        "{} mismatches",
        stats.failures.len()
    );
}
