//! The measuring context today's raster.ts builds: `fontStr(d)` parsed into
//! Blink's font description, `letterSpacing` from tracking, and
//! `measureText` through the Blink port.

use super::content::TextContent;
use super::wrap::Measure;
use crate::blink::measure::{self, Family, Measured};
use crate::blink::shaper::FontSetup;
use crate::font::{FaceId, FontDesc, FontResolver, PreparedCache, tag};
use std::collections::HashMap;

/// The canvas default font (`10px sans-serif`), which a canvas keeps when
/// the `font` string it is given does not parse.
const DEFAULT_DESC: FontDesc = FontDesc { weight: 400.0, italic: false, size: 10.0 };

/// The parsed canvas font: the family list and the description.
#[derive(Clone, Debug)]
pub struct CanvasFont {
    pub families: Vec<Family>,
    pub desc: FontDesc,
}

/// `meas.font = fontStr(d)`:
///
/// ```js
/// `${d.italic ? 'italic ' : ''}${d.weight || 500} ${d.size}px ${families}, -apple-system, sans-serif`
/// ```
///
/// with `families` = `[variableFamily, d.font, 'SF Pro Display']`, quoted.
/// When the string does not parse (a missing or negative size, a weight
/// outside 1..1000) the canvas keeps its default font.
pub fn canvas_font(d: &TextContent, variable: Option<(FaceId, Vec<(u32, f32)>)>) -> CanvasFont {
    let weight = d.weight.filter(|w| *w != 0.0 && !w.is_nan()).unwrap_or(500.0);
    let size = d.size;
    let valid = (1.0..=1000.0).contains(&weight) && size.is_some_and(|s| s.is_finite() && s >= 0.0);
    if !valid {
        return CanvasFont { families: vec![Family::Generic("sans-serif")], desc: DEFAULT_DESC };
    }
    let mut families = Vec::new();
    if let Some((face, settings)) = variable {
        families.push(Family::Custom { face, settings });
    }
    if let Some(font) = d.font.as_deref().filter(|f| !f.is_empty()) {
        families.push(Family::Named(font.to_string()));
    }
    families.push(Family::Named("SF Pro Display".into()));
    // `-apple-system` is not a Blink generic; it is matched as a family name.
    families.push(Family::Named("-apple-system".into()));
    families.push(Family::Generic("sans-serif"));
    CanvasFont { families, desc: FontDesc { weight: weight as f32, italic: d.italic.unwrap_or(false), size: size.unwrap_or(16.0) as f32 } }
}

/// `meas.letterSpacing = (d.tracking || 0) + 'px'`.
pub fn letter_spacing(d: &TextContent) -> f32 {
    d.tracking.filter(|t| t.is_finite()).unwrap_or(0.0) as f32
}

/// `variationFontFamily(d)` (typography/font-renderer.ts): a `FontFace`
/// from the family's variable source with the layer's `fontAxis.*` values,
/// when the layer sets any and the source exists.
pub fn variable_family(resolver: &mut dyn FontResolver, d: &TextContent) -> Option<(FaceId, Vec<(u32, f32)>)> {
    if d.axes.is_empty() {
        return None;
    }
    let family = d.font.as_deref()?.trim();
    if family.is_empty() {
        return None;
    }
    let face = resolver.variable_face(family)?;
    let settings = d.axes.iter().map(|(t, v)| (tag(t), *v as f32)).collect();
    Some((face, settings))
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
    pub fn new(resolver: &'a mut dyn FontResolver, cache: &'a mut PreparedCache, d: &TextContent) -> Self {
        let variable = variable_family(resolver, d);
        let font = canvas_font(d, variable);
        let setup = measure::build_setup(resolver, &font.families, font.desc, letter_spacing(d));
        CanvasMeasurer { resolver, cache, setup, memo: HashMap::new() }
    }

    /// The full measurement (shaped runs and their faces), unmemoized.
    pub fn shape(&mut self, text: &[u16]) -> Measured {
        measure::measure(self.resolver, self.cache, &self.setup, text)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_font_strings_keep_the_canvas_default() {
        let d = TextContent { size: None, ..Default::default() };
        assert_eq!(canvas_font(&d, None).desc.size, 10.0);
        let d = TextContent { size: Some(24.0), weight: Some(1200.0), ..Default::default() };
        assert_eq!(canvas_font(&d, None).desc.weight, 400.0);
        let d = TextContent { size: Some(24.0), weight: Some(0.0), font: Some("Avenir".into()), ..Default::default() };
        let f = canvas_font(&d, None);
        assert_eq!((f.desc.weight, f.desc.size), (500.0, 24.0));
        assert!(matches!(&f.families[0], Family::Named(n) if n == "Avenir"));
        assert_eq!(f.families.len(), 4);
    }
}
