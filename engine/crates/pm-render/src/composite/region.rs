//! Regions of interest and effect padding (plan section 8: "region of
//! interest with effect padding; bounds-expanding effects grow up to a
//! limit"; "cache granularity: region of interest on final output only").
//!
//! A region renders the full frame's geometry through a viewport placed so
//! the region lands at the target's origin, so every pixel of a region
//! equals the full frame's (`exec::region_viewport`). Per-pixel steps
//! (masks, mattes, blend modes, opacity) need nothing beyond the region.
//! An effect that reads neighbors needs its input over the region grown by
//! its padding; a pass holding such a step renders over the grown
//! rectangle (clamped to the frame, the limit) and the region is cut from
//! it.

use super::effects::Padding;

/// A rectangle of a target's full frame, in top-down pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    /// The whole frame.
    pub const fn full(width: u32, height: u32) -> Region {
        Region {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// The region grown by `padding` on every side, clamped to the frame.
    pub fn grow(self, padding: Padding, full: (u32, u32)) -> Region {
        match padding {
            Padding::Full => Region::full(full.0, full.1),
            Padding::Pixels(p) => {
                let x0 = self.x.saturating_sub(p);
                let y0 = self.y.saturating_sub(p);
                let x1 = (self.x + self.width).saturating_add(p).min(full.0);
                let y1 = (self.y + self.height).saturating_add(p).min(full.1);
                Region {
                    x: x0,
                    y: y0,
                    width: x1 - x0,
                    height: y1 - y0,
                }
            }
        }
    }

    /// The origin in GL row order (row 0 at the bottom of the frame).
    pub fn gl_origin(self, full_height: u32) -> (i32, i32) {
        (
            self.x as i32,
            full_height as i32 - self.y as i32 - self.height as i32,
        )
    }

    /// The offset from a pixel of a target covering `self` to the texel of
    /// a texture covering `source`.
    pub fn texel_offset(self, source: Region, full_height: u32) -> (i32, i32) {
        let (tx, ty) = self.gl_origin(full_height);
        let (sx, sy) = source.gl_origin(full_height);
        (tx - sx, ty - sy)
    }

    /// The viewport that draws full-frame geometry so this region lands at
    /// the target's origin: `[x, y, width, height]`.
    pub fn viewport(self, full: (u32, u32)) -> [f32; 4] {
        let below = full.1 - self.y - self.height;
        [
            -(self.x as f32),
            -(below as f32),
            full.0 as f32,
            full.1 as f32,
        ]
    }

    /// Whether `other` lies inside this region.
    pub fn contains(self, other: Region) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.width <= self.x + self.width
            && other.y + other.height <= self.y + self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_is_clamped_to_the_frame() {
        let r = Region {
            x: 10,
            y: 5,
            width: 20,
            height: 10,
        };
        assert_eq!(
            r.grow(Padding::Pixels(8), (100, 100)),
            Region {
                x: 2,
                y: 0,
                width: 36,
                height: 23
            }
        );
        assert_eq!(r.grow(Padding::Full, (100, 50)), Region::full(100, 50));
        assert_eq!(
            Region::full(40, 30).grow(Padding::Pixels(5), (40, 30)),
            Region::full(40, 30)
        );
    }

    #[test]
    fn texel_offsets_follow_gl_rows() {
        let full = 100;
        let inner = Region {
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        };
        let outer = inner.grow(Padding::Pixels(5), (100, 100));
        // The inner region's bottom-left pixel is 5 texels in from the
        // outer one's, on both axes.
        assert_eq!(inner.texel_offset(outer, full), (5, 5));
        assert_eq!(outer.texel_offset(outer, full), (0, 0));
        assert!(outer.contains(inner));
    }
}
