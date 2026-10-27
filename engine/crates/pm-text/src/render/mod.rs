//! Exact outline text coverage, ordered float paint, and color glyphs.
//! The `ink` feature has only the color glyph programs and strikes, and
//! where they draw (`ink`); `render` adds the painter.

#[cfg(feature = "render")]
pub mod atlas;
pub mod bitmap;
pub mod color;
pub mod color4;
pub mod colr;
#[cfg(all(target_os = "macos", feature = "coretext-reference"))]
pub mod coretext;
#[cfg(feature = "render")]
pub mod cpu;
pub mod ink;
pub mod outline;
#[cfg(feature = "render")]
pub mod paint;
#[cfg(feature = "render")]
pub mod scene;

#[cfg(feature = "render")]
use crate::compat::{Compat, GlyphRun, TextContent};
#[cfg(feature = "render")]
use crate::font::FontResolver;
#[cfg(feature = "render")]
use outline::FaceRef;
#[cfg(feature = "render")]
use scene::{Painter, Run, TextScene};

/// Runs borrowed from a resolver's faces.
#[cfg(feature = "render")]
pub fn runs<'a>(resolver: &'a dyn FontResolver, runs: &'a [GlyphRun]) -> Vec<Run<'a>> {
    runs.iter()
        .map(|r| Run {
            face: FaceRef::of(resolver.face(r.face)),
            size: r.size,
            coords: outline::Coords::User(&r.coords),
            synthetic_bold: r.synthetic_bold,
            synthetic_italic: r.synthetic_italic,
            oblique: 0.0,
            glyphs: &r.glyphs,
            bitmap_size: None,
        })
        .collect()
}

/// The canvas `rasterText(d, scale)` paints (`legacy/gl/raster.ts`), as a
/// scene: a `width` × `height` canvas scaled by `density`, every wrapped
/// line drawn with `fillText(line, x, pad + lh × i + size × 0.82)` in
/// `color`. `pad` and `x` come from the text geometry.
#[cfg(feature = "render")]
#[allow(clippy::too_many_arguments)]
pub fn text_raster<R: FontResolver>(
    painter: &mut Painter<'_>,
    compat: &mut Compat<R>,
    d: &TextContent,
    color: color::Rgba,
    pad: f64,
    x: f64,
    density: f64,
    width: u32,
    height: u32,
) -> TextScene {
    let lines = compat.line_runs(d);
    let mut scene = TextScene {
        width,
        height,
        items: Vec::new(),
    };
    let m = kurbo::Affine::scale(density);
    for (row, line) in lines.iter().enumerate() {
        let (relative, _) = compat.text_runs(d, &line.text, d.tracking.unwrap_or(0.0));
        let runs = runs(compat.resolver(), &relative);
        let origin = (
            f64::from((f64::from(x as f32) + line.x) as f32),
            f64::from(((pad + row as f64 * d.line_height()) + d.size() * 0.82) as f32),
        );
        painter.fill_text(&mut scene.items, &runs, origin, m, color, 1.0, 0.0);
    }
    scene
}
