//! Level 2 layout on Parley (plan section 12; section 26 amendment 10).
//!
//! Level 2 is the engine's own text layout, not a reproduction of Canvas2D:
//! Unicode line breaking (UAX #14) with grapheme-aware emergency breaks for
//! words wider than the box, line boxes from font metrics (the baseline sits
//! at half-leading plus the ascent, CSS style), and Parley's font matching
//! and per-script fallback over a font collection the host fills.
//!
//! Fonts never come from the platform here: the host adds faces (full fonts
//! natively, metrics-and-shaping subsets in `pm-wasm`) under the family names
//! the registry reports, and configures fallback from the registry's
//! `FallbackManifest`. Native and WASM builds given the same fonts produce
//! identical layouts.
//!
//! Coordinates follow level 1: x from the alignment origin (center and right
//! alignment place lines around x = 0), y down from the first line's top,
//! source offsets in UTF-16 code units.

use crate::compat::{Align, Boundary, GlyphLayout, PlacedGlyph, TextContent, Unit};
use crate::font::{FontBytes, OPSZ, tag};
use crate::registry::{COMMON_SCRIPT, FallbackManifest};
use parlance::{FontVariation, Tag};
use parley::fontique::{
    Blob, Collection, CollectionOptions, FallbackKey, FontInfoOverride, GenericFamily, Script,
};
use parley::{
    Affinity, Cursor, FontContext, FontFamily, FontFamilyName, FontStyle, FontVariations,
    FontWeight, Layout, LayoutContext, LineHeight, OverflowWrap, PositionedLayoutItem,
    StyleProperty, WhiteSpaceCollapse,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

/// Default weight, as level 1 (`d.weight || 500`).
const DEFAULT_WEIGHT: f32 = 500.0;

/// One laid-out line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub text: String,
    /// UTF-16 source range of the line's content (without its line break).
    pub start: usize,
    pub end: usize,
    /// Left edge after alignment.
    pub x: f64,
    /// Top of the line box.
    pub y: f64,
    pub baseline: f64,
    /// Advance of the line's content, trailing whitespace excluded.
    pub width: f64,
    pub height: f64,
    pub ascent: f64,
    pub descent: f64,
}

/// Glyphs of one face, size, and variation location on one line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub line: usize,
    /// The key the face was added under (`Level2::add_font`).
    pub font: String,
    pub size: f32,
    /// Normalized variation coordinates (F2Dot14 bits), in `fvar` axis order.
    pub coords: Vec<i16>,
    pub synthetic_bold: bool,
    /// Synthetic oblique angle in degrees (0 when none).
    pub skew: f32,
    pub glyphs: Vec<PlacedGlyph>,
}

/// A level 2 layout.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Level2Layout {
    pub lines: Vec<Line>,
    pub runs: Vec<Run>,
    /// Caret boundaries per line at grapheme boundaries (UTF-16 index, x).
    pub caret: Vec<Vec<Boundary>>,
    /// Character, word, and line units in level 1's shape, for animators.
    pub units: GlyphLayout,
    pub width: f64,
    pub height: f64,
    /// Visible clusters no added face covers (drawn as `.notdef`): the host
    /// adds fonts and lays the text out again.
    pub missing_glyphs: usize,
}

/// The level 2 layout engine over a host-filled font collection.
pub struct Level2 {
    fcx: FontContext,
    lcx: LayoutContext<u32>,
    keys: HashMap<u64, String>,
    families: BTreeMap<String, String>,
    base_family: Option<String>,
}

impl Default for Level2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Level2 {
    pub fn new() -> Self {
        let collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        Level2 {
            fcx: FontContext {
                collection,
                source_cache: Default::default(),
            },
            lcx: LayoutContext::new(),
            keys: HashMap::new(),
            families: BTreeMap::new(),
            base_family: None,
        }
    }

    /// Adds every face in `data` under `family` (the registry's family name;
    /// a subset's own name table may name a hidden system family). `key`
    /// names the source in `Run::font`. Returns the number of faces added.
    pub fn add_font(&mut self, key: &str, data: FontBytes, family: &str) -> usize {
        let blob = Blob::new(data.shared());
        self.keys.insert(blob.id(), key.to_string());
        let added = self.fcx.collection.register_fonts(
            blob,
            Some(FontInfoOverride {
                family_name: Some(family),
                ..Default::default()
            }),
        );
        let n: usize = added.iter().map(|(_, faces)| faces.len()).sum();
        if n > 0 {
            self.families
                .insert(family.to_lowercase(), family.to_string());
        }
        n
    }

    /// Configures fallback from the registry's manifest: per-script families,
    /// the emoji family, and the base family as `sans-serif` and `system-ui`.
    /// Families not added yet are skipped; call again after adding fonts.
    pub fn set_manifest(&mut self, manifest: &FallbackManifest) {
        let collection = &mut self.fcx.collection;
        let id = |c: &mut Collection, name: &str| c.family_id(name);
        if let Some(base) = id(collection, &manifest.base_family) {
            collection.set_generic_families(GenericFamily::SansSerif, std::iter::once(base));
            collection.set_generic_families(GenericFamily::SystemUi, std::iter::once(base));
            self.base_family = Some(manifest.base_family.clone());
        }
        if let Some(emoji) = manifest.emoji.as_deref().and_then(|e| id(collection, e)) {
            collection.set_generic_families(GenericFamily::Emoji, std::iter::once(emoji));
        }
        // Families for common-script characters come after every script's own.
        let mut common: Vec<String> = Vec::new();
        for s in manifest
            .scripts
            .iter()
            .filter(|s| s.script == COMMON_SCRIPT)
        {
            for f in &s.families {
                if !common.contains(f) {
                    common.push(f.clone());
                }
            }
        }
        let mut per_key: BTreeMap<(String, Option<String>), Vec<String>> = BTreeMap::new();
        for s in manifest
            .scripts
            .iter()
            .filter(|s| s.script != COMMON_SCRIPT)
        {
            per_key
                .entry((s.script.clone(), s.language.clone()))
                .or_default()
                .extend(s.families.iter().cloned());
        }
        // Text without a locale uses the script's default list: the first
        // language's families, then the families listed without a language.
        let scripts: Vec<String> = per_key.keys().map(|k| k.0.clone()).collect();
        for script in scripts {
            let first = per_key
                .iter()
                .find(|(k, _)| k.0 == script && k.1.is_some())
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let default = per_key.entry((script, None)).or_default();
            for (i, f) in first.into_iter().enumerate() {
                if !default.contains(&f) {
                    default.insert(i, f);
                }
            }
        }
        per_key
            .entry((COMMON_SCRIPT.to_string(), None))
            .or_default();
        for ((script, language), mut families) in per_key {
            for f in &common {
                if !families.contains(f) {
                    families.push(f.clone());
                }
            }
            let bytes: [u8; 4] = match script.as_bytes().try_into() {
                Ok(b) => b,
                Err(_) => continue,
            };
            let ids: Vec<_> = families.iter().filter_map(|f| id(collection, f)).collect();
            if ids.is_empty() {
                continue;
            }
            let key: FallbackKey = match &language {
                Some(l) => (Script::from_bytes(bytes), l.as_str()).into(),
                None => Script::from_bytes(bytes).into(),
            };
            collection.set_fallbacks(key, ids.into_iter());
        }
    }

    /// Families added so far.
    pub fn families(&self) -> impl Iterator<Item = &str> {
        self.families.values().map(String::as_str)
    }

    /// Lays `d` out.
    pub fn layout(&mut self, d: &TextContent) -> Level2Layout {
        let text = d.text.clone();
        let size = d.size() as f32;
        let leading = d
            .leading
            .filter(|l| *l != 0.0 && !l.is_nan())
            .unwrap_or(1.15) as f32;
        let weight = d
            .weight
            .filter(|w| *w != 0.0 && !w.is_nan())
            .unwrap_or(DEFAULT_WEIGHT as f64)
            .clamp(1.0, 1000.0) as f32;
        let tracking = d.tracking.filter(|t| t.is_finite()).unwrap_or(0.0) as f32;
        let mut families: Vec<FontFamilyName<'static>> = Vec::new();
        if let Some(f) = d.font.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
            families.push(FontFamilyName::Named(Cow::Owned(f.to_string())));
        }
        if let Some(base) = &self.base_family {
            families.push(FontFamilyName::Named(Cow::Owned(base.clone())));
        }
        families.push(FontFamilyName::Generic(GenericFamily::SansSerif));
        // CoreText optical sizing: `opsz` follows the size unless the layer
        // sets it.
        let axis = |t: u32, v: f32| FontVariation::new(Tag::from_bytes(t.to_be_bytes()), v);
        let mut variations: Vec<FontVariation> = d
            .axes
            .iter()
            .map(|(t, v)| axis(tag(t), *v as f32))
            .collect();
        if !d.axes.iter().any(|(t, _)| tag(t) == OPSZ) {
            variations.push(axis(OPSZ, size));
        }

        let mut builder = self.lcx.ranged_builder(&mut self.fcx, &text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(Cow::Owned(
            families,
        ))));
        builder.push_default(StyleProperty::FontSize(size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
        builder.push_default(StyleProperty::FontStyle(if d.italic.unwrap_or(false) {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }));
        builder.push_default(StyleProperty::FontVariations(FontVariations::List(
            Cow::Owned(variations),
        )));
        builder.push_default(StyleProperty::LetterSpacing(tracking));
        builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
            leading,
        )));
        builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::BreakWord));
        builder.push_default(StyleProperty::WhiteSpaceCollapse(
            WhiteSpaceCollapse::Preserve,
        ));
        let mut layout: Layout<u32> = builder.build(&text);
        let box_width = d
            .box_width
            .filter(|w| w.is_finite() && *w > 0.0 && d.paragraph.unwrap_or(false));
        layout.break_all_lines(box_width.map(|w| w as f32));
        self.collect(d, &text, &layout)
    }

    fn collect(&self, d: &TextContent, text: &str, layout: &Layout<u32>) -> Level2Layout {
        let utf16_at = Utf16Index::new(text);
        let align = d.align();
        let line_height_floor = d.line_height().max(1.0);
        let box_height = d
            .box_height
            .filter(|h| h.is_finite() && *h > 0.0 && d.paragraph.unwrap_or(false));
        let mut out = Level2Layout::default();
        let mut top = 0.0f64;
        let (mut char_index, mut word_index) = (0usize, 0usize);
        for (row, line) in layout.lines().enumerate() {
            let m = line.metrics();
            let height = m.line_height as f64;
            if let Some(h) = box_height {
                // Keep lines that fit the box, and always the first.
                if row > 0 && top + height > h.max(line_height_floor) + 1e-6 {
                    break;
                }
            }
            let width = (m.advance - m.hanging_advance) as f64;
            let shift = match align {
                Align::Left => 0.0,
                Align::Center => -width / 2.0,
                Align::Right => -width,
            } - m.offset as f64;
            let range = line.text_range();
            // Content without the hard break that ends the line.
            let content_end = text[range.clone()]
                .trim_end_matches(['\n', '\r', '\u{2028}', '\u{2029}'])
                .len()
                + range.start;
            let baseline = m.baseline as f64;
            let ascent = line
                .runs()
                .map(|r| r.font_metrics().ascent as f64)
                .fold(0.0, f64::max);
            let descent = line
                .runs()
                .map(|r| r.font_metrics().descent as f64)
                .fold(0.0, f64::max);
            let (start16, end16) = (utf16_at.of(range.start), utf16_at.of(content_end));
            let line_text = &text[range.start..content_end];
            out.lines.push(Line {
                text: line_text.to_string(),
                start: start16,
                end: end16,
                x: shift,
                y: top,
                baseline,
                width,
                height,
                ascent,
                descent,
            });

            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(gr) = item else {
                    continue;
                };
                let run = gr.run();
                let font = run.font();
                let key = self.keys.get(&font.data.id()).cloned().unwrap_or_default();
                let synthesis = run.synthesis();
                let mut glyphs = Vec::new();
                for g in gr.positioned_glyphs() {
                    glyphs.push(PlacedGlyph {
                        id: g.id,
                        x: g.x as f64 + shift,
                        y: g.y as f64,
                    });
                }
                // `.notdef` for a visible character (controls, spaces, and
                // default-ignorables draw nothing by design).
                for cluster in run.clusters() {
                    let t = &text[cluster.text_range()];
                    let visible = t.chars().any(|c| {
                        !c.is_whitespace()
                            && !c.is_control()
                            && !crate::blink::chars::is_default_ignorable(c as u32)
                    });
                    if visible && cluster.glyphs().any(|g| g.id == 0) {
                        out.missing_glyphs += 1;
                    }
                }
                out.runs.push(Run {
                    line: row,
                    font: key,
                    size: run.font_size(),
                    coords: run
                        .normalized_coords()
                        .iter()
                        .map(|c| c.to_bits())
                        .collect(),
                    synthetic_bold: synthesis.embolden(),
                    skew: synthesis.skew().unwrap_or(0.0),
                    glyphs,
                });
            }

            // Caret boundaries at grapheme boundaries of the line's content.
            let mut caret = Vec::new();
            let mut bounds: Vec<usize> =
                crate::compat::graphemes(&line_text.encode_utf16().collect::<Vec<_>>())
                    .iter()
                    .map(|r| r.start)
                    .collect();
            bounds.push(end16 - start16);
            let to_byte = Utf8Index::new(line_text);
            for (i, b16) in bounds.iter().enumerate() {
                let byte = range.start + to_byte.of(*b16);
                let affinity = if i + 1 == bounds.len() && i > 0 {
                    Affinity::Upstream
                } else {
                    Affinity::Downstream
                };
                let x = Cursor::from_byte_index(layout, byte, affinity)
                    .geometry(layout, 0.0)
                    .x0
                    + shift;
                caret.push(Boundary {
                    index: start16 + b16,
                    x,
                });
            }

            // Units in level 1's shape (x and w from caret edges, so RTL and
            // ligatures are covered).
            let units: Vec<u16> = line_text.encode_utf16().collect();
            let y = top;
            if !units.is_empty() {
                out.units.lines.push(Unit {
                    text: line_text.to_string(),
                    x: shift,
                    y,
                    w: width,
                    line: row,
                    source_start: start16,
                    index: out.units.lines.len(),
                    line_unit: None,
                    word: None,
                });
            }
            let line_unit = out.units.lines.len().saturating_sub(1);
            let edge = |a: usize, b: usize| {
                let (xa, xb) = (caret_x(&caret, start16 + a), caret_x(&caret, start16 + b));
                (xa.min(xb), (xb - xa).abs())
            };
            let mut local_word: i64 = -1;
            let mut was_space = true;
            for r in crate::compat::graphemes(&units) {
                let seg = &units[r.clone()];
                let space = seg.iter().all(|&u| crate::blink::chars::is_js_space(u));
                if !space && was_space {
                    local_word += 1;
                }
                if !space {
                    let (x, w) = edge(r.start, r.end);
                    out.units.characters.push(Unit {
                        text: String::from_utf16_lossy(seg),
                        x,
                        y,
                        w,
                        line: row,
                        source_start: start16 + r.start,
                        index: char_index,
                        line_unit: Some(line_unit),
                        word: Some((word_index as i64 + local_word) as usize),
                    });
                    char_index += 1;
                }
                was_space = space;
            }
            let mut i = 0;
            while i < units.len() {
                if crate::blink::chars::is_js_space(units[i]) {
                    i += 1;
                    continue;
                }
                let s = i;
                while i < units.len() && !crate::blink::chars::is_js_space(units[i]) {
                    i += 1;
                }
                // Word edges at grapheme boundaries (a word never splits one).
                let (x, w) = edge(s, i);
                out.units.words.push(Unit {
                    text: String::from_utf16_lossy(&units[s..i]),
                    x,
                    y,
                    w,
                    line: row,
                    source_start: start16 + s,
                    index: word_index,
                    line_unit: None,
                    word: None,
                });
                word_index += 1;
            }
            out.caret.push(caret);
            out.width = out.width.max(width);
            top += height;
        }
        out.height = top;
        out
    }
}

/// The caret x at a UTF-16 index, or the nearest boundary before it (an
/// index inside a grapheme, which word edges never produce).
fn caret_x(caret: &[Boundary], index: usize) -> f64 {
    caret
        .iter()
        .rev()
        .find(|b| b.index <= index)
        .or(caret.first())
        .map(|b| b.x)
        .unwrap_or(0.0)
}

/// UTF-8 byte offset to UTF-16 offset.
struct Utf16Index(Vec<usize>);

impl Utf16Index {
    fn new(s: &str) -> Self {
        let mut v = vec![0usize; s.len() + 1];
        let mut u = 0;
        for (b, c) in s.char_indices() {
            v[b] = u;
            u += c.len_utf16();
        }
        v[s.len()] = u;
        // Bytes inside a character map to the character's start.
        let mut last = 0;
        for (b, slot) in v.iter_mut().enumerate() {
            if s.is_char_boundary(b) {
                last = *slot;
            } else {
                *slot = last;
            }
        }
        Utf16Index(v)
    }
    fn of(&self, byte: usize) -> usize {
        self.0[byte.min(self.0.len() - 1)]
    }
}

/// UTF-16 offset to UTF-8 byte offset.
struct Utf8Index(Vec<usize>);

impl Utf8Index {
    fn new(s: &str) -> Self {
        let mut v = Vec::with_capacity(s.len() + 1);
        for (b, c) in s.char_indices() {
            for _ in 0..c.len_utf16() {
                v.push(b);
            }
        }
        v.push(s.len());
        Utf8Index(v)
    }
    fn of(&self, unit: usize) -> usize {
        self.0[unit.min(self.0.len() - 1)]
    }
}
