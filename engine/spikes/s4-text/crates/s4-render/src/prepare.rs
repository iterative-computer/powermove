//! Turns scenes into what every renderer draws: positioned glyph runs from
//! the level 1 layout (the same Blink port the layout gate checks, on the
//! native CoreText registry) and absolute Bézier paths. Positions follow
//! Canvas2D `fillText`: lines from `textSourceLines`, `textAlign` from the
//! measured line width, baseline at y + lh × i + size × 0.82, pens advanced by
//! the shaped advances (letter spacing included).

use crate::scene::{Color, Item, Scene};
use kurbo::{Affine, BezPath};
use s4_layout::blink::measure;
use s4_layout::blink::shaper::UNMATCHED_VS_GLYPH;
use s4_layout::canvas::CanvasMeasurer;
use s4_layout::corpus;
use s4_layout::fallback_cache::FallbackCached;
use s4_layout::font::{FaceId, PreparedCache};
use s4_layout::legacy::{self, Align};
use s4_registry::CoreTextRegistry;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct PGlyph {
    pub id: u32,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug)]
pub struct GlyphRun {
    pub face: FaceId,
    pub size: f32,
    /// User-space variation coordinates (tag, value).
    pub coords: Vec<(u32, f32)>,
    pub glyphs: Vec<PGlyph>,
}

#[derive(Clone, Debug)]
pub enum Draw {
    Text { runs: Vec<GlyphRun>, color: Color, stroke: f64, stroke_color: Color },
    Path { path: BezPath, color: Color, even_odd: bool, stroke: f64, round_join: bool },
}

impl Draw {
    /// The same draw shifted horizontally (stress frames).
    pub fn shifted(&self, dx: f64) -> Draw {
        match self {
            Draw::Text { runs, color, stroke, stroke_color } => Draw::Text {
                runs: runs
                    .iter()
                    .map(|r| GlyphRun { glyphs: r.glyphs.iter().map(|g| PGlyph { x: g.x + dx, ..*g }).collect(), ..r.clone() })
                    .collect(),
                color: *color,
                stroke: *stroke,
                stroke_color: *stroke_color,
            },
            Draw::Path { path, color, even_odd, stroke, round_join } => {
                Draw::Path { path: Affine::translate((dx, 0.0)) * path.clone(), color: *color, even_odd: *even_odd, stroke: *stroke, round_join: *round_join }
            }
        }
    }
}

/// Font data for a face, as every renderer loads it.
#[derive(Clone)]
pub struct FaceData {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
    pub postscript_name: String,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    /// Bitmap glyphs (`sbix`, `CBDT`), e.g. Apple Color Emoji.
    pub bitmap: bool,
}

#[derive(Clone)]
pub struct Prepared {
    pub id: String,
    pub group: String,
    pub width: u32,
    pub height: u32,
    pub draws: Vec<Draw>,
    pub glyphs: usize,
    pub path_segments: usize,
}

impl Prepared {
    pub fn shifted(&self, dx: f64) -> Prepared {
        Prepared { draws: self.draws.iter().map(|d| d.shifted(dx)).collect(), ..self.clone() }
    }
}

pub struct Preparer {
    /// Behind Blink's fallback cache, as in the layout comparison (the
    /// Canvas2D page measures scenes in the same order).
    pub registry: FallbackCached<CoreTextRegistry>,
    cache: PreparedCache,
    pub faces: BTreeMap<FaceId, FaceData>,
}

impl Default for Preparer {
    fn default() -> Self {
        Self::new()
    }
}

impl Preparer {
    pub fn new() -> Self {
        Preparer { registry: FallbackCached::new(CoreTextRegistry::new()), cache: PreparedCache::default(), faces: BTreeMap::new() }
    }

    pub fn prepare(&mut self, scene: &Scene) -> Prepared {
        let mut draws = Vec::new();
        let (mut glyphs, mut path_segments) = (0, 0);
        for item in &scene.items {
            match item {
                Item::Text { d, x, y, color, stroke, stroke_color } => {
                    let runs = self.layout(d, *x, *y);
                    glyphs += runs.iter().map(|r| r.glyphs.len()).sum::<usize>();
                    draws.push(Draw::Text { runs, color: *color, stroke: *stroke, stroke_color: *stroke_color });
                }
                Item::Path { d, x, y, color, rule, stroke, join } => {
                    let path = Affine::translate((*x, *y)) * BezPath::from_svg(d).expect("scene path data");
                    path_segments += path.elements().len();
                    draws.push(Draw::Path { path, color: *color, even_odd: rule == "evenodd", stroke: *stroke, round_join: join != "miter" });
                }
            }
        }
        Prepared { id: scene.id.clone(), group: scene.group.clone(), width: scene.width, height: scene.height, draws, glyphs, path_segments }
    }

    fn layout(&mut self, d: &serde_json::Value, x: f64, y: f64) -> Vec<GlyphRun> {
        let case = corpus::parse_case(&json!({ "id": "render", "group": "render", "d": d }));
        assert!(case.d.axes.is_empty(), "render scenes use installed fonts only");
        let size = case.d.size();
        let lh = case.d.line_height();
        let align = case.d.align();
        let mut m = CanvasMeasurer::new(&mut self.registry, &mut self.cache, &case.d, None);
        let lines = legacy::text_source_lines(&case.d, &mut m, lh);
        let mut runs = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let measured = measure::measure(m.resolver, m.cache, &m.setup, &line.text);
            let width = measured.width as f64;
            let mut pen = match align {
                Align::Left => x,
                Align::Center => x - width / 2.0,
                Align::Right => x - width,
            };
            let baseline = y + lh * i as f64 + size * 0.82;
            for item in &measured.items {
                // Runs are in logical order; an RTL item's runs read right to left.
                let ordered: Vec<_> = if item.runs.iter().any(|r| r.rtl) { item.runs.iter().rev().collect() } else { item.runs.iter().collect() };
                for run in ordered {
                    let mut out = GlyphRun { face: run.face, size: run.size, coords: run.coords.clone(), glyphs: Vec::with_capacity(run.glyphs.len()) };
                    for g in &run.glyphs {
                        if g.glyph != UNMATCHED_VS_GLYPH {
                            out.glyphs.push(PGlyph { id: g.glyph, x: pen + g.x_offset as f64 / 65536.0, y: baseline - g.y_offset as f64 / 65536.0 });
                        }
                        pen += g.advance as f64 / 65536.0;
                    }
                    if !self.faces.contains_key(&run.face) {
                        let info = m.resolver.face(run.face);
                        self.faces.insert(
                            run.face,
                            FaceData {
                                data: info.data.0.clone(),
                                index: info.index,
                                postscript_name: info.postscript_name.clone(),
                                synthetic_bold: info.synthetic_bold,
                                synthetic_italic: info.synthetic_italic,
                                bitmap: has_bitmaps(&info.data.0, info.index),
                            },
                        );
                    }
                    runs.push(out);
                }
            }
        }
        runs
    }
}

fn has_bitmaps(data: &[u8], index: u32) -> bool {
    use read_fonts::types::Tag;
    read_fonts::FontRef::from_index(data, index).is_ok_and(|f| f.table_data(Tag::new(b"sbix")).is_some() || f.table_data(Tag::new(b"CBDT")).is_some())
}
