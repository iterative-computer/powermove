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
    Custom { face: FaceId, settings: Vec<(u32, f32)> },
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
/// selection request (weight, width, slope), but not by specified size, and
/// `FontPlatformDataFromCTFont` sets `opsz` from the specified size of the
/// description that created the entry. Whoever creates an entry first fixes
/// its optical size for every later user of the key in that renderer. DOM
/// text is laid out at its CSS size times the zoom (on macOS, the device
/// scale factor applies as zoom), so UI text at 12 px on a 2x display
/// creates the 24 px entry with `opsz` 12 (clamped to the axis minimum, 17,
/// for the system font), and a canvas measuring 24 px text in the same
/// family, weight, and style afterwards gets SF's text-size advances (about
/// 3.7% wider than at `opsz` 24).
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
}

impl FontCacheSeed {
    fn matches(&self, family: &str, desc: &FontDesc) -> bool {
        self.family.eq_ignore_ascii_case(family) && self.weight == desc.weight && self.italic == desc.italic && self.size == desc.size
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
                        group.push(GroupFont::Custom { face: f, settings: Vec::new() });
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
            Family::Custom { face, settings } => group.push(GroupFont::Custom { face: *face, settings: settings.clone() }),
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
            let has_space = font.cmap().ok().and_then(|c| c.map_codepoint(0x20u32)).is_some_and(|g| g.to_u32() != 0);
            if has_space {
                primary = Some(g.face());
                break;
            }
        }
    }
    if primary.is_none() {
        primary = group.first().map(|g| g.face()).or_else(|| resolver.last_resort(&desc));
    }
    FontSetup { group, desc, specified, seeded_opsz, letter_spacing, primary }
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
        if chars::is_mark_or_modifier(ch) || ch == chars::ZERO_WIDTH_JOINER || chars::is_emoji_component(ch) || chars::is_extended_pictographic(ch) {
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
    let Some(para) = info.paragraphs.first() else { return vec![(0, original.len(), false)] };
    let levels = &info.levels;
    if levels.iter().all(|l| l.is_ltr()) {
        return vec![(0, original.len(), false)];
    }
    let (_, runs) = info.visual_runs(para, para.range.clone());
    runs.into_iter().map(|r| (r.start, r.end - r.start, levels[r.start].is_rtl())).collect()
}

fn is_cursive(script: Script) -> bool {
    matches!(
        script,
        Script::Arabic | Script::HanifiRohingya | Script::Mandaic | Script::Mongolian | Script::Nko | Script::PhagsPa | Script::Syriac
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
            let treat_as_space = chars::is_treat_as_space(c) || chars::is_normalized_canvas_space(c);
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
pub fn measure(resolver: &mut dyn FontResolver, cache: &mut PreparedCache, setup: &FontSetup, text: &[u16]) -> Measured {
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
            let item = Item { start: start + index, len: end - index, rtl };
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
