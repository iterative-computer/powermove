//! The measuring context today's raster.ts builds: `fontStr(d)` parsed into
//! Blink's font description, `letterSpacing` from tracking, and
//! `measureText` through the Blink port.

use crate::blink::measure::{self, Family};
use crate::blink::shaper::FontSetup;
use crate::font::{FaceId, FontDesc, FontResolver, PreparedCache};
use crate::legacy::{Measure, TextContent};
use std::collections::HashMap;

/// `fontStr(d)`'s family list: `[variableFamily, d.font, 'SF Pro Display']`
/// quoted, then `-apple-system, sans-serif`. `custom` is the `FontFace` the
/// variable-font renderer registered for this content, if any.
pub fn families(d: &TextContent, custom: Option<(FaceId, Vec<(u32, f32)>)>) -> Vec<Family> {
    let mut out = Vec::new();
    if let Some((face, settings)) = custom {
        out.push(Family::Custom { face, settings });
    }
    if let Some(font) = d.font.as_deref().filter(|f| !f.is_empty()) {
        out.push(Family::Named(font.to_string()));
    }
    out.push(Family::Named("SF Pro Display".into()));
    // `-apple-system` is not a Blink generic; it is matched as a family name.
    out.push(Family::Named("-apple-system".into()));
    out.push(Family::Generic("sans-serif"));
    out
}

/// The canvas font description: weight `d.weight || 500`, `italic`, size px.
pub fn font_desc(d: &TextContent) -> FontDesc {
    let weight = d.weight.filter(|w| *w != 0.0 && !w.is_nan()).unwrap_or(500.0) as f32;
    FontDesc { weight, italic: d.italic.unwrap_or(false), size: d.size.unwrap_or(16.0) as f32 }
}

/// `meas.letterSpacing = (d.tracking || 0) + 'px'`.
pub fn letter_spacing(d: &TextContent) -> f32 {
    d.tracking.filter(|t| !t.is_nan()).unwrap_or(0.0) as f32
}

/// A configured measuring context with a per-string memo (the legacy
/// algorithm re-measures prefixes many times).
pub struct CanvasMeasurer<'a> {
    pub resolver: &'a mut dyn FontResolver,
    pub cache: &'a mut PreparedCache,
    pub setup: FontSetup,
    memo: HashMap<Vec<u16>, f64>,
}

impl<'a> CanvasMeasurer<'a> {
    pub fn new(resolver: &'a mut dyn FontResolver, cache: &'a mut PreparedCache, d: &TextContent, custom: Option<(FaceId, Vec<(u32, f32)>)>) -> Self {
        let fams = families(d, custom);
        let setup = measure::build_setup(resolver, &fams, font_desc(d), letter_spacing(d));
        CanvasMeasurer { resolver, cache, setup, memo: HashMap::new() }
    }
}

impl Measure for CanvasMeasurer<'_> {
    fn width(&mut self, text: &[u16]) -> f64 {
        if let Some(w) = self.memo.get(text) {
            return *w;
        }
        let w = measure::measure(self.resolver, self.cache, &self.setup, text).width as f64;
        self.memo.insert(text.to_vec(), w);
        w
    }
}
