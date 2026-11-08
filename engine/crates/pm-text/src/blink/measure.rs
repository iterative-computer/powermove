//! Canvas `measureText` width, ported from Blink's `PlainTextNode` (space
//! normalization, bidi runs, word-by-word items) and
//! `ShapeResult::ApplySpacing`.

use super::chars;
use super::segment::next_cp;
use super::shaper::{FontSetup, GroupFont, ShapeResult, Shaper};
use crate::font::{FaceId, FontDesc, FontResolver, PreparedCache};
use icu_properties::props::Script;

/// A family in the canvas `font` shorthand.
#[derive(Clone, Debug)]
pub enum Family {
    /// A quoted or non-generic family name.
    Named(String),
    /// `sans-serif`, `serif`, `monospace`, `cursive`, `fantasy`.
    Generic(&'static str),
    /// A `FontFace` registered from bytes, with its `variationSettings`.
    Custom {
        face: FaceId,
        settings: Vec<(u32, f32)>,
    },
}

/// Chrome's default generic families on macOS (`locale_settings_mac.grd`).
pub fn generic_family(name: &str) -> &'static str {
    match name {
        "serif" => "Times",
        "monospace" => "Courier",
        "cursive" => "Apple Chancery",
        "fantasy" => "Papyrus",
        _ => "Helvetica",
    }
}

/// Platform data Blink's renderer-wide font cache already holds when a
/// canvas asks for a font.
///
/// `FontPlatformDataCache` keys platform data by family, effective size, and
/// selection request (weight, width, slope), plus the complete platform
/// identity carried by `cache_key`, but not by specified size, and
/// `FontPlatformDataFromCTFont` sets `opsz` from the specified size of the
/// description that created the entry. Whoever creates an entry first fixes
/// its optical size for every later user of the key in that renderer. DOM
/// text is laid out at its CSS size times the zoom (on macOS, the device
/// scale factor applies as zoom), so UI text at 12 px on a 2x display
/// creates the 24 px entry with `opsz` 12 (clamped to the axis minimum, 17,
/// for the system font), and a canvas measuring 24 px text in the same
/// family and identical platform key afterwards gets SF's text-size advances
/// (about 3.7% wider than at `opsz` 24).
///
/// A seed declares one such entry, so level 1 can reproduce a renderer whose
/// cache is in that state. Without seeds, every entry is the canvas's own
/// (`opsz` at the canvas's specified size), which is what a fresh renderer
/// measures.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FontCacheSeed {
    /// The family name the entry was created for (`FontFaceCreationParams`,
    /// compared ignoring ASCII case), e.g. `system-ui`.
    pub family: String,
    pub weight: f32,
    pub italic: bool,
    /// The effective size (the cache key's size).
    pub size: f32,
    /// The specified size of the description that created the entry.
    pub specified: f32,
    /// Complete Blink platform-data identity; legacy static captures use
    /// the default Canvas2D key when absent.
    #[serde(default, rename = "cacheKey", skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
}

/// The CSS sizes of the app's own UI text that create `system-ui` entries in
/// a default-density editor renderer's font cache at startup (`--fs-sm` and
/// `--fs-md`, weight 400, upright; `docs/engine/text.md`, "The renderer's
/// font cache"): the model for tools with no window to ask (corpus tests,
/// parity, `pm-cli`). In the app, main sends each window's seeds as its
/// renderer reports the UI text it laid out (the UI font census).
pub const APP_UI_TEXT_SIZES: [f32; 2] = [12.0, 13.0];

/// The seeds of an editor renderer whose window has been on displays with
/// these device scale factors (each scale factor's entries stay in the
/// renderer's cache for the life of the window), in that order. Equal to
/// `appUiFontCacheSeeds` in `apps/desktop/src/shared/engine-fonts-seeds.ts`.
pub fn app_ui_font_cache_seeds(scale_factors: &[f32]) -> Vec<FontCacheSeed> {
    let mut out: Vec<FontCacheSeed> = Vec::new();
    for &scale in scale_factors {
        if !(scale.is_finite() && scale > 0.0) {
            continue;
        }
        for css in APP_UI_TEXT_SIZES {
            let seed = FontCacheSeed {
                family: "system-ui".into(),
                weight: 400.0,
                italic: false,
                size: crate::compat::canvas::effective_font_size(css * scale),
                specified: css,
                cache_key: None,
            };
            // The first creator of a key fixes it.
            if !out
                .iter()
                .any(|s| s.size == seed.size && s.weight == seed.weight && s.italic == seed.italic)
            {
                out.push(seed);
            }
        }
    }
    out
}

/// The names Blink resolves to the system font on macOS. They share one
/// font cache key: the app's UI text in `-apple-system` (`--f-ui`) creates
/// the entries a canvas measuring `system-ui` finds (the real project's
/// capture, `docs/engine/text.md`), so a seed named by one matches them all.
const SYSTEM_FONT_ALIASES: [&str; 3] = ["system-ui", "-apple-system", "blinkmacsystemfont"];

fn same_cache_family(a: &str, b: &str) -> bool {
    let system = |n: &str| {
        SYSTEM_FONT_ALIASES
            .iter()
            .any(|s| n.eq_ignore_ascii_case(s))
    };
    a.eq_ignore_ascii_case(b) || (system(a) && system(b))
}

/// Same default key as shared/engine-fonts-seeds.ts. Only identical
/// platform-data keys can share the first creator's optical size.
const CANVAS_FONT_CACHE_KEY: &str = r#"["auto","auto",true,true,"normal","100%","horizontal","normal","none",[],"normal","normal",true,false,false,false]"#;

impl FontCacheSeed {
    fn matches(&self, family: &str, desc: &FontDesc) -> bool {
        let canvas_key = if desc.italic {
            CANVAS_FONT_CACHE_KEY.replace("\"normal\",\"100%\"", "\"italic\",\"100%\"")
        } else {
            CANVAS_FONT_CACHE_KEY.to_string()
        };
        self.cache_key.as_deref().unwrap_or(&canvas_key) == canvas_key
            && same_cache_family(&self.family, family)
            && self.weight == desc.weight
            && self.italic == desc.italic
            && self.size == desc.size
    }
}

/// Resolves the family list into Blink's `FontFallbackList` order, appending
/// the standard font after the last family. `desc.size` is the effective
/// size; `specified` is the size before the floor.
pub fn build_setup(
    resolver: &mut dyn FontResolver,
    families: &[Family],
    desc: FontDesc,
    specified: f32,
    letter_spacing: f32,
    seeds: &[FontCacheSeed],
) -> FontSetup {
    let mut group = Vec::new();
    let mut seeded_opsz = Vec::new();
    let mut seed = |face: FaceId, family: &str| {
        if let Some(s) = seeds.iter().find(|s| s.matches(family, &desc))
            && !seeded_opsz.iter().any(|(f, _)| *f == face)
        {
            seeded_opsz.push((face, s.specified));
        }
    };
    for family in families {
        match family {
            Family::Named(name) => {
                if let Some(f) = resolver.family(name, &desc) {
                    // A project font is a web font (a segmented `FontFace`):
                    // it shapes with web-font coordinates and asks for the
                    // full hint list.
                    if resolver.face(f).custom {
                        group.push(GroupFont::Custom {
                            face: f,
                            settings: Vec::new(),
                        });
                    } else {
                        seed(f, name);
                        group.push(GroupFont::Installed(f));
                    }
                }
            }
            Family::Generic(g) => {
                let name = generic_family(g);
                if let Some(f) = resolver.family(name, &desc) {
                    seed(f, name);
                    group.push(GroupFont::Installed(f));
                }
            }
            Family::Custom { face, settings } => group.push(GroupFont::Custom {
                face: *face,
                settings: settings.clone(),
            }),
        }
    }
    if let Some(f) = resolver.standard(&desc) {
        group.push(GroupFont::Installed(f));
    }
    // PrimarySimpleFontDataWithSpace: the first font with a space glyph.
    let mut primary = None;
    for g in &group {
        let info = resolver.face(g.face());
        if let Ok(font) = read_fonts::FontRef::from_index(info.data.as_slice(), info.index) {
            use read_fonts::TableProvider;
            let has_space = font
                .cmap()
                .ok()
                .and_then(|c| c.map_codepoint(0x20u32))
                .is_some_and(|g| g.to_u32() != 0);
            if has_space {
                primary = Some(g.face());
                break;
            }
        }
    }
    if primary.is_none() {
        primary = group
            .first()
            .map(|g| g.face())
            .or_else(|| resolver.last_resort(&desc));
    }
    FontSetup {
        group,
        desc,
        specified,
        seeded_opsz,
        letter_spacing,
        primary,
    }
}

/// Result of measuring one string.
#[derive(Clone, Debug, Default)]
pub struct Measured {
    pub width: f32,
    pub items: Vec<ShapeResult>,
}

fn normalize(text: &[u16]) -> (Vec<u16>, bool) {
    let mut out = Vec::with_capacity(text.len());
    let mut changed = false;
    let mut i = 0;
    while i < text.len() {
        let (c, n) = next_cp(text, i);
        // Branches mirror Blink's `NormalizeSpacesAndMirrorChars` order.
        #[allow(clippy::if_same_then_else)]
        let normalized = if chars::is_normalized_canvas_space(c) {
            chars::SPACE
        } else if chars::is_treat_as_space(c) && c != chars::NO_BREAK_SPACE {
            chars::SPACE
        } else if chars::treat_as_zero_width_space_in_complex_script_legacy(c) {
            chars::ZERO_WIDTH_SPACE
        } else {
            c
        };
        if normalized != c {
            changed = true;
            out.push(normalized as u16);
        } else {
            out.extend_from_slice(&text[i..i + n]);
        }
        i += n;
    }
    (out, changed)
}

fn is_word_delimiter(ch: u32, split_by_zws: bool) -> bool {
    ch == chars::SPACE || ch == chars::TAB || (split_by_zws && ch == chars::ZERO_WIDTH_SPACE)
}

fn next_word_end(text: &[u16], start: usize, is_8bit: bool) -> usize {
    let len = text.len();
    if start >= len {
        return 0;
    }
    if start + 1 == len || is_word_delimiter(text[start] as u32, true) {
        return start + 1;
    }
    if is_8bit {
        let mut i = start + 1;
        loop {
            if i == len || is_word_delimiter(text[i] as u32, false) {
                return i;
            }
            i += 1;
        }
    }
    let (ch, n) = next_cp(text, start);
    let mut end = start + n;
    if !chars::is_cjk_ideograph_or_symbol(ch) {
        let mut next_end = end;
        while end < len {
            let (ch, n) = next_cp(text, next_end);
            next_end += n;
            if is_word_delimiter(ch, true) || chars::is_cjk_ideograph_or_symbol_base(ch) {
                return end;
            }
            end = next_end;
        }
        return len;
    }
    let mut has_any_script = !chars::is_common_or_inherited_script(ch);
    let mut next_end = end;
    while end < len {
        let (ch, n) = next_cp(text, next_end);
        next_end += n;
        if chars::is_mark_or_modifier(ch)
            || ch == chars::ZERO_WIDTH_JOINER
            || chars::is_emoji_component(ch)
            || chars::is_extended_pictographic(ch)
        {
            end = next_end;
            continue;
        }
        if chars::is_cjk_ideograph_or_symbol(ch) {
            if chars::is_common_or_inherited_script(ch) {
                end = next_end;
                continue;
            }
            if !has_any_script {
                has_any_script = true;
                end = next_end;
                continue;
            }
        }
        return end;
    }
    len
}

struct Item {
    start: usize,
    len: usize,
    rtl: bool,
}

fn bidi_runs(original: &[u16]) -> Vec<(usize, usize, bool)> {
    let maybe_bidi = {
        let mut i = 0;
        let mut found = false;
        while i < original.len() {
            let (c, n) = next_cp(original, i);
            if chars::maybe_bidi_rtl(c) {
                found = true;
                break;
            }
            i += n;
        }
        found
    };
    if !maybe_bidi {
        return vec![(0, original.len(), false)];
    }
    let info = unicode_bidi::utf16::BidiInfo::new(original, Some(unicode_bidi::Level::ltr()));
    let Some(para) = info.paragraphs.first() else {
        return vec![(0, original.len(), false)];
    };
    let levels = &info.levels;
    if levels.iter().all(|l| l.is_ltr()) {
        return vec![(0, original.len(), false)];
    }
    let (_, runs) = info.visual_runs(para, para.range.clone());
    runs.into_iter()
        .map(|r| (r.start, r.end - r.start, levels[r.start].is_rtl()))
        .collect()
}

fn is_cursive(script: Script) -> bool {
    matches!(
        script,
        Script::Arabic
            | Script::HanifiRohingya
            | Script::Mandaic
            | Script::Mongolian
            | Script::Nko
            | Script::PhagsPa
            | Script::Syriac
    )
}

fn fixed_to_f32(raw: i64) -> f32 {
    raw as f32 / 65536.0
}

fn apply_spacing(result: &mut ShapeResult, text: &[u16], item_start: usize, letter_spacing: i32) {
    let mut total = 0.0f32;
    for run in &mut result.runs {
        let run_start = run.start_index as usize + item_start;
        let cursive = is_cursive(run.script);
        let mut total_run: i64 = 0;
        let n = run.glyphs.len();
        for i in 0..n {
            if i + 1 < n && run.glyphs[i].character_index == run.glyphs[i + 1].character_index {
                total_run += run.glyphs[i].advance as i64;
                continue;
            }
            let index = run_start + run.glyphs[i].character_index as usize;
            let mut c = text[index] as u32;
            let treat_as_space =
                chars::is_treat_as_space(c) || chars::is_normalized_canvas_space(c);
            if treat_as_space && c != chars::NO_BREAK_SPACE {
                c = chars::SPACE;
            }
            let apply = !cursive || treat_as_space;
            let mut spacing = 0i32;
            if letter_spacing != 0 && !chars::treat_as_zero_width_space(c) && apply {
                spacing += letter_spacing;
            }
            run.glyphs[i].advance += spacing;
            total_run += run.glyphs[i].advance as i64;
        }
        run.width = fixed_to_f32(total_run);
        total += run.width;
    }
    result.width = total;
}

/// Measures `text` (UTF-16, as JavaScript holds it) like
/// `CanvasRenderingContext2D.measureText(text).width`.
/// `measure` remembered per font setup and text. A layout engine's face,
/// fallback and prepared-face answers only ever stick within a session, so
/// a repeated measurement equals the first (Blink keeps shaped words in the
/// font's `ShapeCache` the same way). Text drawn every frame is shaped once.
pub fn measure(
    resolver: &mut dyn FontResolver,
    cache: &mut PreparedCache,
    setup: &FontSetup,
    text: &[u16],
) -> Measured {
    let key = (setup_key(setup), text.to_vec());
    if let Some(measured) = cache.measured.get(&key) {
        return measured;
    }
    let measured = measure_uncached(resolver, cache, setup, text);
    cache.measured.insert(key, measured.clone());
    measured
}

/// Every field of `setup`, as bits.
fn setup_key(setup: &FontSetup) -> Vec<u64> {
    let FontSetup {
        group,
        desc,
        specified,
        seeded_opsz,
        letter_spacing,
        primary,
    } = setup;
    let FontDesc {
        weight,
        italic,
        size,
    } = desc;
    let mut key = vec![
        u64::from(weight.to_bits()),
        u64::from(*italic),
        u64::from(size.to_bits()),
        u64::from(specified.to_bits()),
        u64::from(letter_spacing.to_bits()),
        primary.map_or(u64::MAX, u64::from),
        group.len() as u64,
    ];
    for font in group {
        match font {
            GroupFont::Installed(face) => key.extend([0, u64::from(*face)]),
            GroupFont::Custom { face, settings } => {
                key.extend([1, u64::from(*face), settings.len() as u64]);
                key.extend(
                    settings
                        .iter()
                        .map(|(tag, value)| (u64::from(*tag) << 32) | u64::from(value.to_bits())),
                );
            }
        }
    }
    key.extend(
        seeded_opsz
            .iter()
            .map(|(face, size)| (u64::from(*face) << 32) | u64::from(size.to_bits())),
    );
    key
}

/// A measurement's font setup bits and text.
type MeasureKey = (Vec<u64>, Vec<u16>);

/// Bounded LRU of measurements (see [`measure`]).
#[derive(Default)]
pub struct MeasureMemo {
    /// The measurement, when it was last used, and its retained bytes.
    entries: std::collections::HashMap<MeasureKey, (Measured, u64, usize)>,
    order: std::collections::BTreeMap<u64, MeasureKey>,
    tick: u64,
    bytes: usize,
}

/// Measurements kept per session.
const MEASURE_MEMO_BYTES: usize = 8 * 1024 * 1024;

impl MeasureMemo {
    fn get(&mut self, key: &MeasureKey) -> Option<Measured> {
        let (measured, used, _) = self.entries.get_mut(key)?;
        self.tick += 1;
        let key = self.order.remove(used).expect("ordered entry");
        *used = self.tick;
        let measured = measured.clone();
        self.order.insert(self.tick, key);
        Some(measured)
    }

    fn insert(&mut self, key: MeasureKey, measured: Measured) {
        let bytes = (key.0.len() * 8 + key.1.len() * 2) * 2
            + measured
                .items
                .iter()
                .flat_map(|item| &item.runs)
                .map(|run| {
                    std::mem::size_of_val(run)
                        + run.glyphs.len() * std::mem::size_of::<super::shaper::GlyphData>()
                        + run.coords.len() * 8
                })
                .sum::<usize>()
            + std::mem::size_of::<Measured>();
        if bytes > MEASURE_MEMO_BYTES {
            return;
        }
        while self.bytes + bytes > MEASURE_MEMO_BYTES {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some((_, _, freed)) = self.entries.remove(&oldest) {
                self.bytes -= freed;
            }
        }
        self.tick += 1;
        self.order.insert(self.tick, key.clone());
        self.entries.insert(key, (measured, self.tick, bytes));
        self.bytes += bytes;
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

fn measure_uncached(
    resolver: &mut dyn FontResolver,
    cache: &mut PreparedCache,
    setup: &FontSetup,
    text: &[u16],
) -> Measured {
    let is_8bit_source = text.iter().all(|&u| u <= 0xFF);
    let (content, changed) = normalize(text);
    let is_8bit = is_8bit_source && !changed;
    let mut items: Vec<Item> = Vec::new();
    for (start, len, rtl) in bidi_runs(text) {
        let insertion = items.len();
        let sub = &content[start..start + len];
        let mut index = 0;
        while index < len {
            let end = next_word_end(sub, index, is_8bit);
            let item = Item {
                start: start + index,
                len: end - index,
                rtl,
            };
            if rtl {
                items.insert(insertion, item);
            } else {
                items.push(item);
            }
            index = end;
        }
    }
    let letter_spacing = (setup.letter_spacing * 65536.0) as i32;
    let mut out = Measured::default();
    let mut shaper = Shaper::new(resolver, cache, setup);
    for item in items {
        let item_text = &content[item.start..item.start + item.len];
        let mut result = shaper.shape(item_text, is_8bit, item.rtl);
        if letter_spacing != 0 {
            apply_spacing(&mut result, &content, item.start, letter_spacing);
        }
        out.width += result.width;
        out.items.push(result);
    }
    out
}

#[cfg(test)]
mod seed_tests {
    use super::*;

    #[test]
    fn app_ui_seeds_follow_the_windows_scale_factors() {
        let two = app_ui_font_cache_seeds(&[2.0]);
        assert_eq!(
            two.iter()
                .map(|s| (s.size, s.specified))
                .collect::<Vec<_>>(),
            [(24.0, 12.0), (26.0, 13.0)]
        );
        assert!(
            two.iter()
                .all(|s| s.family == "system-ui" && s.weight == 400.0 && !s.italic)
        );
        // At 1x the UI's entries are the canvas's own sizes.
        let one = app_ui_font_cache_seeds(&[1.0]);
        assert!(one.iter().all(|s| s.size == s.specified));
        // A window that went from 2x to 1x keeps both; the first creator of
        // a key wins, and invalid factors are ignored.
        let both = app_ui_font_cache_seeds(&[2.0, f32::NAN, 1.0, 2.0]);
        assert_eq!(both.len(), 4);
        assert_eq!(both[0].size, 24.0);
        // Fractional scale factors floor as Blink's effective size does.
        assert_eq!(app_ui_font_cache_seeds(&[1.1])[0].size, 13.2);
        assert_eq!(app_ui_font_cache_seeds(&[]).len(), 0);
    }

    #[test]
    fn system_font_aliases_share_a_cache_key() {
        assert!(same_cache_family("system-ui", "-apple-system"));
        assert!(same_cache_family("BlinkMacSystemFont", "system-ui"));
        assert!(same_cache_family("Helvetica", "helvetica"));
        assert!(!same_cache_family("system-ui", "Helvetica"));
    }
}
