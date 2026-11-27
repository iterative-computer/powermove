//! The one selection ring every Settings picker draws (Appearance, Open
//! Projects In, the plan cards): an accent stroke standing the same gap
//! (`settings.picker_ring_gap`) outside the chosen tile on all four sides,
//! `settings.picker_ring` wide, concentric with it: its inner edge's corner
//! is the tile's plus the gap (`Radii::around`, the inverse of `nested`).
//! The stroke is centred on a line half its width beyond that edge, so the
//! visible gap is exactly the token.
use objc2::rc::Retained;
use objc2_app_kit::{NSBezierPath, NSColor};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use pm_design::{Radii, Settings};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Ring {
    gap: f64,
    width: f64,
}
impl Ring {
    pub(super) fn of(s: &Settings) -> Self {
        Self {
            gap: s.picker_ring_gap as f64,
            width: s.picker_ring as f64,
        }
    }
    /// The room the ring takes outside its tile, the same on every side.
    pub(super) fn margin(&self) -> f64 {
        self.gap + self.width
    }
    /// The stroke's centre line around `tile` (corner `radius`): its rect
    /// and its corner.
    pub(super) fn centre_line(&self, radii: &Radii, tile: NSRect, radius: f64) -> (NSRect, f64) {
        let o = self.gap + self.width / 2.;
        let rect = NSRect::new(
            NSPoint::new(tile.origin.x - o, tile.origin.y - o),
            NSSize::new(tile.size.width + 2. * o, tile.size.height + 2. * o),
        );
        let inner = radii.around(radius as f32, self.gap as f32) as f64;
        (rect, inner + self.width / 2.)
    }
    /// Strokes the ring around `tile` in `ink` in the current context.
    pub(super) fn stroke(&self, radii: &Radii, tile: NSRect, radius: f64, ink: &NSColor) {
        let (rect, corner) = self.centre_line(radii, tile, radius);
        let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, corner, corner);
        ink.setStroke();
        path.setLineWidth(self.width);
        path.stroke();
    }
}

/// A token colour (0xRRGGBB) in sRGB.
pub(super) fn srgb(rgb: u32, alpha: f64) -> Retained<NSColor> {
    let channel = |shift: u32| ((rgb >> shift) & 255) as f64 / 255.;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_design::{Scheme, Tokens};

    /// Equal gap on every side, concentric corners, the stroke centred so
    /// its inner edge stands exactly the gap off the tile, and every edge on
    /// a whole point (crisp at 1x and 2x) when the tile's is.
    #[test]
    fn the_ring_stands_one_gap_off_its_tile_on_every_side_and_is_concentric() {
        for scheme in [Scheme::Light, Scheme::Dark] {
            let s = Settings::of(scheme);
            let radii = Tokens::new(scheme, Default::default()).radius;
            let ring = Ring::of(&s);
            let tile = NSRect::new(NSPoint::new(10., 20.), NSSize::new(96., 64.));
            for radius in [s.picker_radius as f64, 6., 10.] {
                let (line, corner) = ring.centre_line(&radii, tile, radius);
                let half = ring.width / 2.;
                let inner = (
                    line.origin.x + half,
                    line.origin.y + half,
                    line.origin.x + line.size.width - half,
                    line.origin.y + line.size.height - half,
                );
                let gaps = [
                    tile.origin.x - inner.0,
                    tile.origin.y - inner.1,
                    inner.2 - (tile.origin.x + tile.size.width),
                    inner.3 - (tile.origin.y + tile.size.height),
                ];
                assert!(gaps.iter().all(|g| (g - ring.gap).abs() < 1e-9), "{gaps:?}");
                // Inner edge corner = tile corner + gap; outer = inner + width.
                assert_eq!(corner - half, radius + ring.gap);
                assert_eq!(
                    radii.nested((corner - half) as f32, ring.gap as f32) as f64,
                    radius.max(radii.sm as f64)
                );
                for edge in [inner.0, inner.1, inner.0 - ring.width, inner.1 - ring.width] {
                    assert_eq!(edge.fract(), 0., "an edge off the point grid");
                }
                assert_eq!(ring.margin(), ring.gap + ring.width);
            }
        }
    }
}
