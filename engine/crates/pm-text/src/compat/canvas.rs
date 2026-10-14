//! The measuring context today's raster.ts builds: `fontStr(d)` parsed into
//! Blink's font description, `letterSpacing` from tracking, and
//! `measureText` through the Blink port.

use super::content::TextContent;
use super::wrap::Measure;
use crate::blink::measure::{self, Family, FontCacheSeed, Measured};
use crate::blink::shaper::FontSetup;
use crate::font::{FaceId, FontDesc, FontResolver, PreparedCache, tag};
use std::collections::HashMap;

/// The canvas default font (`10px sans-serif`), which a canvas keeps when
/// the `font` string it is given does not parse.
const DEFAULT_DESC: FontDesc = FontDesc {
    weight: 400.0,
    italic: false,
    size: 10.0,
};

/// The parsed canvas font: the family list and the description.
#[derive(Clone, Debug)]
pub struct CanvasFont {
    pub families: Vec<Family>,
    /// The description at Blink's effective size.
    pub desc: FontDesc,
    /// The specified size (`d.size` in `float`, before the floor).
    pub specified: f32,
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
    let weight = d
        .weight
        .filter(|w| *w != 0.0 && !w.is_nan())
        .unwrap_or(500.0);
    let size = d.size;
    let valid = (1.0..=1000.0).contains(&weight) && size.is_some_and(|s| s.is_finite() && s >= 0.0);
    if !valid {
        return CanvasFont {
            families: vec![Family::Generic("sans-serif")],
            desc: DEFAULT_DESC,
            specified: DEFAULT_DESC.size,
        };
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
    let specified = size.unwrap_or(16.0) as f32;
    CanvasFont {
        families,
        desc: FontDesc {
            weight: weight as f32,
            italic: d.italic.unwrap_or(false),
            size: effective_font_size(specified),
        },
        specified,
    }
}

/// Blink's `FontDescription::EffectiveFontSize()`: the computed size floored
/// to the font cache key's precision (`FontCacheKey::PrecisionMultiplier()`,
/// 100), in `float`:
///
/// ```cpp
/// return floorf(size * FontCacheKey::PrecisionMultiplier()) / FontCacheKey::PrecisionMultiplier();
/// ```
///
/// Chrome creates, scales, and measures every font at this size, so a
/// caption shrunk to 15.9048 px measures at 15.9 px. Two things still see
/// the specified size: HarfBuzz's `trak` lookup (`ptem`) and `opsz`
/// (`FontSetup::specified`). The floor is not idempotent in `f32` (`1.06px`
/// measures at 1.05 px, and flooring again gives 1.04), so it is applied
/// exactly once, here, where the canvas font is parsed.
pub fn effective_font_size(size: f32) -> f32 {
    const PRECISION: f32 = 100.0;
    (size * PRECISION).floor() / PRECISION
}

/// `meas.letterSpacing = (d.tracking || 0) + 'px'`.
pub fn letter_spacing(d: &TextContent) -> f32 {
    d.tracking.filter(|t| t.is_finite()).unwrap_or(0.0) as f32
}

/// `variationFontFamily(d)` (typography/font-renderer.ts): a `FontFace`
/// from the family's variable source with the layer's `fontAxis.*` values,
/// when the layer sets any and the source exists.
pub fn variable_family(
    resolver: &mut dyn FontResolver,
    d: &TextContent,
) -> Option<(FaceId, Vec<(u32, f32)>)> {
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
    pub fn new(
        resolver: &'a mut dyn FontResolver,
        cache: &'a mut PreparedCache,
        d: &TextContent,
        seeds: &[FontCacheSeed],
    ) -> Self {
        let variable = variable_family(resolver, d);
        let font = canvas_font(d, variable);
        let setup = measure::build_setup(
            resolver,
            &font.families,
            font.desc,
            font.specified,
            letter_spacing(d),
            seeds,
        );
        CanvasMeasurer {
            resolver,
            cache,
            setup,
            memo: HashMap::new(),
        }
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
        let d = TextContent {
            size: None,
            ..Default::default()
        };
        assert_eq!(canvas_font(&d, None).desc.size, 10.0);
        let d = TextContent {
            size: Some(24.0),
            weight: Some(1200.0),
            ..Default::default()
        };
        assert_eq!(canvas_font(&d, None).desc.weight, 400.0);
        let d = TextContent {
            size: Some(24.0),
            weight: Some(0.0),
            font: Some("Avenir".into()),
            ..Default::default()
        };
        let f = canvas_font(&d, None);
        assert_eq!((f.desc.weight, f.desc.size), (500.0, 24.0));
        assert!(matches!(&f.families[0], Family::Named(n) if n == "Avenir"));
        assert_eq!(f.families.len(), 4);
    }

    #[test]
    fn sizes_are_floored_to_blinks_font_cache_precision() {
        let at = |size: f64| {
            canvas_font(
                &TextContent {
                    size: Some(size),
                    ..Default::default()
                },
                None,
            )
            .desc
            .size
        };
        // The corpus caption `captions/middle-lines` shrinks to 15.9048 px;
        // Chrome measures it at 15.9 px.
        assert_eq!(at(15.9048), 15.9f32);
        assert_eq!(at(9.3245), 9.32f32);
        assert_eq!(at(10.1722), 10.17f32);
        // Sizes on the grid stay (integers are exact in f32).
        assert_eq!(at(24.0), 24.0);
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(0.004), 0.0);
        // Some sizes on the decimal grid are not fixed points in f32, and the
        // floor is not idempotent: Chrome measures `1.06px` at 1.05 px, and a
        // second floor would give 1.04.
        assert_eq!(at(1.06), 1.05f32);
        assert_eq!(effective_font_size(effective_font_size(1.06)), 1.04f32);
    }

    #[test]
    fn the_specified_size_is_kept_for_trak_and_opsz() {
        let f = canvas_font(
            &TextContent {
                size: Some(11.867549668874172),
                ..Default::default()
            },
            None,
        );
        assert_eq!(
            (f.desc.size, f.specified),
            (11.86f32, 11.867549668874172f64 as f32)
        );
        // The canvas default font when the string does not parse.
        let f = canvas_font(
            &TextContent {
                size: Some(-1.0),
                ..Default::default()
            },
            None,
        );
        assert_eq!((f.desc.size, f.specified), (10.0, 10.0));
    }
}
