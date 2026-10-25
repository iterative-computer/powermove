//! Where a color glyph draws, without painting it: the ink the painter's
//! own placement gives a bitmap strike or a COLR program. pm-eval measures
//! text bounds with it (`text_measure`), in pm-wasm too.

use kurbo::{Affine, Rect};

use super::bitmap::{has_bitmaps, ink_bounds, is_colr_glyph};
use super::colr::{ColrRequest, program};
use super::outline::{Coords, FaceRef};

/// The drawn area of a bitmap or COLR glyph relative to its baseline
/// origin at device scale 1, from the draw path's own placement: the
/// strike's nonzero texels and reconstruction reach for a bitmap face
/// (`bitmap::ink_bounds`; `bitmap_size` is the resolved full-em size the
/// painter passes, the requested `size` when absent), or the unrounded
/// painted/clip bounds of the COLR program. `None` means the glyph is an
/// ordinary outline; `Some(None)` that it draws nothing.
pub fn drawn_color_glyph_ink(
    face: FaceRef<'_>,
    coords: Coords<'_>,
    glyph: u32,
    size: f32,
    skew: f64,
    bitmap_size: Option<f32>,
) -> Option<Option<Rect>> {
    if has_bitmaps(face)
        && let Some(ink) = ink_bounds(face, glyph, size, bitmap_size.unwrap_or(size))
    {
        return Some(ink);
    }
    if !is_colr_glyph(face, glyph) {
        return None;
    }
    // The painter draws nothing for a program it cannot build.
    let program = program(ColrRequest {
        face,
        coords,
        id: glyph,
        size,
        skew: skew as f32,
        linear: Affine::IDENTITY,
        origin: (0.0, 0.0),
        foreground: [1.0; 4],
    })
    .ok()
    .flatten();
    Some(program.map(|p| p.painted_bounds))
}
