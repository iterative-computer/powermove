//! Glyph rendering (plan section 12, amendment 10; design notes in
//! `docs/engine/text-render.md`).
//!
//! - [`profile`]: the engine-owned stem-darkening and positioning profiles
//!   (Canvas2D for `compat` level 1, CoreText for level 2).
//! - [`outline`], [`bitmap`]: glyph outlines (Skrifa, synthetic bold and
//!   italic, the profile's expansion) and color bitmap glyphs placed as
//!   Canvas2D places them, both cached across frames.
//! - [`scene`]: a text raster as ordered canvas draws (coverage items and
//!   image items) in device pixels; [`text_raster`] builds the scene of a text
//!   layer's `rasterText` canvas.
//! - [`atlas`]: coverage cells 1:1 with device pixels.
//! - [`cpu`], [`paint`]: the `vello_cpu` coverage fallback and the float
//!   paint pass on the CPU (the reference for the GPU pass in `pm-render`).
//! - [`color`]: canvas color strings.

pub mod atlas;
pub mod bitmap;
#[cfg(all(target_os = "macos", feature = "cg-masks"))]
pub mod cgmask;
pub mod color;
pub mod color4;
#[cfg(feature = "cg-masks")]
pub mod colr;
#[cfg(all(target_os = "macos", feature = "cg-masks"))]
pub mod coretext;
pub mod cpu;
pub mod graphite;
pub mod mask_tables;
pub mod outline;
pub mod paint;
pub mod profile;
pub mod scene;
pub mod sdf;
#[cfg(feature = "cg-masks")]
pub mod skaaa;
#[cfg(feature = "cg-masks")]
pub mod skpath;

use kurbo::Affine;

use crate::compat::{Compat, GlyphRun, TextContent};
use crate::font::FontResolver;
use outline::FaceRef;
use scene::{Painter, Run, TextScene};

/// Runs borrowed from a resolver's faces.
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
    let m = Affine::scale(density);
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
