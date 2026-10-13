//! `HarfBuzzShaper` and `FontFallbackIterator` ported from Blink (Chromium
//! 152), shaping with HarfRust at Blink's 16.16 fixed-point scale.

use super::chars;
use super::segment::{self, FallbackPriority, Segment};
use crate::font::{FaceId, FontDesc, FontResolver, PreparedCache, PreparedFace};
use harfrust::{Buffer, Direction, Feature, FontFuncs, GlyphId, Language, ShapeOptions, ShaperFont, Tag};
use icu_properties::PropertyNamesShort;
use icu_properties::props::Script;
use std::cell::Cell;
use std::collections::{HashSet, VecDeque};
use std::str::FromStr;
use std::sync::Arc;

pub const UNMATCHED_VS_GLYPH: u32 = u32::MAX;

/// Glyph data as stored in `HarfBuzzRunGlyphData` (advance in 16.16).
#[derive(Clone, Copy, Debug)]
pub struct GlyphData {
    pub glyph: u32,
    pub character_index: u32,
    pub advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub face: FaceId,
    pub script: Script,
    pub rtl: bool,
    pub start_index: u32,
    pub num_characters: u32,
    pub glyphs: Vec<GlyphData>,
    pub width: f32,
    pub size: f32,
    /// The face's final variation coordinates (user space) at this size,
    /// for renderers that instance the same face.
    pub coords: Vec<(u32, f32)>,
}

#[derive(Clone, Debug, Default)]
pub struct ShapeResult {
    pub runs: Vec<Run>,
    pub width: f32,
}

/// One entry of Blink's `FontFallbackList`.
#[derive(Clone, Debug)]
pub enum GroupFont {
    Installed(FaceId),
    /// A web font (`FontFace`) with its variation settings descriptor.
    Custom { face: FaceId, settings: Vec<(u32, f32)> },
}

impl GroupFont {
    pub fn face(&self) -> FaceId {
        match self {
            GroupFont::Installed(f) => *f,
            GroupFont::Custom { face, .. } => *face,
        }
    }
}

/// Everything a shaping call needs about the font: the resolved family list
/// (with `-webkit-standard` appended, as `FontFallbackList::GetFontData` does),
/// the description, and features derived from letter spacing.
pub struct FontSetup {
    pub group: Vec<GroupFont>,
    /// The description at Blink's effective size (`desc.size`), which fonts
    /// are created, scaled, and measured at.
    pub desc: FontDesc,
    /// `FontDescription::SpecifiedSize()`: the size as given, before the
    /// effective-size floor. HarfBuzz looks `trak` up at it
    /// (`HarfBuzzFace::GetScaledFont` sets `ptem` to it) and
    /// `FontPlatformDataFromCTFont` sets `opsz` to it (font-optical-sizing:
    /// auto).
    pub specified: f32,
    /// Installed group fonts whose platform data Blink's font cache already
    /// held, created at another specified size (see
    /// [`crate::blink::measure::FontCacheSeed`]): `(face, specified size)`.
    pub seeded_opsz: Vec<(FaceId, f32)>,
    pub letter_spacing: f32,
    pub primary: Option<FaceId>,
}

impl FontSetup {
    /// The size `opsz` follows for an installed face: the specified size of
    /// whoever created its platform data.
    pub fn optical_size(&self, face: FaceId) -> f32 {
        self.seeded_opsz.iter().find(|(f, _)| *f == face).map_or(self.specified, |(_, s)| *s)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Intermediate,
    Last,
    IntermediateWithVs,
    LastWithVs,
    IntermediateIgnoreVs,
    LastIgnoreVs,
}

fn is_last(s: Stage) -> bool {
    matches!(s, Stage::Last | Stage::LastIgnoreVs)
}
fn needs_reset(s: Stage) -> bool {
    s == Stage::LastWithVs
}
fn to_last(s: Stage) -> Stage {
    match s {
        Stage::Intermediate => Stage::Last,
        Stage::IntermediateWithVs => Stage::LastWithVs,
        Stage::IntermediateIgnoreVs => Stage::LastIgnoreVs,
        other => other,
    }
}
fn to_vs(s: Stage) -> Stage {
    match s {
        Stage::Intermediate => Stage::IntermediateWithVs,
        Stage::Last => Stage::LastWithVs,
        other => other,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IterStage {
    FallbackPriorityFonts,
    FontGroupFonts,
    SystemFonts,
    FirstCandidateForNotdef,
    OutOfLuck,
}

struct FallbackIter {
    stage: IterStage,
    index: usize,
    priority: FallbackPriority,
    previously_asked: HashSet<u32>,
    unique_returned: HashSet<String>,
    first_candidate: Option<usize>,
}

/// A font returned by the iterator: index into `Shaper::fonts`.
struct IterFont(usize);

#[derive(Clone, Copy, PartialEq, Eq)]
enum VsMode {
    UseSpecified,
    Ignore,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QAction {
    NextFont,
    Range,
    Reset,
}

#[derive(Clone, Copy)]
struct QItem {
    action: QAction,
    start: u32,
    num: u32,
}

struct Slice {
    start_char: u32,
    num_chars: u32,
    start_glyph: usize,
    num_glyphs: usize,
}

/// Blink's glyph callbacks (`HarfBuzzGetGlyph`, Skia advances).
struct BlinkFuncs<'p> {
    face: &'p PreparedFace,
    vs_mode: Cell<VsMode>,
    skia_advances: bool,
}

impl BlinkFuncs<'_> {
    fn base_glyph(&self, font: &ShaperFont, unicode: u32) -> Option<GlyphId> {
        font.default_nominal_glyph(unicode)
    }

    fn get_glyph(&self, font: &ShaperFont, mut unicode: u32, mut vs: u32) -> Option<GlyphId> {
        if unicode == chars::LINE_SEPARATOR || unicode == chars::PARAGRAPH_SEPARATOR {
            unicode = chars::SPACE;
        }
        let mut consider_vs = false;
        if self.vs_mode.get() != VsMode::Ignore && chars::is_unicode_variation_selector(vs) && chars::is_variation_sequence(unicode, vs) {
            consider_vs = true;
        }
        let mut text_requested = false;
        let mut emoji_requested = false;
        if consider_vs {
            text_requested = vs == chars::VS15;
            emoji_requested = vs == chars::VS16;
            if let Some(g) = font.default_variation_glyph(unicode, vs) {
                return Some(g);
            }
            vs = 0;
        }
        let _ = vs;
        let base = self.base_glyph(font, unicode);
        if consider_vs && base.is_some() {
            if text_requested || emoji_requested {
                let color = self.face.has_color_table;
                if (color && text_requested) || (!color && emoji_requested) {
                    return Some(GlyphId::new(UNMATCHED_VS_GLYPH));
                }
            } else {
                return Some(GlyphId::new(UNMATCHED_VS_GLYPH));
            }
        }
        if base.is_none() {
            if unicode == chars::HYPHEN {
                return self.face.synthesized_hyphens[0].map(|g| GlyphId::new(g as u32));
            }
            if unicode == chars::NON_BREAKING_HYPHEN {
                return self.face.synthesized_hyphens[1].map(|g| GlyphId::new(g as u32));
            }
        }
        base
    }

    /// CoreText advance at the face's size as Skia reports it, in 16.16.
    fn skia_advance(&self, glyph: GlyphId) -> i32 {
        if glyph.to_u32() == UNMATCHED_VS_GLYPH {
            return 0;
        }
        let metrics = self.face.font.glyph_metrics();
        let mut units = metrics.h_advance(glyph);
        if self.face.phantom_advances {
            // CoreText moves the phantom points by the rounded `gvar` delta.
            units = units.round();
        }
        let px = match self.face.em_advance {
            // CoreText's strike-sized color glyph advance, proportional to hmtx.
            Some(em) => (units as f64 * em as f64 / self.face.upem as f64) as f32,
            None => (units as f64 * self.face.size as f64 / self.face.upem as f64) as f32,
        };
        (px * 65536.0f32) as i32
    }
}

impl BlinkFuncs<'_> {
    /// HarfBuzz's `hb_ot` advance as Blink gets it. The advance comes in
    /// font units from the parent `hb_ot` font (`HarfBuzzFontData`'s
    /// `ot_font`, unscaled, so its scale is the upem): the `hmtx` advance plus
    /// the variation delta rounded to a whole unit
    /// (`hmtx_accelerator_t::get_advance_with_var_unscaled`; HarfRust keeps
    /// the fractional delta instead). The scaled sub-font Blink shapes with
    /// has no advance function of its own, so HarfBuzz converts the parent's
    /// value with `parent_scale_x_distance`, `v * x_scale / parent_x_scale`
    /// in `int64_t`: a truncating division, not `em_mult`'s rounding. At
    /// fractional sizes that is one 16.16 unit less about half the time.
    fn harfbuzz_advance(&self, glyph: GlyphId) -> i32 {
        let units = match &self.face.default_font {
            Some(default) => {
                let base = default.glyph_metrics().h_advance(glyph) as f64;
                let varied = self.face.font.glyph_metrics().h_advance_exact(glyph).to_f64();
                let delta = (varied - base) as f32;
                (base as f32 + delta.round()).max(0.0) as i64
            }
            None => self.face.font.glyph_metrics().h_advance(glyph).max(0.0) as i64,
        };
        let scale = i64::from((self.face.size * 65536.0f32) as i32);
        (units * scale / i64::from(self.face.upem)) as i32
    }
}

impl FontFuncs for BlinkFuncs<'_> {
    fn nominal_glyph(&self, font: &ShaperFont, c: u32) -> Option<GlyphId> {
        self.get_glyph(font, c, 0)
    }
    fn variation_glyph(&self, font: &ShaperFont, c: u32, vs: u32) -> Option<GlyphId> {
        self.get_glyph(font, c, vs)
    }
    fn glyph_h_advance(&self, _font: &ShaperFont, glyph: GlyphId) -> i32 {
        if self.skia_advances {
            self.skia_advance(glyph)
        } else if glyph.to_u32() == UNMATCHED_VS_GLYPH {
            0
        } else {
            self.harfbuzz_advance(glyph)
        }
    }
}

pub fn hb_script(script: Script) -> Option<harfrust::Script> {
    let name = PropertyNamesShort::<Script>::new().get_locale_script(script)?;
    let bytes = name.as_str().as_bytes();
    if bytes.len() != 4 {
        return None;
    }
    harfrust::Script::from_iso15924_tag(Tag::new(&[bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub struct Shaper<'r> {
    pub resolver: &'r mut dyn FontResolver,
    pub cache: &'r mut PreparedCache,
    pub setup: &'r FontSetup,
    /// Fonts returned by fallback during this shaping call (index → face, custom settings).
    fonts: Vec<GroupFont>,
}

impl<'r> Shaper<'r> {
    pub fn new(resolver: &'r mut dyn FontResolver, cache: &'r mut PreparedCache, setup: &'r FontSetup) -> Self {
        Shaper { resolver, cache, setup, fonts: Vec::new() }
    }

    fn prepared(&mut self, font: &GroupFont) -> Option<Arc<PreparedFace>> {
        let size = self.setup.desc.size;
        let info = self.resolver.face(font.face()).clone();
        let coords = match font {
            GroupFont::Installed(f) => crate::font::platform_coords(&info, self.setup.optical_size(*f)),
            GroupFont::Custom { settings, .. } => crate::font::custom_coords(&info, &self.setup.desc, self.setup.specified, settings),
        };
        let face = font.face();
        let resolver = &mut *self.resolver;
        self.cache.get(face, &info, coords, size, || resolver.em_advance(face, size))
    }

    fn push_font(&mut self, f: GroupFont) -> usize {
        self.fonts.push(f);
        self.fonts.len() - 1
    }

    fn unique_or_next(&mut self, it: &mut FallbackIter, candidate: GroupFont, hint: &[u32]) -> Option<IterFont> {
        let key = self.resolver.face(candidate.face()).typeface_key.clone();
        let key = match &candidate {
            GroupFont::Custom { settings, .. } => format!("{key}|{settings:?}"),
            _ => key,
        };
        if it.unique_returned.contains(&key) {
            return self.iter_next(it, hint);
        }
        it.unique_returned.insert(key);
        let idx = self.push_font(candidate);
        if it.first_candidate.is_none() {
            it.first_candidate = Some(idx);
        }
        Some(IterFont(idx))
    }

    fn iter_next(&mut self, it: &mut FallbackIter, hint: &[u32]) -> Option<IterFont> {
        let desc = self.setup.desc;
        match it.stage {
            IterStage::OutOfLuck => None,
            IterStage::FallbackPriorityFonts => {
                it.stage = IterStage::SystemFonts;
                let ch = hint[0];
                let found = if chars::is_private_use(ch) || chars::is_non_character(ch) {
                    None
                } else {
                    let primary = self.setup.primary?;
                    self.resolver.fallback_for_char(&desc, ch, primary, it.priority.is_emoji_presentation())
                };
                match found {
                    Some(f) => self.unique_or_next(it, GroupFont::Installed(f), hint),
                    None => self.iter_next(it, hint),
                }
            }
            IterStage::SystemFonts => {
                let found = self.unique_system_font(it, hint);
                if let Some(f) = found {
                    return self.unique_or_next(it, GroupFont::Installed(f), hint);
                }
                it.stage = IterStage::FirstCandidateForNotdef;
                match self.resolver.last_resort(&desc) {
                    Some(f) => self.unique_or_next(it, GroupFont::Installed(f), hint),
                    None => self.iter_next(it, hint),
                }
            }
            IterStage::FirstCandidateForNotdef => {
                it.stage = IterStage::OutOfLuck;
                let idx = it.first_candidate?;
                let f = self.fonts[idx].clone();
                Some(IterFont(self.push_font(f)))
            }
            IterStage::FontGroupFonts => {
                let Some(font) = self.setup.group.get(it.index).cloned() else {
                    it.stage = if it.priority.is_non_text() { IterStage::FallbackPriorityFonts } else { IterStage::SystemFonts };
                    return self.iter_next(it, hint);
                };
                it.index += 1;
                self.unique_or_next(it, font, hint)
            }
        }
    }

    fn unique_system_font(&mut self, it: &mut FallbackIter, hint: &[u32]) -> Option<FaceId> {
        if hint.is_empty() {
            return None;
        }
        let mut idx = 0;
        if hint.len() > 1
            && let Some(i) = (1..hint.len()).find(|&i| chars::has_likely_script(hint[i])) {
                idx = i;
            }
        let ch = hint[idx];
        if ch == 0 || it.previously_asked.contains(&ch) {
            return None;
        }
        it.previously_asked.insert(ch);
        if chars::is_private_use(ch) || chars::is_non_character(ch) {
            return None;
        }
        let primary = self.setup.primary?;
        self.resolver.fallback_for_char(&self.setup.desc, ch, primary, false)
    }

    fn needs_hint_list(&self, it: &FallbackIter) -> bool {
        it.stage == IterStage::FontGroupFonts && matches!(self.setup.group.get(it.index), Some(GroupFont::Custom { .. }))
    }

    /// `HarfBuzzShaper::Shape(font, direction)` for a whole item.
    pub fn shape(&mut self, text: &[u16], is_8bit: bool, rtl: bool) -> ShapeResult {
        let mut result = ShapeResult::default();
        if text.is_empty() {
            return result;
        }
        if is_8bit {
            let seg = Segment { start: 0, end: text.len(), script: Script::Latin, priority: FallbackPriority::Text };
            self.shape_segment(text, seg, rtl, &mut result);
        } else {
            for seg in segment::run_segments(text) {
                self.shape_segment(text, seg, rtl, &mut result);
            }
        }
        result
    }

    fn collect_hint(&self, text: &[u16], queue: &VecDeque<QItem>, needs_list: bool) -> Option<Vec<u32>> {
        if queue.is_empty() {
            return None;
        }
        let mut hint = Vec::new();
        for item in queue {
            if item.action == QAction::NextFont {
                break;
            }
            let s = &text[item.start as usize..(item.start + item.num) as usize];
            let mut i = 0;
            while i < s.len() {
                let (c, n) = segment::next_cp(s, i);
                // UTF16TextIterator yields U+FFFD for an unpaired surrogate.
                let c = if (0xD800..0xE000).contains(&c) { 0xFFFD } else { c };
                hint.push(c);
                if !needs_list && !chars::is_common_or_inherited_script(c) {
                    return Some(hint);
                }
                i += n;
            }
        }
        if hint.is_empty() { None } else { Some(hint) }
    }

    fn shape_segment(&mut self, text: &[u16], seg: Segment, rtl: bool, result: &mut ShapeResult) {
        let mut it = FallbackIter {
            stage: IterStage::FontGroupFonts,
            index: 0,
            priority: seg.priority,
            previously_asked: HashSet::new(),
            unique_returned: HashSet::new(),
            first_candidate: None,
        };
        let mut queue: VecDeque<QItem> = VecDeque::new();
        queue.push_back(QItem { action: QAction::NextFont, start: 0, num: 0 });
        queue.push_back(QItem { action: QAction::Range, start: seg.start as u32, num: (seg.end - seg.start) as u32 });
        let mut font_cycle_queued = false;
        let mut current: Option<usize> = None;
        let mut stage = Stage::Intermediate;
        let mut vs_mode = VsMode::UseSpecified;
        while let Some(item) = queue.pop_front() {
            if item.action != QAction::Range {
                if item.action == QAction::Reset {
                    it.stage = IterStage::FontGroupFonts;
                    it.index = 0;
                    it.previously_asked.clear();
                    it.unique_returned.clear();
                    it.first_candidate = None;
                    stage = Stage::IntermediateIgnoreVs;
                    vs_mode = VsMode::Ignore;
                }
                let needs = self.needs_hint_list(&it);
                let Some(hint) = self.collect_hint(text, &queue, needs) else {
                    queue.clear();
                    break;
                };
                match self.iter_next(&mut it, &hint) {
                    Some(IterFont(idx)) => current = Some(idx),
                    None => break,
                }
                font_cycle_queued = false;
                continue;
            }
            if it.stage == IterStage::OutOfLuck {
                stage = to_last(stage);
            }
            let Some(font_idx) = current else { break };
            let font = self.fonts[font_idx].clone();
            let Some(prepared) = self.prepared(&font) else {
                // A face that cannot be parsed shapes nothing; treat as all-notdef.
                if !is_last(stage) {
                    if !font_cycle_queued {
                        queue.push_back(QItem { action: if needs_reset(stage) { QAction::Reset } else { QAction::NextFont }, start: 0, num: 0 });
                        font_cycle_queued = true;
                    }
                    queue.push_back(item);
                }
                continue;
            };
            let buffer = self.shape_range(text, item, &prepared, seg.script, rtl, vs_mode);
            self.extract(
                text,
                &buffer,
                &mut queue,
                &mut font_cycle_queued,
                item,
                &font,
                &prepared,
                seg.script,
                rtl,
                &mut stage,
                result,
            );
        }
    }

    fn shape_range(&self, text: &[u16], item: QItem, face: &PreparedFace, script: Script, rtl: bool, vs_mode: VsMode) -> Buffer {
        let funcs = BlinkFuncs { face, vs_mode: Cell::new(vs_mode), skia_advances: !face.has_trak || face.has_sbix };
        let scale = (face.size * 65536.0f32) as i32;
        let shaper_font = ShaperFont::new(&face.font).with_scale(scale).with_font_funcs(Some(&funcs));
        let mut buffer = Buffer::new();
        let start = item.start as usize;
        let end = (item.start + item.num) as usize;
        let decode = |s: &[u16]| -> Vec<(u32, usize)> {
            let mut out = Vec::new();
            let mut i = 0;
            while i < s.len() {
                let (c, n) = segment::next_cp(s, i);
                let c = if (0xD800..0xE000).contains(&c) { 0xFFFD } else { c };
                out.push((c, i));
                i += n;
            }
            out
        };
        let pre: Vec<u32> = decode(&text[..start]).into_iter().map(|(c, _)| c).collect();
        let post: Vec<u32> = decode(&text[end..]).into_iter().map(|(c, _)| c).collect();
        let pre_tail: Vec<u32> = pre.iter().rev().take(5).rev().copied().collect();
        buffer.set_pre_context_codepoints(&pre_tail);
        for (c, i) in decode(&text[start..end]) {
            buffer.push(c, (start + i) as u32);
        }
        let post_head: Vec<u32> = post.iter().take(5).copied().collect();
        buffer.set_post_context_codepoints(&post_head);
        buffer.set_direction(if rtl { Direction::RightToLeft } else { Direction::LeftToRight });
        buffer.set_script(hb_script(script));
        buffer.set_language(Language::from_str("en-us").ok());
        let mut features = Vec::new();
        if self.setup.letter_spacing != 0.0 {
            for t in [b"liga", b"clig", b"calt"] {
                features.push(Feature::new(Tag::new(t), 0, ..));
            }
        }
        features.push(Feature::new(Tag::new(b"chws"), 1, ..));
        let options = ShapeOptions::new().features(&features).point_size(Some(self.setup.specified));
        harfrust::shape(&shaper_font, &mut buffer, options).expect("direction is set");
        buffer
    }

    #[allow(clippy::too_many_arguments)]
    fn extract(
        &mut self,
        text: &[u16],
        buf: &Buffer,
        queue: &mut VecDeque<QItem>,
        font_cycle_queued: &mut bool,
        item: QItem,
        font: &GroupFont,
        face: &PreparedFace,
        script: Script,
        rtl: bool,
        stage: &mut Stage,
        result: &mut ShapeResult,
    ) {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Cr {
            Shaped,
            NotDef,
            Unknown,
        }
        let infos = buf.glyph_infos();
        let num_glyphs = infos.len();
        if num_glyphs == 0 {
            return;
        }
        let mut current_result = Cr::Unknown;
        let mut previous_result = Cr::Unknown;
        let mut current_cluster = 0u32;
        let mut last_change = 0usize;
        let mut prev_cluster_start = 0usize;
        let space_glyph = face.space_glyph;
        let forward = !rtl;
        let range_end = item.start + item.num;

        let compute_slice = |old: usize, new: usize| -> Slice {
            let mut s = Slice { start_char: 0, num_chars: 0, start_glyph: old, num_glyphs: new - old };
            if forward {
                s.start_char = infos[old].cluster;
                if new == num_glyphs {
                    s.num_chars = range_end - s.start_char;
                } else {
                    s.num_chars = infos[new].cluster - s.start_char;
                }
            } else {
                s.start_char = infos[new - 1].cluster;
                if old == 0 {
                    s.num_chars = range_end - s.start_char;
                } else {
                    s.num_chars = infos[old - 1].cluster - infos[new - 1].cluster;
                }
            }
            s
        };

        let queue_chars = |queue: &mut VecDeque<QItem>, slice: &Slice, stage: Stage, queued: &mut bool| {
            if !*queued {
                queue.push_back(QItem { action: if needs_reset(stage) { QAction::Reset } else { QAction::NextFont }, start: 0, num: 0 });
                *queued = true;
            }
            queue.push_back(QItem { action: QAction::Range, start: slice.start_char, num: slice.num_chars });
        };

        let commit = |result: &mut ShapeResult, slice: &Slice| {
            let positions = buf.glyph_positions();
            let mut glyphs = Vec::with_capacity(slice.num_glyphs);
            let mut total: i64 = 0;
            for i in slice.start_glyph..slice.start_glyph + slice.num_glyphs {
                let gi = &infos[i];
                let gp = &positions[i];
                glyphs.push(GlyphData {
                    glyph: gi.glyph_id,
                    character_index: gi.cluster - slice.start_char,
                    advance: gp.x_advance,
                    x_offset: gp.x_offset,
                    y_offset: gp.y_offset,
                });
                total += gp.x_advance as i64;
            }
            let width = (total.max(0) as f64 / 65536.0) as f32;
            result.width += width;
            result.runs.push(Run {
                face: font.face(),
                script,
                rtl,
                start_index: slice.start_char,
                num_characters: slice.num_chars,
                glyphs,
                width,
                size: face.size,
                coords: face.coords.clone(),
            });
        };

        for (glyph_index, info) in infos.iter().enumerate() {
            let previous_cluster = current_cluster;
            current_cluster = info.cluster;
            let gid = info.glyph_id;
            // Branches mirror Blink's `ExtractShapeResults` checks one by one.
            #[allow(clippy::if_same_then_else)]
            let glyph_result = if gid == 0 {
                Cr::NotDef
            } else if Some(gid) == space_glyph && !is_last(*stage) && text[current_cluster as usize] as u32 == chars::IDEOGRAPHIC_SPACE {
                Cr::NotDef
            } else if gid == UNMATCHED_VS_GLYPH {
                *stage = to_vs(*stage);
                Cr::NotDef
            } else {
                Cr::Shaped
            };
            if current_cluster != previous_cluster {
                if previous_result != current_result && previous_result != Cr::Unknown {
                    let slice = compute_slice(last_change, prev_cluster_start);
                    if current_result == Cr::Shaped && !is_last(*stage) {
                        queue_chars(queue, &slice, *stage, font_cycle_queued);
                    } else {
                        commit(result, &slice);
                    }
                    last_change = prev_cluster_start;
                }
                previous_result = current_result;
                prev_cluster_start = glyph_index;
                current_result = glyph_result;
            } else {
                current_result = if glyph_result == Cr::Shaped && (current_result == Cr::Shaped || current_result == Cr::Unknown) {
                    Cr::Shaped
                } else {
                    Cr::NotDef
                };
            }
        }

        if current_result != previous_result && previous_result != Cr::Unknown && !is_last(*stage) {
            if current_result == Cr::Shaped {
                let slice = compute_slice(last_change, prev_cluster_start);
                queue_chars(queue, &slice, *stage, font_cycle_queued);
                let slice = compute_slice(prev_cluster_start, num_glyphs);
                commit(result, &slice);
            } else {
                let slice = compute_slice(last_change, prev_cluster_start);
                commit(result, &slice);
                let slice = compute_slice(prev_cluster_start, num_glyphs);
                queue_chars(queue, &slice, *stage, font_cycle_queued);
            }
        } else {
            let slice = compute_slice(last_change, num_glyphs);
            if current_result == Cr::NotDef && !is_last(*stage) {
                queue_chars(queue, &slice, *stage, font_cycle_queued);
            } else {
                commit(result, &slice);
            }
        }
    }
}
