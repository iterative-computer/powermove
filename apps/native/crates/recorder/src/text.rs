use crate::{Error, Result};
use kurbo::{Affine, BezPath, Rect, Shape};
use pm_text::{
    blink::{
        measure::{self, Family},
        shaper::{Shaper, UNMATCHED_VS_GLYPH},
    },
    font::{FontDesc, FontResolver, PreparedCache},
    render::outline::{Coords, FaceRef, GlyphStyle, Outlines},
};
use read_fonts::TableProvider;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    End,
    Left,
    Right,
    Center,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextBaseline {
    Top,
    Hanging,
    Middle,
    #[default]
    Alphabetic,
    Ideographic,
    Bottom,
}
#[derive(Debug, Clone)]
pub struct Font {
    pub desc: FontDesc,
    pub families: Vec<Family>,
}
impl Default for Font {
    fn default() -> Self {
        Self {
            desc: FontDesc {
                weight: 400.,
                italic: false,
                size: 10.,
            },
            families: vec![Family::Generic("sans-serif")],
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct FontContext {
    pub element_size: f32,
    pub root_size: f32,
    pub weight: f32,
}
impl Default for FontContext {
    fn default() -> Self {
        Self {
            element_size: 16.,
            root_size: 16.,
            weight: 400.,
        }
    }
}
impl Font {
    /// Parse the extension shorthand: style, weight, absolute/relative size,
    /// optional line-height, and comma-separated quoted/unquoted families.
    /// Relative values are resolved by the supplied element/root font sizes.
    pub fn parse(input: &str, em: f32, rem: f32) -> Result<Self> {
        Self::parse_with_context(
            input,
            FontContext {
                element_size: em,
                root_size: rem,
                ..FontContext::default()
            },
        )
    }
    pub fn parse_with_context(input: &str, context: FontContext) -> Result<Self> {
        let (em, rem) = (context.element_size, context.root_size);
        if !em.is_finite()
            || !rem.is_finite()
            || em < 0.
            || rem < 0.
            || !(1. ..=1000.).contains(&context.weight)
        {
            return Err(Error::Syntax);
        }
        let tokens = tokenize(input)?;
        let mut desc = FontDesc {
            weight: 400.,
            italic: false,
            size: 10.,
        };
        let mut size_at = None;
        let mut weight = false;
        let mut style = false;
        for (i, t) in tokens.iter().enumerate() {
            let lower = t.to_ascii_lowercase();
            if let Some(v) = size(&lower, em, rem) {
                if i > 4 || v < 0. || !v.is_finite() {
                    return Err(Error::Syntax);
                }
                desc.size = v;
                size_at = Some(i);
                break;
            }
            match lower.as_str() {
                "normal" => {}
                "italic" | "oblique" if !style => {
                    desc.italic = true;
                    style = true;
                }
                "bolder" if !weight => {
                    desc.weight = if context.weight < 350. {
                        400.
                    } else if context.weight < 550. {
                        700.
                    } else {
                        900.
                    };
                    weight = true;
                }
                "lighter" if !weight => {
                    desc.weight = if context.weight < 550. {
                        100.
                    } else if context.weight < 750. {
                        400.
                    } else {
                        700.
                    };
                    weight = true;
                }
                "bold" if !weight => {
                    desc.weight = 700.;
                    weight = true;
                }
                _ => {
                    if let Ok(v) = t.parse::<f32>() {
                        if !weight && (1. ..=1000.).contains(&v) {
                            desc.weight = v;
                            weight = true;
                        } else {
                            return Err(Error::Syntax);
                        }
                    } else {
                        return Err(
                            if matches!(
                                lower.as_str(),
                                "small-caps"
                                    | "condensed"
                                    | "expanded"
                                    | "ultra-condensed"
                                    | "extra-condensed"
                                    | "semi-condensed"
                                    | "semi-expanded"
                                    | "extra-expanded"
                                    | "ultra-expanded"
                                    | "caption"
                                    | "icon"
                                    | "menu"
                                    | "message-box"
                                    | "small-caption"
                                    | "status-bar"
                                    | "xx-small"
                                    | "x-small"
                                    | "small"
                                    | "medium"
                                    | "large"
                                    | "x-large"
                                    | "xx-large"
                                    | "xxx-large"
                                    | "smaller"
                                    | "larger"
                            ) {
                                Error::NotSupported
                            } else {
                                Error::Syntax
                            },
                        );
                    }
                }
            }
        }
        let mut i = size_at.ok_or(Error::Syntax)? + 1;
        if tokens.get(i).map(String::as_str) == Some("/") {
            i += 1;
            let t = tokens.get(i).ok_or(Error::Syntax)?;
            let lower = t.to_ascii_lowercase();
            if lower != "normal"
                && !size(&lower, em, rem)
                    .or_else(|| lower.parse::<f32>().ok())
                    .is_some_and(|v| v.is_finite() && v >= 0.)
            {
                return Err(Error::Syntax);
            }

            i += 1;
        }
        let mut families = Vec::new();
        let mut words: Vec<String> = Vec::new();
        for t in tokens[i..].iter().chain(std::iter::once(&",".to_string())) {
            if t == "," {
                if words.is_empty() {
                    return Err(Error::Syntax);
                }
                let quoted_words = words
                    .iter()
                    .filter(|w| w.starts_with('"') || w.starts_with('\''))
                    .count();
                if quoted_words > 0 && words.len() != 1 {
                    return Err(Error::Syntax);
                }
                let s = words.join(" ");
                words.clear();
                let quoted = s.starts_with('"') || s.starts_with('\'');
                let name = if quoted {
                    s[1..s.len() - 1].to_string()
                } else {
                    s
                };
                if name.is_empty() {
                    return Err(Error::Syntax);
                }
                let family = if !quoted {
                    match name.to_ascii_lowercase().as_str() {
                        "serif" => Family::Generic("serif"),
                        "sans-serif" => Family::Generic("sans-serif"),
                        "monospace" => Family::Generic("monospace"),
                        "cursive" => Family::Generic("cursive"),
                        "fantasy" => Family::Generic("fantasy"),
                        "inherit" | "initial" | "unset" | "revert" | "revert-layer" => {
                            return Err(Error::Syntax);
                        }
                        _ => {
                            if !name.split_whitespace().all(valid_identifier) {
                                return Err(Error::Syntax);
                            }
                            Family::Named(name)
                        }
                    }
                } else {
                    Family::Named(name)
                };
                families.push(family);
            } else {
                words.push(t.clone());
            }
        }
        if families.is_empty() {
            return Err(Error::Syntax);
        }
        Ok(Self { desc, families })
    }
}
fn valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_' || first == '-' || !first.is_ascii())
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-' || !c.is_ascii())
}
fn size(s: &str, em: f32, rem: f32) -> Option<f32> {
    if matches!(s, "0" | "+0" | "-0") {
        return Some(0.);
    }
    for (unit, scale) in [
        ("rem", rem),
        ("px", 1.),
        ("pt", 96. / 72.),
        ("pc", 16.),
        ("in", 96.),
        ("cm", 96. / 2.54),
        ("mm", 96. / 25.4),
        ("q", 96. / 101.6),
        ("em", em),
        ("%", em / 100.),
    ] {
        if let Some(n) = s.strip_suffix(unit) {
            return n.parse::<f32>().ok().map(|n| n * scale);
        }
    }
    None
}
fn tokenize(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            word.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            word.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            if !word.is_empty() {
                return Err(Error::Syntax);
            }
            quote = Some(c);
            word.push(c);
        } else if c.is_ascii_whitespace() || c == ',' || c == '/' {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            if c == ',' || c == '/' {
                out.push(c.to_string());
            }
        } else {
            word.push(c);
        }
    }
    if quote.is_some() || escaped {
        return Err(Error::Syntax);
    }
    if !word.is_empty() {
        out.push(word);
    }
    Ok(out)
}
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextMetrics {
    pub width: f64,
    pub actual_bounding_box_left: f64,
    pub actual_bounding_box_right: f64,
    pub actual_bounding_box_ascent: f64,
    pub actual_bounding_box_descent: f64,
    pub font_bounding_box_ascent: f64,
    pub font_bounding_box_descent: f64,
    pub em_height_ascent: f64,
    pub em_height_descent: f64,
    pub hanging_baseline: f64,
    pub alphabetic_baseline: f64,
    pub ideographic_baseline: f64,
}
/// Font baseline positions in CSS pixels above the alphabetic baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontBaselines {
    pub em_over: f64,
    pub em_under: f64,
    pub hanging: f64,
    pub ideographic_under: f64,
}
impl FontBaselines {
    fn from_font(
        face: &read_fonts::FontRef<'_>,
        text: &[u16],
        size: f64,
        ascent: f64,
        descent: f64,
        scale: f64,
    ) -> Self {
        // CSS inline layout synthesizes a one-em box when a font has no baseline table.
        // Its ascent/descent proportions follow the first available font.
        let total = ascent + descent;
        let em_over = if total > 0. {
            size * ascent / total
        } else {
            size * 0.8
        };
        let mut out = Self {
            em_over,
            em_under: em_over - size,
            hanging: ascent * 0.8,
            ideographic_under: -descent,
        };
        let Some(baseline) = horizontal_baselines(face, text) else {
            return out;
        };
        let roman = baseline(*b"romn").unwrap_or(0.);
        if let Some(value) = baseline(*b"hang") {
            out.hanging = (value - roman) * scale;
        }
        if let Some(value) = baseline(*b"ideo") {
            out.ideographic_under = (value - roman) * scale;
            out.em_under = out.ideographic_under;
            out.em_over = out.em_under + size;
        }
        if let Some(value) = baseline(*b"idtp") {
            out.em_over = (value - roman) * scale;
            out.em_under = out.em_over - size;
        }
        out
    }
}
fn horizontal_baselines<'a>(
    face: &read_fonts::FontRef<'a>,
    text: &[u16],
) -> Option<impl Fn([u8; 4]) -> Option<f64> + use<'a>> {
    let axis = face.base().ok()?.horiz_axis()?.ok()?;
    let tags = axis.base_tag_list()?.ok()?;
    let scripts = axis.base_script_list().ok()?;
    let script = pm_text::blink::segment::run_segments(text)
        .first()
        .and_then(|s| pm_text::blink::shaper::hb_script(s.script))
        .map(|s| {
            let mut bytes = s.tag().to_be_bytes();
            bytes[0] = bytes[0].to_ascii_lowercase();
            bytes
        })
        .unwrap_or(*b"latn");
    let records = scripts.base_script_records();
    let record = records
        .iter()
        .find(|r| r.base_script_tag().to_be_bytes() == script)
        .or_else(|| {
            records
                .iter()
                .find(|r| r.base_script_tag().to_be_bytes() == *b"DFLT")
        })?;
    let values = record
        .base_script(scripts.offset_data())
        .ok()?
        .base_values()?
        .ok()?;
    Some(move |tag| {
        let index = tags
            .baseline_tags()
            .iter()
            .position(|t| t.get().to_be_bytes() == tag)?;
        Some(values.base_coords().get(index).ok()?.coordinate() as f64)
    })
}
#[derive(Debug, Clone)]
pub struct TextLayout {
    /// Each glyph's outline at its origin on the baseline.
    pub glyphs: Vec<(kurbo::Point, Arc<BezPath>)>,
    pub width: f64,
    pub ascent: f64,
    pub descent: f64,
    pub baselines: FontBaselines,
}
pub trait TextProvider {
    /// The host must resolve loaded project/system fonts before entering the worker.
    fn layout(&mut self, font: &Font, text: &[u16], direction: Direction) -> Result<TextLayout> {
        self.layout_expanded(font, text, direction, (0., 0.))
    }
    /// [`TextProvider::layout`] with each glyph's outline dilated by
    /// `expand` per side, x and y, in glyph pixels (round joins: today's
    /// font smoothing, `pm_text::render::smoothing`). Advances are unchanged.
    fn layout_expanded(
        &mut self,
        font: &Font,
        text: &[u16],
        direction: Direction,
        expand: (f64, f64),
    ) -> Result<TextLayout>;
}
/// Platform-independent pm-text adapter. R may be the native font broker or a Linux test resolver.
pub struct TextEngine<R> {
    pub resolver: R,
    prepared: PreparedCache,
    outlines: Outlines,
}
impl<R: FontResolver> TextEngine<R> {
    pub fn new(resolver: R) -> Self {
        Self {
            resolver,
            prepared: PreparedCache::default(),
            outlines: Outlines::new(),
        }
    }
}
impl<R: FontResolver> TextProvider for TextEngine<R> {
    fn layout_expanded(
        &mut self,
        font: &Font,
        text: &[u16],
        direction: Direction,
        expand: (f64, f64),
    ) -> Result<TextLayout> {
        // HTML normalizes ASCII whitespace to U+0020 and does not wrap lines.
        let text: Vec<u16> = text
            .iter()
            .map(|c| {
                if matches!(*c, 9 | 10 | 12 | 13) {
                    32
                } else {
                    *c
                }
            })
            .collect();
        let setup = measure::build_setup(
            &mut self.resolver,
            &font.families,
            font.desc,
            font.desc.size,
            0.,
            &[],
        );
        let primary = setup.primary.ok_or(Error::MissingFont)?;
        let info = self.resolver.face(primary);
        let face = read_fonts::FontRef::from_index(info.data.as_slice(), info.index)
            .map_err(|_| Error::MissingFont)?;
        let hhea = face.hhea().map_err(|_| Error::MissingFont)?;
        let upem = face.head().map_err(|_| Error::MissingFont)?.units_per_em() as f64;
        let scale = font.desc.size as f64 / upem;
        let (ascent, descent) = (
            hhea.ascender().to_i16() as f64 * scale,
            -(hhea.descender().to_i16() as f64) * scale,
        );
        let baselines =
            FontBaselines::from_font(&face, &text, font.desc.size as f64, ascent, descent, scale);
        let bidi = unicode_bidi::utf16::BidiInfo::new(
            &text,
            Some(if direction == Direction::Rtl {
                unicode_bidi::Level::rtl()
            } else {
                unicode_bidi::Level::ltr()
            }),
        );
        let mut glyphs = Vec::new();
        let mut x = 0.;
        for paragraph in &bidi.paragraphs {
            let (_, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
            for range in runs {
                let rtl = bidi.levels[range.start].is_rtl();
                let slice = &text[range];
                let shaped = Shaper::new(&mut self.resolver, &mut self.prepared, &setup).shape(
                    slice,
                    slice.iter().all(|c| *c <= 255),
                    rtl,
                );
                let runs: Vec<_> = if rtl {
                    shaped.runs.iter().rev().collect()
                } else {
                    shaped.runs.iter().collect()
                };
                for run in runs {
                    let info = self.resolver.face(run.face);
                    let style = GlyphStyle {
                        size: run.size,
                        synthetic_bold: if info.synthetic_bold {
                            let v = pm_text::render::outline::synthetic_bold_width(run.size as f64)
                                / 2.;
                            (v, v)
                        } else {
                            (0., 0.)
                        },
                        smooth: expand,
                        skew: if info.synthetic_italic {
                            pm_text::render::outline::SYNTHETIC_ITALIC_SKEW
                        } else {
                            0.
                        },
                        expand_tolerance: 1e-5,
                    };
                    for glyph in &run.glyphs {
                        if glyph.glyph != UNMATCHED_VS_GLYPH {
                            if pm_text::render::ink::drawn_color_glyph_ink(
                                FaceRef::of(info),
                                Coords::User(&run.coords),
                                glyph.glyph,
                                run.size,
                                style.skew,
                                None,
                            )
                            .is_some()
                            {
                                return Err(Error::NeedsPromotion("color glyph"));
                            }
                            if let Some(outline) = self.outlines.get(
                                FaceRef::of(info),
                                Coords::User(&run.coords),
                                glyph.glyph,
                                style,
                            ) {
                                glyphs.push((
                                    kurbo::Point::new(
                                        x + glyph.x_offset as f64 / 65536.,
                                        -(glyph.y_offset as f64) / 65536.,
                                    ),
                                    outline.path.clone(),
                                ));
                            }
                        }
                        x += glyph.advance as f64 / 65536.;
                    }
                }
            }
        }
        Ok(TextLayout {
            glyphs,
            width: x,
            ascent,
            descent,
            baselines,
        })
    }
}
impl TextLayout {
    /// The glyphs' outlines, each at its origin as `place` moves it.
    pub fn path(&self, mut place: impl FnMut(kurbo::Point) -> kurbo::Point) -> BezPath {
        let mut path = BezPath::new();
        for (origin, outline) in &self.glyphs {
            let m = Affine::translate(place(*origin).to_vec2());
            path.extend(outline.iter().map(|el| m * el));
        }
        path
    }
    pub(crate) fn anchor(
        &self,
        align: TextAlign,
        baseline: TextBaseline,
        direction: Direction,
    ) -> (f64, f64) {
        let x = match align {
            TextAlign::Left => 0.,
            TextAlign::Right => self.width,
            TextAlign::Center => self.width / 2.,
            TextAlign::Start => {
                if direction == Direction::Ltr {
                    0.
                } else {
                    self.width
                }
            }
            TextAlign::End => {
                if direction == Direction::Ltr {
                    self.width
                } else {
                    0.
                }
            }
        };
        let y = match baseline {
            TextBaseline::Top => self.baselines.em_over,
            TextBaseline::Hanging => self.baselines.hanging,
            TextBaseline::Middle => (self.baselines.em_over + self.baselines.em_under) / 2.,
            TextBaseline::Alphabetic => 0.,
            TextBaseline::Ideographic => self.baselines.ideographic_under,
            TextBaseline::Bottom => self.baselines.em_under,
        };
        (x, y)
    }
    pub(crate) fn metrics(
        &self,
        align: TextAlign,
        baseline: TextBaseline,
        direction: Direction,
    ) -> TextMetrics {
        let (x, y) = self.anchor(align, baseline, direction);
        let path = self.path(|at| at);
        let b = if path.elements().is_empty() {
            Rect::ZERO
        } else {
            path.bounding_box()
        };
        TextMetrics {
            width: self.width,
            actual_bounding_box_left: x - b.x0,
            actual_bounding_box_right: b.x1 - x,
            actual_bounding_box_ascent: -b.y0 - y,
            actual_bounding_box_descent: b.y1 + y,
            font_bounding_box_ascent: self.ascent - y,
            font_bounding_box_descent: self.descent + y,
            em_height_ascent: self.baselines.em_over - y,
            em_height_descent: -self.baselines.em_under + y,
            hanging_baseline: self.baselines.hanging - y,
            alphabetic_baseline: -y,
            ideographic_baseline: self.baselines.ideographic_under - y,
        }
    }
}
