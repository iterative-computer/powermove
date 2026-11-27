//! Captions layers: the style, word timings, block layout, placement, and
//! per-word state of the caption showing at a time (`captions/model.ts`,
//! `captions/layout.ts`, and `createCaptionRasterizer`'s `frame` and
//! `idleBounds` in `captions/render.ts`; plan section 10, "word states in
//! `pm-eval`"), and `PM.captionBounds` (`legacy/gl/raster.ts`).
//!
//! Everything here is pure apart from text measurement, which goes through
//! [`TextMeasure::spaced_width`]: `measureText(text).width` with the
//! caption's `fontStr` and letter spacing, as the rasterizer's measuring
//! canvas does.
//!
//! # Stored and normalized styles
//!
//! The rasterizer uses a stored style as it is when it has a `maxLines` key
//! (the form every writer stores), and normalizes anything else. A stored
//! style is read with JavaScript's coercions: numbers by `ToNumber`
//! (`*`, `/`, `-`, and comparisons apply it), flags by truthiness, and enum
//! strings by `===` with the legacy fallbacks. Where the legacy code *adds* a
//! stored non-number (`x0 + style.offsetX`, `style.stroke + …`) JavaScript
//! concatenates strings; the port adds numbers (see
//! `docs/engine/pm-eval-layers.md`). Project validation and every captions
//! command store normalized styles, so stored styles hold numbers.
//!
//! Strings are JavaScript strings: tokens split on ECMAScript whitespace,
//! iterate by code point (unpaired surrogates included), and count length in
//! UTF-16 units.

use pm_model::{JsString, Node, NodeKind};
use pm_text::compat::TextContent;

use crate::js_sem::{self, JsResult, JsThrow, JsValue, math_round};
use crate::math;
use crate::node::{self, Got};
use crate::scope::{Scope, js_max, js_min};
use crate::text_geom::TextMeasure;

/* ── style ──────────────────────────────────────────────────────────────── */

/// `CaptionStyle['placement']`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Bottom,
    Middle,
    Top,
}

/// `CaptionStyle['align']`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// `CaptionStyle['textCase']`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCase {
    Original,
    Upper,
    Lower,
}

/// `CaptionStyle['wordAnimation']`. `Other` is a stored value that is none
/// of the three: words are not animated, but the frame computes word states
/// (`wordAnimation !== 'none'`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordAnimation {
    None,
    Pop,
    Fade,
    Other,
}

impl Placement {
    pub fn name(self) -> &'static str {
        match self {
            Placement::Bottom => "bottom",
            Placement::Middle => "middle",
            Placement::Top => "top",
        }
    }
}

impl Align {
    pub fn name(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }
}

impl TextCase {
    pub fn name(self) -> &'static str {
        match self {
            TextCase::Original => "original",
            TextCase::Upper => "upper",
            TextCase::Lower => "lower",
        }
    }
}

impl WordAnimation {
    /// The stored name (`None` for [`WordAnimation::Other`]).
    pub fn name(self) -> Option<&'static str> {
        match self {
            WordAnimation::None => Some("none"),
            WordAnimation::Pop => Some("pop"),
            WordAnimation::Fade => Some("fade"),
            WordAnimation::Other => None,
        }
    }
}

/// A caption style as layout and drawing read it.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionStyle {
    pub preset: JsString,
    /// The family (`fontStr` drops an empty one).
    pub font: JsString,
    pub weight: f64,
    pub italic: bool,
    /// Pixels at the composition's resolution.
    pub size: f64,
    pub tracking: f64,
    /// Line height as a multiple of `size`.
    pub leading: f64,
    pub fill: JsString,
    pub stroke: f64,
    pub stroke_color: JsString,
    pub boxed: bool,
    pub box_color: JsString,
    pub box_opacity: f64,
    pub box_padding: f64,
    pub box_radius: f64,
    pub placement: Placement,
    pub align: Align,
    pub offset_x: f64,
    pub offset_y: f64,
    /// Percent of the composition height.
    pub safe_margin: f64,
    /// Percent of the composition width.
    pub max_width: f64,
    pub max_lines: f64,
    pub text_case: TextCase,
    pub highlight: bool,
    pub highlight_color: JsString,
    /// Percent.
    pub highlight_scale: f64,
    pub word_animation: WordAnimation,
}

/// `CAPTION_PRESETS` ids.
const PRESETS: [&str; 6] = ["classic", "boxed", "whisper", "spotlight", "pop", "paper"];

/// `DEFAULT_CAPTION_STYLE`.
pub fn default_caption_style() -> CaptionStyle {
    CaptionStyle {
        preset: JsString::from("classic"),
        font: JsString::from("SF Pro Display"),
        weight: 600.0,
        italic: false,
        size: 54.0,
        tracking: 0.0,
        leading: 1.2,
        fill: JsString::from("#FFFFFF"),
        stroke: 3.0,
        stroke_color: JsString::from("#000000"),
        boxed: false,
        box_color: JsString::from("#000000"),
        box_opacity: 72.0,
        box_padding: 14.0,
        box_radius: 8.0,
        placement: Placement::Bottom,
        align: Align::Center,
        offset_x: 0.0,
        offset_y: 0.0,
        safe_margin: 8.0,
        max_width: 80.0,
        max_lines: 2.0,
        text_case: TextCase::Original,
        highlight: false,
        highlight_color: JsString::from("#FFD43B"),
        highlight_scale: 100.0,
        word_animation: WordAnimation::None,
    }
}

/// `finite(value, fallback)`: a blank string is not a number.
fn finite(value: Option<&Node>, fallback: f64) -> f64 {
    let n = match node::string(value) {
        Some(s) if js_trim_is_empty(s) => f64::NAN,
        _ => node::to_number(value),
    };
    if n.is_finite() { n } else { fallback }
}

fn js_trim_is_empty(s: &JsString) -> bool {
    s.code_points().all(is_js_space)
}

/// `Math.max(min, Math.min(max, value))` for a finite value.
fn clamp(value: f64, min: f64, max: f64) -> f64 {
    js_max(min, js_min(max, value))
}

/// `color(value, fallback)`: `#rrggbb` or `#rrggbbaa`, upper-cased.
fn color(value: Option<&Node>, fallback: &JsString) -> JsString {
    match node::string(value).and_then(JsString::as_str) {
        Some(s)
            if (s.len() == 7 || s.len() == 9)
                && s.starts_with('#')
                && s[1..].bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            JsString::from(s.to_ascii_uppercase())
        }
        _ => fallback.clone(),
    }
}

fn one_of<T: Copy>(value: Option<&Node>, options: &[(&str, T)], fallback: T) -> T {
    node::string(value)
        .and_then(|s| options.iter().find(|(name, _)| s == *name))
        .map_or(fallback, |(_, v)| *v)
}

const PLACEMENTS: [(&str, Placement); 3] = [
    ("bottom", Placement::Bottom),
    ("middle", Placement::Middle),
    ("top", Placement::Top),
];
const ALIGNS: [(&str, Align); 3] = [
    ("left", Align::Left),
    ("center", Align::Center),
    ("right", Align::Right),
];
const TEXT_CASES: [(&str, TextCase); 3] = [
    ("original", TextCase::Original),
    ("upper", TextCase::Upper),
    ("lower", TextCase::Lower),
];
const WORD_ANIMATIONS: [(&str, WordAnimation); 3] = [
    ("none", WordAnimation::None),
    ("pop", WordAnimation::Pop),
    ("fade", WordAnimation::Fade),
];

/// `normalizeCaptionStyle(raw)`.
pub fn normalize_caption_style(raw: Option<&Node>) -> CaptionStyle {
    let source = raw.filter(|n| matches!(n.kind(), NodeKind::Object(_)));
    let get = |key: &str| node::member(source, key);
    let base = default_caption_style();
    let preset = match node::string(get("preset")) {
        Some(p) if p == "custom" || PRESETS.iter().any(|id| p == *id) => p.clone(),
        _ => base.preset.clone(),
    };
    let font = match node::string(get("font")) {
        Some(f) if !js_trim_is_empty(f) => f.clone(),
        _ => base.font.clone(),
    };
    CaptionStyle {
        preset,
        font,
        weight: clamp(
            math_round(finite(get("weight"), base.weight)),
            100.0,
            1000.0,
        ),
        italic: matches!(get("italic").map(Node::kind), Some(NodeKind::Bool(true))),
        size: clamp(finite(get("size"), base.size), 4.0, 1000.0),
        tracking: clamp(finite(get("tracking"), base.tracking), -50.0, 200.0),
        leading: clamp(finite(get("leading"), base.leading), 0.5, 4.0),
        fill: color(get("fill"), &base.fill),
        stroke: clamp(finite(get("stroke"), base.stroke), 0.0, 100.0),
        stroke_color: color(get("strokeColor"), &base.stroke_color),
        boxed: matches!(get("box").map(Node::kind), Some(NodeKind::Bool(true))),
        box_color: color(get("boxColor"), &base.box_color),
        box_opacity: clamp(finite(get("boxOpacity"), base.box_opacity), 0.0, 100.0),
        box_padding: clamp(finite(get("boxPadding"), base.box_padding), 0.0, 400.0),
        box_radius: clamp(finite(get("boxRadius"), base.box_radius), 0.0, 400.0),
        placement: one_of(get("placement"), &PLACEMENTS, base.placement),
        align: one_of(get("align"), &ALIGNS, base.align),
        offset_x: clamp(finite(get("offsetX"), 0.0), -100_000.0, 100_000.0),
        offset_y: clamp(finite(get("offsetY"), 0.0), -100_000.0, 100_000.0),
        safe_margin: clamp(finite(get("safeMargin"), base.safe_margin), 0.0, 45.0),
        max_width: clamp(finite(get("maxWidth"), base.max_width), 10.0, 100.0),
        max_lines: clamp(
            math_round(finite(get("maxLines"), base.max_lines)),
            1.0,
            6.0,
        ),
        text_case: one_of(get("textCase"), &TEXT_CASES, base.text_case),
        highlight: matches!(get("highlight").map(Node::kind), Some(NodeKind::Bool(true))),
        highlight_color: color(get("highlightColor"), &base.highlight_color),
        highlight_scale: clamp(
            finite(get("highlightScale"), base.highlight_scale),
            50.0,
            300.0,
        ),
        word_animation: one_of(get("wordAnimation"), &WORD_ANIMATIONS, base.word_animation),
    }
}

/// A stored style (one with `maxLines`) as the rasterizer reads it, without
/// normalization.
pub fn stored_caption_style(style: &Node) -> CaptionStyle {
    let get = |key: &str| node::member(Some(style), key);
    let num = |key: &str| node::to_number(get(key));
    let text = |key: &str| {
        let v = get(key);
        if node::truthy(v) {
            node::to_js_string(v)
        } else {
            JsString::new()
        }
    };
    let is = |key: &str, name: &str| node::is_str(get(key), name);
    CaptionStyle {
        preset: node::to_js_string(get("preset")),
        font: text("font"),
        weight: num("weight"),
        italic: node::truthy(get("italic")),
        size: num("size"),
        tracking: num("tracking"),
        leading: num("leading"),
        fill: node::to_js_string(get("fill")),
        stroke: num("stroke"),
        stroke_color: node::to_js_string(get("strokeColor")),
        boxed: node::truthy(get("box")),
        box_color: node::to_js_string(get("boxColor")),
        box_opacity: num("boxOpacity"),
        box_padding: num("boxPadding"),
        box_radius: num("boxRadius"),
        placement: if is("placement", "top") {
            Placement::Top
        } else if is("placement", "middle") {
            Placement::Middle
        } else {
            Placement::Bottom
        },
        align: if is("align", "left") {
            Align::Left
        } else if is("align", "right") {
            Align::Right
        } else {
            Align::Center
        },
        offset_x: num("offsetX"),
        offset_y: num("offsetY"),
        safe_margin: num("safeMargin"),
        max_width: num("maxWidth"),
        max_lines: num("maxLines"),
        text_case: if is("textCase", "upper") {
            TextCase::Upper
        } else if is("textCase", "lower") {
            TextCase::Lower
        } else {
            TextCase::Original
        },
        highlight: node::truthy(get("highlight")),
        highlight_color: node::to_js_string(get("highlightColor")),
        highlight_scale: num("highlightScale"),
        word_animation: match WORD_ANIMATIONS
            .iter()
            .find(|(name, _)| is("wordAnimation", name))
        {
            Some((_, w)) => *w,
            None => WordAnimation::Other,
        },
    }
}

/// The style `frame` uses for `content.style`.
pub fn frame_style(style: Option<&Node>) -> CaptionStyle {
    match style {
        Some(s) if matches!(s.kind(), NodeKind::Object(m) if m.get(b"maxLines".as_slice()).is_some()) => {
            stored_caption_style(s)
        }
        other => normalize_caption_style(other),
    }
}

/* ── text units ─────────────────────────────────────────────────────────── */

/// ECMAScript WhiteSpace and LineTerminator (`\s`, `trim`).
pub fn is_js_space(cp: u32) -> bool {
    matches!(
        cp,
        0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000
            ..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    )
}

/// `isCjkChar(char)`.
pub fn is_cjk(cp: u32) -> bool {
    matches!(
        cp,
        0x2E80..=0x2FFF
            | 0x3000..=0x303F
            | 0x3040..=0x30FF
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF
            | 0x2_0000..=0x2_FFFF
    )
}

/// `CJK_CLOSING`: punctuation that stays with the character before it.
fn is_closing(cp: u32) -> bool {
    char::from_u32(cp).is_some_and(|c| {
        "、。，．！？：；」』）】〕〉》〙〗〛…‥・ーぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ々〻!?.,;:)]}%'\"”’"
            .contains(c)
    })
}

/// `CJK_OPENING`: punctuation that stays with the character after it.
fn is_opening(cp: u32) -> bool {
    char::from_u32(cp).is_some_and(|c| "「『（【〔〈《〘〖〚([{“‘".contains(c))
}

/// A JavaScript string from code points (unpaired surrogates kept).
pub fn from_code_points(cps: &[u32]) -> JsString {
    let mut units = Vec::with_capacity(cps.len());
    for &cp in cps {
        if cp >= 0x1_0000 {
            let v = cp - 0x1_0000;
            units.push(0xD800 | (v >> 10) as u16);
            units.push(0xDC00 | (v & 0x3FF) as u16);
        } else {
            units.push(cp as u16);
        }
    }
    JsString::from_utf16_units(&units)
}

/// `TextToken`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextToken {
    pub text: JsString,
    /// A space separates this token from the previous one on its line.
    pub gap: bool,
}

/// `textTokens(paragraph)`: space-separated words, and single Chinese or
/// Japanese characters (closing punctuation stays with the character
/// before, opening punctuation with the one after).
pub fn text_tokens(paragraph: &JsString) -> Vec<TextToken> {
    let cps: Vec<u32> = paragraph.code_points().collect();
    let mut tokens: Vec<TextToken> = Vec::new();
    for chunk in cps.split(|&c| is_js_space(c)).filter(|c| !c.is_empty()) {
        let mut current: Vec<u32> = Vec::new();
        let mut gap = !tokens.is_empty();
        let mut opening = false;
        let push = |current: &mut Vec<u32>, gap: &mut bool, tokens: &mut Vec<TextToken>| {
            if !current.is_empty() {
                tokens.push(TextToken {
                    text: from_code_points(current),
                    gap: *gap,
                });
                *gap = false;
                current.clear();
            }
        };
        for &c in chunk {
            let cjk = is_cjk(c);
            let last_cjk = current.last().is_some_and(|&l| is_cjk(l));
            // A new token starts at a Chinese or Japanese character, or after
            // one, unless the character is closing punctuation or follows
            // opening punctuation.
            let joins = current.is_empty() || opening || (is_closing(c) && (cjk || last_cjk));
            if !joins && (cjk || last_cjk) {
                push(&mut current, &mut gap, &mut tokens);
            }
            current.push(c);
            opening = is_opening(c);
        }
        push(&mut current, &mut gap, &mut tokens);
    }
    tokens
}

/// `text.split('\n')`.
fn paragraphs(text: &JsString) -> Vec<JsString> {
    let cps: Vec<u32> = text.code_points().collect();
    cps.split(|&c| c == 0x0A).map(from_code_points).collect()
}

/// `cueTokens(text)`: every token, paragraph after paragraph.
pub fn cue_tokens(text: &JsString) -> Vec<TextToken> {
    paragraphs(text).iter().flat_map(text_tokens).collect()
}

/// `toLocaleUpperCase` / `toLocaleLowerCase` (default case mapping;
/// unpaired surrogates are kept).
fn map_case(text: &JsString, upper: bool) -> JsString {
    let map = |s: &str| {
        if upper {
            s.to_uppercase()
        } else {
            s.to_lowercase()
        }
    };
    if let Some(s) = text.as_str() {
        return JsString::from(map(s));
    }
    let mut out: Vec<u32> = Vec::new();
    let mut run = String::new();
    for cp in text.code_points() {
        match char::from_u32(cp) {
            Some(c) => run.push(c),
            None => {
                out.extend(map(&run).chars().map(u32::from));
                run.clear();
                out.push(cp);
            }
        }
    }
    out.extend(map(&run).chars().map(u32::from));
    from_code_points(&out)
}

/// `applyTextCase(text, textCase)`: throws for a non-string, as calling the
/// string method does.
fn apply_text_case(text: &JsValue, case: TextCase) -> JsResult<JsString> {
    let JsValue::String(s) = text else {
        return Err(JsThrow::type_error("caption text is not a string"));
    };
    Ok(match case {
        TextCase::Upper => map_case(s, true),
        TextCase::Lower => map_case(s, false),
        TextCase::Original => s.clone(),
    })
}

/* ── word timings ───────────────────────────────────────────────────────── */

/// A caption word: `{ ...word, text }`. Times are the stored values
/// (`undefined` when absent); arithmetic reads them with `ToNumber`.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionWord {
    pub text: JsString,
    pub start: JsValue,
    pub end: JsValue,
}

impl CaptionWord {
    pub fn start(&self) -> f64 {
        self.start.to_number()
    }
    pub fn end(&self) -> f64 {
        self.end.to_number()
    }
}

/// `Math.round(value * 1e6) / 1e6`.
fn round6(value: f64) -> f64 {
    math_round(value * 1e6) / 1e6
}

fn not_a_function(what: &str) -> JsThrow {
    JsThrow::type_error(format!("{what} is not a function"))
}

/// `squash(text)`: whitespace removed, lower-cased, as code points.
fn squash(text: &Got<'_>) -> JsResult<Vec<u32>> {
    let JsValue::String(s) = text.to_js() else {
        return Err(JsThrow::type_error("caption word text is not a string"));
    };
    let joined: Vec<u32> = s.code_points().filter(|&c| !is_js_space(c)).collect();
    Ok(map_case(&from_code_points(&joined), false)
        .code_points()
        .collect())
}

/// `wordsByCharacter(words, tokens)`.
fn words_by_character(words: &Node, tokens: &[JsString]) -> JsResult<Option<Vec<CaptionWord>>> {
    let Some(list) = node::array(Some(words)) else {
        return Err(not_a_function("words.forEach"));
    };
    let mut owner: Vec<usize> = Vec::new();
    for (index, word) in list.iter().enumerate() {
        let text = node::get_checked(Some(word), b"text")?;
        owner.extend(std::iter::repeat_n(index, squash(&text)?.len()));
    }
    let mut characters: Vec<u32> = Vec::new();
    for token in tokens {
        characters.extend(squash(&Got::Value(JsValue::String(token.clone())))?);
    }
    // `words.map(word => word.text).join('')`: `join` writes nullish as ''.
    let mut joined = Vec::new();
    for word in list {
        let text = node::get_checked(Some(word), b"text")?;
        if !text.is_nullish() {
            joined.extend(text.to_js().to_js_string().code_points());
        }
    }
    let all = squash(&Got::Value(JsValue::String(from_code_points(&joined))))?;
    if characters.len() != owner.len() || all != characters {
        return Ok(None);
    }
    let mut share = vec![0usize; list.len()];
    for &i in &owner {
        share[i] += 1;
    }
    let mut seen = vec![0usize; list.len()];
    let mut position = 0;
    let mut out = Vec::with_capacity(tokens.len());
    for token in tokens {
        let count = squash(&Got::Value(JsValue::String(token.clone())))?.len();
        let (mut start, mut end) = (f64::INFINITY, f64::NEG_INFINITY);
        for _ in 0..count {
            let index = owner[position];
            let word = &list[index];
            let (n, k) = (share[index] as f64, seen[index] as f64);
            seen[index] += 1;
            let w_start = node::get(word, b"start").to_js();
            let w_end = node::get(word, b"end").to_js();
            let step = js_sem::sub(&w_end, &w_start) / n;
            start = js_min(start, w_start.to_number() + step * k);
            end = js_max(end, w_start.to_number() + step * (k + 1.0));
            position += 1;
        }
        out.push(CaptionWord {
            text: token.clone(),
            start: JsValue::Number(round6(start)),
            end: JsValue::Number(round6(end)),
        });
    }
    Ok(Some(out))
}

/// The parts of a cue layout reads.
#[derive(Clone, Copy, Debug)]
pub struct CueRef<'a> {
    pub node: &'a Node,
}

impl<'a> CueRef<'a> {
    fn get(&self, key: &str) -> Got<'a> {
        node::get(self.node, key.as_bytes())
    }
    pub fn text(&self) -> JsValue {
        self.get("text").to_js()
    }
    pub fn start(&self) -> JsValue {
        self.get("start").to_js()
    }
    pub fn end(&self) -> JsValue {
        self.get("end").to_js()
    }
}

/// `cueWords({ ...cue, text })`: word timings, one per layout token.
pub fn cue_words(cue: CueRef<'_>, text: &JsString) -> JsResult<Vec<CaptionWord>> {
    let tokens: Vec<JsString> = cue_tokens(text).into_iter().map(|t| t.text).collect();
    let words = cue.get("words");
    let length = match &words {
        Got::Node(w) if !node::is_nullish(Some(w)) => node::get(w, b"length"),
        _ => Got::Undefined,
    };
    if length.truthy() {
        let w = words.node().expect("words with a length");
        if matches!(length.to_js(), JsValue::Number(n) if n == tokens.len() as f64) {
            let Some(list) = node::array(Some(w)) else {
                return Err(not_a_function("cue.words.map"));
            };
            return Ok(list
                .iter()
                .zip(&tokens)
                .map(|(word, token)| CaptionWord {
                    text: token.clone(),
                    start: node::get(word, b"start").to_js(),
                    end: node::get(word, b"end").to_js(),
                })
                .collect());
        }
        if let Some(mapped) = words_by_character(w, &tokens)? {
            return Ok(mapped);
        }
    }
    let total = tokens
        .iter()
        .fold(0.0, |sum, t| sum + t.utf16_len() as f64 + 1.0);
    let total = if total == 0.0 { 1.0 } else { total };
    let start = cue.start();
    let span = js_sem::sub(&cue.end(), &start);
    let mut at = start.to_number();
    Ok(tokens
        .into_iter()
        .map(|token| {
            let length = span * (token.utf16_len() as f64 + 1.0) / total;
            let word = CaptionWord {
                text: token,
                start: JsValue::Number(round6(at)),
                end: JsValue::Number(round6(at + length)),
            };
            at += length;
            word
        })
        .collect())
}

/// `cueIndexAt(cues, t)`: the cue showing at layer time `t`, by binary
/// search (cues are sorted and disjoint). Throws for a `null` cue it reads.
pub fn cue_index_at(cues: &[Node], t: f64) -> JsResult<Option<usize>> {
    let t = JsValue::Number(t);
    let (mut lo, mut hi) = (0isize, cues.len() as isize - 1);
    while lo <= hi {
        let mid = (lo + hi) >> 1;
        let cue = &cues[mid as usize];
        let start = node::get_checked(Some(cue), b"start")?.to_js();
        if js_sem::less_than(&t, &start) {
            hi = mid - 1;
            continue;
        }
        let end = node::get_checked(Some(cue), b"end")?.to_js();
        if js_sem::greater_equal(&t, &end) {
            lo = mid + 1;
        } else {
            return Ok(Some(mid as usize));
        }
    }
    Ok(None)
}

/* ── layout ─────────────────────────────────────────────────────────────── */

/// `LaidWord`.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidWord {
    /// Index into the cue's words.
    pub index: usize,
    pub text: JsString,
    /// Left edge relative to the block's left edge.
    pub x: f64,
    pub width: f64,
}

/// `LaidLine`.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidLine {
    pub words: Vec<LaidWord>,
    pub width: f64,
    pub y: f64,
}

/// `CaptionLayout`.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionLayout {
    pub size: f64,
    pub line_height: f64,
    pub lines: Vec<LaidLine>,
    /// The text block, without box padding.
    pub width: f64,
    pub height: f64,
    pub words: Vec<CaptionWord>,
}

#[derive(Clone, Debug)]
struct Token {
    index: usize,
    text: JsString,
    width: f64,
    break_before: bool,
    gap: bool,
}

fn wrap(tokens: &[Token], space: f64, limit: f64) -> Vec<Vec<usize>> {
    let mut lines = Vec::new();
    let mut line: Vec<usize> = Vec::new();
    let mut width = 0.0;
    for (i, token) in tokens.iter().enumerate() {
        let next = if line.is_empty() {
            token.width
        } else {
            width + if token.gap { space } else { 0.0 } + token.width
        };
        if !line.is_empty() && (token.break_before || next > limit) {
            lines.push(std::mem::take(&mut line));
            line.push(i);
            width = token.width;
        } else {
            line.push(i);
            width = next;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn line_width(tokens: &[Token], line: &[usize], space: f64) -> f64 {
    line.iter().enumerate().fold(0.0, |sum, (position, &i)| {
        let t = &tokens[i];
        sum + t.width + if position > 0 && t.gap { space } else { 0.0 }
    })
}

/// `measure(text, size)` for layout: the caption's font at `size`.
pub type Measure<'m> = dyn FnMut(&JsString, f64) -> f64 + 'm;

/// The `TextContent` the measuring canvas is set to: `fontStr({ ...style,
/// size })` and `letterSpacing = style.tracking * size / style.size px`.
pub fn measure_content(style: &CaptionStyle, size: f64) -> TextContent {
    TextContent {
        font: Some(style.font.to_string_lossy()).filter(|f| !f.is_empty()),
        weight: Some(style.weight),
        size: Some(size),
        tracking: Some(style.tracking * size / style.size),
        italic: Some(style.italic),
        ..TextContent::default()
    }
}

/// `layoutCaption(cue, style, measure, { compositionWidth, applyCase })`.
pub fn layout_caption(
    cue: CueRef<'_>,
    style: &CaptionStyle,
    measure: &mut Measure<'_>,
    composition_width: f64,
    apply_case: bool,
) -> JsResult<CaptionLayout> {
    let raw = cue.text();
    let text = if apply_case {
        apply_text_case(&raw, style.text_case)?
    } else {
        match raw {
            JsValue::String(s) => s,
            _ => return Err(JsThrow::type_error("caption text is not a string")),
        }
    };
    let words = cue_words(cue, &text)?;
    let paragraphs = paragraphs(&text);
    let padding = if style.boxed {
        style.box_padding * 2.0
    } else {
        0.0
    };
    let limit = js_max(1.0, composition_width * style.max_width / 100.0 - padding);
    let mut build = |size: f64| {
        let mut tokens: Vec<Token> = Vec::new();
        let mut index = 0;
        for paragraph in &paragraphs {
            for (position, token) in text_tokens(paragraph).into_iter().enumerate() {
                let width = measure(&token.text, size);
                tokens.push(Token {
                    index,
                    text: token.text,
                    width,
                    break_before: position == 0 && !tokens.is_empty(),
                    gap: token.gap,
                });
                index += 1;
            }
        }
        let space = measure(&JsString::from(" "), size);
        (tokens, space)
    };

    let mut size = style.size;
    let (mut tokens, mut space) = build(size);
    let mut lines = wrap(&tokens, space, limit);
    let mut step = 0;
    while (lines.len() as f64) > style.max_lines && step < 8 {
        size = js_max(style.size * 0.6, size * 0.94);
        (tokens, space) = build(size);
        lines = wrap(&tokens, space, limit);
        if size <= style.size * 0.6 {
            break;
        }
        step += 1;
    }

    if lines.len() > 1 && !tokens.iter().any(|t| t.break_before) {
        let mut lo = tokens
            .iter()
            .fold(f64::NEG_INFINITY, |m, t| js_max(m, t.width));
        let mut hi = limit;
        let mut step = 0;
        while step < 12 && hi - lo > 1.0 {
            let mid = (lo + hi) / 2.0;
            if wrap(&tokens, space, mid).len() <= lines.len() {
                hi = mid;
            } else {
                lo = mid;
            }
            step += 1;
        }
        lines = wrap(&tokens, space, hi);
    }

    let line_height = size * style.leading;
    let widths: Vec<f64> = lines
        .iter()
        .map(|l| line_width(&tokens, l, space))
        .collect();
    let width = widths.iter().fold(1.0, |m, &w| js_max(m, w));
    let laid = lines
        .iter()
        .enumerate()
        .map(|(row, line)| {
            let w = widths[row];
            let mut x = match style.align {
                Align::Left => 0.0,
                Align::Right => width - w,
                Align::Center => (width - w) / 2.0,
            };
            let words = line
                .iter()
                .enumerate()
                .map(|(position, &i)| {
                    let token = &tokens[i];
                    if position > 0 && token.gap {
                        x += space;
                    }
                    let word = LaidWord {
                        index: token.index,
                        text: token.text.clone(),
                        x,
                        width: token.width,
                    };
                    x += token.width;
                    word
                })
                .collect();
            LaidLine {
                words,
                width: w,
                y: row as f64 * line_height,
            }
        })
        .collect();
    Ok(CaptionLayout {
        size,
        line_height,
        lines: laid,
        width,
        height: js_max(line_height, lines.len() as f64 * line_height),
        words,
    })
}

/// `BlockPlacement`: the block in layer space (origin at the composition
/// centre), box padding included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// `placeBlock(layout, style, compositionWidth, compositionHeight)`.
pub fn place_block(
    width: f64,
    height: f64,
    style: &CaptionStyle,
    composition_width: f64,
    composition_height: f64,
) -> Block {
    let padding = if style.boxed { style.box_padding } else { 0.0 };
    let w = width + padding * 2.0;
    let h = height + padding * 2.0;
    let column = composition_width * style.max_width / 100.0;
    let safe = composition_height * style.safe_margin / 100.0;
    let x0 = match style.align {
        Align::Left => -column / 2.0,
        Align::Right => column / 2.0 - w,
        Align::Center => -w / 2.0,
    };
    let y0 = match style.placement {
        Placement::Top => -composition_height / 2.0 + safe,
        Placement::Middle => -h / 2.0,
        Placement::Bottom => composition_height / 2.0 - safe - h,
    };
    Block {
        x0: x0 + style.offset_x,
        y0: y0 + style.offset_y,
        x1: x0 + style.offset_x + w,
        y1: y0 + style.offset_y + h,
    }
}

/* ── word state ─────────────────────────────────────────────────────────── */

/// `WordState`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WordState {
    pub alpha: f64,
    pub scale: f64,
    pub highlighted: bool,
}

const POP: f64 = 0.18;
const FADE: f64 = 0.16;
const HIGHLIGHT_IN: f64 = 0.08;

fn ease_out_back(t: f64) -> f64 {
    let c = 1.6;
    1.0 + (c + 1.0) * math::pow(t - 1.0, 3.0) + c * math::pow(t - 1.0, 2.0)
}

/// `wordStates(words, style, t)`: each word's appearance at layer time `t`,
/// quantized to 1/60 s.
pub fn word_states(words: &[CaptionWord], style: &CaptionStyle, t: f64) -> Vec<WordState> {
    let time = math_round(t * 60.0) / 60.0;
    let time_v = JsValue::Number(time);
    words
        .iter()
        .map(|word| {
            let mut alpha = 1.0;
            let mut scale = 1.0;
            let since = time - word.start();
            if style.word_animation == WordAnimation::Fade {
                alpha = if since < 0.0 {
                    0.0
                } else {
                    js_min(1.0, since / FADE)
                };
            }
            if style.word_animation == WordAnimation::Pop {
                alpha = if since < 0.0 {
                    0.0
                } else {
                    js_min(1.0, since / (POP / 2.0))
                };
                scale = if since < 0.0 {
                    0.7
                } else if since >= POP {
                    1.0
                } else {
                    0.7 + 0.3 * ease_out_back(since / POP)
                };
            }
            let highlighted = style.highlight
                && js_sem::greater_equal(&time_v, &word.start)
                && js_sem::less_than(&time_v, &word.end);
            if highlighted && style.highlight_scale != 100.0 {
                let k = js_min(1.0, js_max(0.0, since / HIGHLIGHT_IN));
                scale *= 1.0 + (style.highlight_scale / 100.0 - 1.0) * k;
            }
            WordState {
                alpha: math_round(alpha * 100.0) / 100.0,
                scale: math_round(scale * 1000.0) / 1000.0,
                highlighted,
            }
        })
        .collect()
}

/* ── frame ──────────────────────────────────────────────────────────────── */

/// What a captions layer shows at a layer time (`frame` without the raster
/// cache key).
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionFrame {
    /// The cue's index in `content.cues`.
    pub cue: usize,
    pub style: CaptionStyle,
    pub layout: CaptionLayout,
    /// Per word, when words are highlighted or animated; empty otherwise.
    pub states: Vec<WordState>,
    pub block: Block,
    /// Room around the block for strokes and scaled words.
    pub margin: f64,
}

/// `frame(content, local, compositionWidth, compositionHeight)`: the caption
/// showing at layer-local time `local`, or `None` between cues.
pub fn caption_frame(
    content: Option<&Node>,
    local: f64,
    composition_width: f64,
    composition_height: f64,
    measure: &mut dyn TextMeasure,
) -> JsResult<Option<CaptionFrame>> {
    let content = content.filter(|c| !node::is_nullish(Some(c)));
    let cues = node::array(node::member(content, "cues")).unwrap_or(&[]);
    let Some(index) = cue_index_at(cues, local)? else {
        return Ok(None);
    };
    let cue = CueRef { node: &cues[index] };
    let style = frame_style(node::member(content, "style"));
    let mut m = |text: &JsString, size: f64| {
        measure.spaced_width(&measure_content(&style, size), &text.to_string_lossy())
    };
    let layout = layout_caption(cue, &style, &mut m, composition_width, true)?;
    let animated = style.highlight || style.word_animation != WordAnimation::None;
    let states = if animated {
        word_states(&layout.words, &style, local)
    } else {
        Vec::new()
    };
    let block = place_block(
        layout.width,
        layout.height,
        &style,
        composition_width,
        composition_height,
    );
    let growth = js_max(
        js_max(1.0, style.highlight_scale / 100.0),
        if style.word_animation == WordAnimation::Pop {
            1.1
        } else {
            1.0
        },
    );
    let margin = (style.stroke + layout.size * (growth - 1.0) + 4.0).ceil();
    Ok(Some(CaptionFrame {
        cue: index,
        style,
        layout,
        states,
        block,
        margin,
    }))
}

/// `idleBounds(content, compositionWidth, compositionHeight)`: where the
/// next caption will sit, one line tall, when no cue is showing.
pub fn idle_block(
    content: Option<&Node>,
    composition_width: f64,
    composition_height: f64,
) -> Block {
    let content = content.filter(|c| !node::is_nullish(Some(c)));
    let style = normalize_caption_style(node::member(content, "style"));
    let line_height = style.size * style.leading;
    let padding = if style.boxed {
        style.box_padding * 2.0
    } else {
        0.0
    };
    place_block(
        composition_width * style.max_width / 100.0 - padding,
        line_height,
        &style,
        composition_width,
        composition_height,
    )
}

impl<'d> Scope<'_, 'd> {
    /// The caption a captions layer shows at composition time `time`
    /// (`captions.frame(L.d, time - L.from, comp.w, comp.h)` with the
    /// composition being evaluated).
    pub fn caption_frame(
        &mut self,
        layer: &'d Node,
        time: f64,
        measure: &mut dyn TextMeasure,
    ) -> JsResult<Option<CaptionFrame>> {
        let local = time - node::to_number(node::member(Some(layer), "from"));
        let (w, h) = self.comp_size();
        caption_frame(node::member(Some(layer), "d"), local, w, h, measure)
    }

    /// `PM.captionBounds(L, T)`: the showing caption's block, or the idle
    /// column one line tall; the anchor at the origin.
    pub fn caption_bounds(
        &mut self,
        layer: &'d Node,
        time: f64,
        measure: &mut dyn TextMeasure,
    ) -> JsResult<Block> {
        let (w, h) = self.comp_size();
        Ok(match self.caption_frame(layer, time, measure)? {
            Some(f) => f.block,
            None => idle_block(node::member(Some(layer), "d"), w, h),
        })
    }
}
