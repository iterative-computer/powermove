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
impl Font {
    /// Parse the extension shorthand: style, weight, absolute/relative size,
    /// optional line-height, and comma-separated quoted/unquoted families.
    /// Relative values are resolved by the supplied element/root font sizes.
    pub fn parse(input: &str, em: f32, rem: f32) -> Result<Self> {
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
            if let Some(v) = size(t, em, rem) {
                if v <= 0. || !v.is_finite() {
                    return Err(Error::Syntax);
                }
                desc.size = v;
                size_at = Some(i);
                break;
            }
            match t.as_str() {
                "normal" => {}
                "italic" | "oblique" if !style => {
                    desc.italic = true;
                    style = true;
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
                                t.as_str(),
                                "small-caps"
                                    | "condensed"
                                    | "expanded"
                                    | "ultra-condensed"
                                    | "extra-condensed"
                                    | "semi-condensed"
                                    | "semi-expanded"
                                    | "extra-expanded"
                                    | "ultra-expanded"
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
            if t != "normal"
                && size(t, em, rem).is_none()
                && t.parse::<f32>().ok().filter(|n| *n >= 0.).is_none()
            {
                return Err(Error::Syntax);
            }
            i += 1;
        }
        let mut families = Vec::new();
        let mut words = Vec::new();
        for t in tokens[i..].iter().chain(std::iter::once(&",".to_string())) {
            if t == "," {
                if words.is_empty() {
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
                    match name.as_str() {
                        "serif" => Family::Generic("serif"),
                        "sans-serif" => Family::Generic("sans-serif"),
                        "monospace" => Family::Generic("monospace"),
                        "cursive" => Family::Generic("cursive"),
                        "fantasy" => Family::Generic("fantasy"),
                        _ => Family::Named(name),
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
fn size(s: &str, em: f32, rem: f32) -> Option<f32> {
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
#[derive(Debug, Clone)]
pub struct TextLayout {
    pub path: BezPath,
    pub width: f64,
    pub ascent: f64,
    pub descent: f64,
}
pub trait TextProvider {
    /// The host must resolve loaded project/system fonts before entering the worker.
    fn layout(&mut self, font: &Font, text: &[u16], direction: Direction) -> Result<TextLayout>;
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
    fn layout(&mut self, font: &Font, text: &[u16], direction: Direction) -> Result<TextLayout> {
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
        let bidi = unicode_bidi::utf16::BidiInfo::new(
            &text,
            Some(if direction == Direction::Rtl {
                unicode_bidi::Level::rtl()
            } else {
                unicode_bidi::Level::ltr()
            }),
        );
        let mut path = BezPath::new();
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
                                let m = Affine::translate((
                                    x + glyph.x_offset as f64 / 65536.,
                                    -(glyph.y_offset as f64) / 65536.,
                                ));
                                path.extend(outline.path.iter().map(|el| m * el));
                            }
                        }
                        x += glyph.advance as f64 / 65536.;
                    }
                }
            }
        }
        Ok(TextLayout {
            path,
            width: x,
            ascent,
            descent,
        })
    }
}
impl TextLayout {
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
            TextBaseline::Top => self.ascent,
            TextBaseline::Hanging => self.ascent * 0.8,
            TextBaseline::Middle => (self.ascent - self.descent) / 2.,
            TextBaseline::Alphabetic => 0.,
            TextBaseline::Ideographic | TextBaseline::Bottom => -self.descent,
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
        let b = if self.path.elements().is_empty() {
            Rect::ZERO
        } else {
            self.path.bounding_box()
        };
        TextMetrics {
            width: self.width,
            actual_bounding_box_left: x - b.x0,
            actual_bounding_box_right: b.x1 - x,
            actual_bounding_box_ascent: -b.y0 - y,
            actual_bounding_box_descent: b.y1 + y,
            font_bounding_box_ascent: self.ascent - y,
            font_bounding_box_descent: self.descent + y,
            em_height_ascent: self.ascent - y,
            em_height_descent: self.descent + y,
            hanging_baseline: 0.8 * self.ascent - y,
            alphabetic_baseline: -y,
            ideographic_baseline: -self.descent - y,
        }
    }
}
