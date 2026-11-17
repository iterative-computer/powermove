//! Vendored glyphs (`icons/`, see its README): where one stands for an SF
//! Symbol of the same meaning, the app draws it instead, in the same place,
//! size and tint. Pierre's set (Apache-2.0) covers the general chrome; glyphs
//! it has no counterpart for are drawn on the same grid (`icons/powermove`).
use resvg::{tiny_skia, usvg};

/// A vendored glyph: its SVG (16 px grid) and how many quarter turns
/// (clockwise) it is drawn at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub svg: &'static str,
    pub turns: u8,
}

const HOME: &str = include_str!("../icons/pierre/IconHome.svg");
const ALIGN_LEFT: &str = include_str!("../icons/pierre/IconAlignLeft.svg");
const ALIGN_CENTER: &str = include_str!("../icons/pierre/IconAlignCenter.svg");
const ALIGN_RIGHT: &str = include_str!("../icons/pierre/IconAlignRight.svg");
const DISTRIBUTE: &str = include_str!("../icons/powermove/distribute-horizontal.svg");

/// The vendored glyph that stands for SF Symbol `name`, if one does.
pub fn glyph(name: &str) -> Option<Glyph> {
    let g = |svg, turns| Some(Glyph { svg, turns });
    match name {
        "house" => g(HOME, 0),
        "align.horizontal.left" => g(ALIGN_LEFT, 0),
        "align.horizontal.center" => g(ALIGN_CENTER, 0),
        "align.horizontal.right" => g(ALIGN_RIGHT, 0),
        // The same three drawings turned a quarter: one family.
        "align.vertical.top" => g(ALIGN_LEFT, 1),
        "align.vertical.center" => g(ALIGN_CENTER, 1),
        "align.vertical.bottom" => g(ALIGN_RIGHT, 1),
        "distribute.horizontal.center" => g(DISTRIBUTE, 0),
        "distribute.vertical.center" => g(DISTRIBUTE, 1),
        _ => None,
    }
}

/// How much of a symbol's square a vendored glyph fills: the 16 px grid
/// drawn edge to edge reads larger than an SF Symbol of the same point size,
/// so it stands a little inside (matched by eye against SF neighbours).
pub const FILL: f32 = 14. / 16.;

/// `glyph`'s coverage (0–255 per pixel) on a `side` × `side` canvas, centred,
/// filling `FILL` of it.
pub fn coverage(glyph: Glyph, side: u32) -> Option<Vec<u8>> {
    let tree = usvg::Tree::from_str(glyph.svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(side, side)?;
    let scale = side as f32 * FILL / 16.;
    let offset = (side as f32 - 16. * scale) / 2.;
    let transform = tiny_skia::Transform::from_translate(offset, offset)
        .pre_scale(scale, scale)
        .pre_rotate_at(90. * glyph.turns as f32, 8., 8.);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(pixmap.pixels().iter().map(|p| p.alpha()).collect())
}

/// `glyph` tinted `color` (0xRRGGBB) as straight BGRA, GPUI's sprite format.
pub fn tinted_bgra(glyph: Glyph, side: u32, color: u32) -> Option<Vec<u8>> {
    let (r, g, b) = ((color >> 16) as u8, (color >> 8) as u8, color as u8);
    Some(
        coverage(glyph, side)?
            .into_iter()
            .flat_map(|a| [b, g, r, a])
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mapped glyph parses and draws something, and a quarter turn
    /// moves an align-left glyph's edge line to the top.
    #[test]
    fn every_glyph_draws_and_turns() {
        for name in [
            "house",
            "align.horizontal.left",
            "align.horizontal.center",
            "align.horizontal.right",
            "align.vertical.top",
            "align.vertical.center",
            "align.vertical.bottom",
            "distribute.horizontal.center",
            "distribute.vertical.center",
        ] {
            let c = coverage(glyph(name).unwrap(), 32).unwrap();
            assert!(c.iter().any(|a| *a > 200), "{name} draws nothing");
        }
        let side = 32usize;
        let at = |c: &[u8], x: usize, y: usize| c[y * side + x];
        let left = coverage(glyph("align.horizontal.left").unwrap(), 32).unwrap();
        let top = coverage(glyph("align.vertical.top").unwrap(), 32).unwrap();
        // The edge line: a column at the left of `left`, a row at the top of `top`.
        let edge = (32. - 32. * FILL) as usize / 2 + 1;
        assert!(at(&left, edge, 16) > 200 && at(&left, 16, edge) < 50);
        assert!(at(&top, 16, edge) > 200 && at(&top, edge, 16) < 50);
    }
}
