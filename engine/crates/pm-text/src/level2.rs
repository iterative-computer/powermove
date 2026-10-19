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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// UTF-16 source of each glyph's shaping atom. Ligature continuations
    /// share the first grapheme's pose; shaping is never repeated per letter.
    #[serde(default)]
    pub sources: Vec<usize>,
}

/// An evaluated typography span, in UTF-16 source coordinates. Callers
/// resolve overlapping styles before shaping, in authored order.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleSpan {
    pub start: usize,
    pub end: usize,
    pub content: TextContent,
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
    fonts: FontSet,
    lcx: LayoutContext<u32>,
}

/// A font collection with its registrations and fallback configuration.
/// A host that lays out text over several font sets swaps them through
/// [`Level2::replace_fonts`] and keeps one layout context, whose shaping
/// caches are keyed by [`FaceBlob`] identity.
pub struct FontSet {
    fcx: FontContext,
    keys: HashMap<u64, String>,
    families: BTreeMap<String, String>,
    base_family: Option<String>,
    registrations: usize,
}

impl Default for FontSet {
    fn default() -> Self {
        Self::new()
    }
}

impl FontSet {
    pub fn new() -> Self {
        let collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        FontSet {
            fcx: FontContext {
                collection,
                source_cache: Default::default(),
            },
            keys: HashMap::new(),
            families: BTreeMap::new(),
            base_family: None,
            registrations: 0,
        }
    }

    /// Fonts added so far (`add_font` and `add_face` calls).
    pub fn registrations(&self) -> usize {
        self.registrations
    }
}

/// Font bytes as Parley shares them. Parley's shaping caches are keyed by
/// the blob's identity, so a host that keeps one `FaceBlob` per registration
/// reuses shaping data whenever the face is registered again.
#[derive(Clone)]
pub struct FaceBlob(Blob<u8>);

impl FaceBlob {
    pub fn new(data: &FontBytes) -> Self {
        FaceBlob(Blob::new(data.shared()))
    }
    pub fn id(&self) -> u64 {
        self.0.id()
    }
}

impl Default for Level2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Level2 {
    pub fn new() -> Self {
        Level2 {
            fonts: FontSet::new(),
            lcx: LayoutContext::new(),
        }
    }

    /// Lays out with `fonts` from now on and returns the previous set.
    pub fn replace_fonts(&mut self, fonts: FontSet) -> FontSet {
        std::mem::replace(&mut self.fonts, fonts)
    }

    /// Adds every face in `data` under `family` (the registry's family name;
    /// a subset's own name table may name a hidden system family). `key`
    /// names the source in `Run::font`. Returns the number of faces added.
    pub fn add_font(&mut self, key: &str, data: FontBytes, family: &str) -> usize {
        self.add_face(key, &FaceBlob::new(&data), family)
    }

    /// [`Self::add_font`] with bytes the host keeps, so shaping data for a
    /// face registered again is reused.
    pub fn add_face(&mut self, key: &str, blob: &FaceBlob, family: &str) -> usize {
        let fonts = &mut self.fonts;
        fonts.keys.insert(blob.id(), key.to_string());
        fonts.registrations += 1;
        let added = fonts.fcx.collection.register_fonts(
            blob.0.clone(),
            Some(FontInfoOverride {
                family_name: Some(family),
                ..Default::default()
            }),
        );
        let n: usize = added.iter().map(|(_, faces)| faces.len()).sum();
        if n > 0 {
            fonts
                .families
                .insert(family.to_lowercase(), family.to_string());
        }
        n
    }

    /// Configures fallback from the registry's manifest: per-script families,
    /// the emoji family, and the base family as `sans-serif` and `system-ui`.
    /// Families not added yet are skipped; call again after adding fonts.
    pub fn set_manifest(&mut self, manifest: &FallbackManifest) {
        let collection = &mut self.fonts.fcx.collection;
        let id = |c: &mut Collection, name: &str| c.family_id(name);
        if let Some(base) = id(collection, &manifest.base_family) {
            collection.set_generic_families(GenericFamily::SansSerif, std::iter::once(base));
            collection.set_generic_families(GenericFamily::SystemUi, std::iter::once(base));
            self.fonts.base_family = Some(manifest.base_family.clone());
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
        self.fonts.families.values().map(String::as_str)
    }

    /// A host that resolves only the faces needed by this text can provide
    /// their ordered fallback families instead of an eager platform census.
    pub fn set_fallback_families(&mut self, families: &[String]) {
        let collection = &mut self.fonts.fcx.collection;
        let ids: Vec<_> = families
            .iter()
            .filter_map(|f| collection.family_id(f))
            .collect();
        collection.set_generic_families(GenericFamily::SansSerif, ids.iter().copied());
        collection.set_generic_families(GenericFamily::SystemUi, ids.iter().copied());
        collection.set_fallbacks(Script::from_bytes(*b"Zyyy"), ids.into_iter());
    }

    /// Lays `d` out.
    pub fn layout(&mut self, d: &TextContent) -> Level2Layout {
        self.layout_styled(d, &[])
    }

    /// Styles participate in shaping, line breaking and font-metric line
    /// boxes, including whitespace. Source ranges never cut a UTF-8 scalar.
    pub fn layout_styled(&mut self, d: &TextContent, spans: &[StyleSpan]) -> Level2Layout {
        let text = d.text.clone();
        let size = d.size() as f32;
        let (leading, weight, tracking) = (leading_of(d), weight_of(d), tracking_of(d));
        let base_family = self.fonts.base_family.clone();
        let families = Self::families_of(base_family.as_deref(), d);
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

        let mut builder = self
            .lcx
            .ranged_builder(&mut self.fonts.fcx, &text, 1.0, false);
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
        let index = Utf8Index::new(&text);
        for span in spans {
            let range = index.of(span.start)..index.of(span.end);
            if range.is_empty() {
                continue;
            }
            let s = &span.content;
            // A span normalizes exactly as the base does, so a range that
            // restates the base style lays out identically to none.
            if s.font.as_deref().is_some_and(|f| !f.trim().is_empty()) {
                builder.push(
                    StyleProperty::FontFamily(FontFamily::List(Cow::Owned(Self::families_of(
                        base_family.as_deref(),
                        s,
                    )))),
                    range.clone(),
                );
            }
            builder.push(StyleProperty::FontSize(s.size() as f32), range.clone());
            builder.push(
                StyleProperty::FontWeight(FontWeight::new(weight_of(s))),
                range.clone(),
            );
            builder.push(
                StyleProperty::FontStyle(if s.italic.unwrap_or(false) {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                }),
                range.clone(),
            );
            builder.push(StyleProperty::LetterSpacing(tracking_of(s)), range.clone());
            builder.push(
                StyleProperty::LineHeight(LineHeight::FontSizeRelative(leading_of(s))),
                range.clone(),
            );
            let mut axes: Vec<_> = s
                .axes
                .iter()
                .map(|(t, v)| axis(tag(t), *v as f32))
                .collect();
            if !s.axes.iter().any(|(t, _)| tag(t) == OPSZ) {
                axes.push(axis(OPSZ, s.size() as f32));
            }
            builder.push(
                StyleProperty::FontVariations(FontVariations::List(Cow::Owned(axes))),
                range,
            );
        }
        let mut layout: Layout<u32> = builder.build(&text);
        let box_width = d
            .box_width
            .filter(|w| w.is_finite() && *w > 0.0 && d.paragraph.unwrap_or(false));
        layout.break_all_lines(box_width.map(|w| w as f32));
        self.collect(d, &text, &layout)
    }

    /// The layer's family, then the registry's base family, then
    /// `sans-serif` (the base family through the manifest).
    fn families_of(base: Option<&str>, d: &TextContent) -> Vec<FontFamilyName<'static>> {
        let mut families: Vec<FontFamilyName<'static>> = Vec::new();
        if let Some(f) = d.font.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
            families.push(FontFamilyName::Named(Cow::Owned(f.to_string())));
        }
        if let Some(base) = base {
            families.push(FontFamilyName::Named(Cow::Owned(base.to_string())));
        }
        families.push(FontFamilyName::Generic(GenericFamily::SansSerif));
        families
    }

    fn collect(&self, d: &TextContent, text: &str, layout: &Layout<u32>) -> Level2Layout {
        let utf16_at = Utf16Index::new(text);
        let align = d.align();
        let line_height_floor = d.line_height().max(1.0);
        let box_height = d
            .box_height
            .filter(|h| h.is_finite() && *h > 0.0 && d.paragraph.unwrap_or(false));
        let mut out = Level2Layout::default();
        if text.is_empty() {
            // Parley gives an empty layout an internal 0..1 sentinel line.
            // Keep its font metrics, but expose only the authored empty
            // source, one caret and no sentinel glyphs or units.
            let mut line = Line {
                height: line_height_floor,
                ..Default::default()
            };
            if let Some(empty) = layout.lines().next() {
                line.height = (empty.metrics().line_height as f64).max(line_height_floor);
                line.baseline = empty.metrics().baseline as f64;
                for run in empty.runs() {
                    line.ascent = line.ascent.max(run.font_metrics().ascent as f64);
                    line.descent = line.descent.max(run.font_metrics().descent as f64);
                }
            }
            out.height = line.height;
            out.lines.push(line);
            out.caret.push(vec![Boundary { index: 0, x: 0.0 }]);
            return out;
        }
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

            // Parley cuts one shaped run into several glyph runs where the
            // style changes without splitting shaping (a range that differs
            // only in line height, say). The run's visual clusters are walked
            // once per line, and each glyph run takes the next clusters until
            // its glyph count is covered, so sources stay glyph-aligned.
            let mut cluster_cursor: HashMap<usize, (Vec<ClusterSource>, usize)> = HashMap::new();
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(gr) = item else {
                    continue;
                };
                let run = gr.run();
                let font = run.font();
                let key = self
                    .fonts
                    .keys
                    .get(&font.data.id())
                    .cloned()
                    .unwrap_or_default();
                let synthesis = run.synthesis();
                let mut glyphs = Vec::new();
                for g in gr.positioned_glyphs() {
                    glyphs.push(PlacedGlyph {
                        id: g.id,
                        x: g.x as f64 + shift,
                        y: g.y as f64,
                    });
                }
                let (clusters, cursor) = cluster_cursor
                    .entry(run.index())
                    .or_insert_with(|| (cluster_sources(run, text, &utf16_at), 0));
                let mut sources = Vec::with_capacity(glyphs.len());
                while sources.len() < glyphs.len() && *cursor < clusters.len() {
                    let c = &clusters[*cursor];
                    *cursor += 1;
                    sources.extend(std::iter::repeat_n(c.source, c.glyphs));
                    out.missing_glyphs += usize::from(c.missing);
                }
                debug_assert_eq!(sources.len(), glyphs.len(), "glyph run sources");
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
                    sources,
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

/// Line height relative to size, as level 1 (`d.leading || 1.15`).
fn leading_of(d: &TextContent) -> f32 {
    d.leading
        .filter(|l| *l != 0.0 && !l.is_nan())
        .unwrap_or(1.15) as f32
}

/// Weight, as level 1 (`d.weight || 500`), clamped to the CSS range.
fn weight_of(d: &TextContent) -> f32 {
    d.weight
        .filter(|w| *w != 0.0 && !w.is_nan())
        .unwrap_or(DEFAULT_WEIGHT as f64)
        .clamp(1.0, 1000.0) as f32
}

fn tracking_of(d: &TextContent) -> f32 {
    d.tracking.filter(|t| t.is_finite()).unwrap_or(0.0) as f32
}

/// One visual cluster of a run's line slice: its UTF-16 source, how many
/// glyphs it yields (a ligature's continuations yield none), and whether it
/// draws `.notdef` for a visible character.
struct ClusterSource {
    source: usize,
    glyphs: usize,
    missing: bool,
}

fn cluster_sources(
    run: &parley::Run<'_, u32>,
    text: &str,
    utf16_at: &Utf16Index,
) -> Vec<ClusterSource> {
    run.visual_clusters()
        .map(|c| {
            let range = c.text_range();
            // Controls, spaces and default-ignorables draw nothing by design.
            let visible = text[range.clone()].chars().any(|ch| {
                !ch.is_whitespace()
                    && !ch.is_control()
                    && !crate::blink::chars::is_default_ignorable(ch as u32)
            });
            let mut glyphs = 0;
            let mut notdef = false;
            for g in c.glyphs() {
                glyphs += 1;
                notdef |= g.id == 0;
            }
            ClusterSource {
                source: utf16_at.of(range.start),
                glyphs,
                missing: visible && notdef,
            }
        })
        .collect()
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
