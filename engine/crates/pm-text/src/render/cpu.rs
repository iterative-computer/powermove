//! The CPU coverage fallback (`vello_cpu`, amendment 10): the same pages
//! the GPU path renders, with alpha exported as R8 coverage. S4 measured
//! `vello_cpu` matching Vello's GPU
//! coverage to the third decimal of SSIM.

use kurbo::{BezPath, Shape};
use vello_cpu::color::palette::css::WHITE;
use vello_cpu::{Pixmap, RenderContext, Resources};

use super::atlas::Atlas;
use super::scene::{CoveragePathCache, ItemContent, TextScene};

/// Reusable Vello CPU state for coverage pages. A renderer can retain one
/// instance across frames, including when the page dimensions change.
pub struct Rasterizer {
    context: RenderContext,
    resources: Resources,
    pixmap: Pixmap,
    alpha: Vec<u8>,
    paths: CoveragePathCache,
}

impl Default for Rasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Rasterizer {
    pub fn new() -> Self {
        Self {
            context: RenderContext::new(1, 1),
            resources: Resources::new(),
            pixmap: Pixmap::new(1, 1),
            alpha: Vec::new(),
            paths: CoveragePathCache::default(),
        }
    }

    /// The same device-space path cache used by CPU pages; the GPU page
    /// builder may borrow it to avoid duplicating flattened geometry.
    pub fn coverage_cache(&mut self) -> &mut CoveragePathCache {
        &mut self.paths
    }

    fn draw(&mut self, scene: &TextScene, atlas: &Atlas, page: usize) -> (u32, u32) {
        let (w, h) = atlas.pages[page];
        let (w16, h16) = (
            w.clamp(1, u32::from(u16::MAX)) as u16,
            h.clamp(1, u32::from(u16::MAX)) as u16,
        );
        self.context.reset_and_resize(w16, h16);
        self.context.set_paint(WHITE);
        for cell in atlas.cells.iter().filter(|c| c.page == page) {
            let ItemContent::Coverage { paths } = &scene.items[cell.item].content else {
                continue;
            };
            let to_page = cell.transform();
            if cell.clip {
                self.context.reset_transform();
                let clip: BezPath = cell.rect().to_path(0.1);
                self.context.push_clip_layer(&clip);
            }
            for p in paths {
                let shape = self.paths.get(p);
                let [_, _, _, _, x, y] = p.transform.as_coeffs();
                self.context
                    .set_transform(to_page * kurbo::Affine::translate((x, y)));
                self.context.set_fill_rule(vello_cpu::peniko::Fill::NonZero);
                self.context.fill_path(shape.as_ref());
            }
            if cell.clip {
                self.context.pop_layer();
            }
        }
        self.context.flush();
        self.pixmap.resize(w16, h16);
        self.pixmap.data_as_u8_slice_mut().fill(0);
        self.context.render(&mut self.pixmap, &mut self.resources);
        (w, h)
    }

    /// RGBA8 white-on-transparent coverage, borrowed until the next draw.
    pub fn render_page_rgba(
        &mut self,
        scene: &TextScene,
        atlas: &Atlas,
        page: usize,
    ) -> (u32, u32, &[u8]) {
        let (w, h) = self.draw(scene, atlas, page);
        (w, h, self.pixmap.data_as_u8_slice())
    }

    /// Tightly packed R8 alpha coverage, borrowed until the next draw.
    pub fn render_page_r8(
        &mut self,
        scene: &TextScene,
        atlas: &Atlas,
        page: usize,
    ) -> (u32, u32, &[u8]) {
        let (w, h) = self.draw(scene, atlas, page);
        self.alpha.clear();
        self.alpha.reserve((w * h) as usize);
        self.alpha
            .extend(self.pixmap.data().iter().map(|pixel| pixel.a));
        (w, h, &self.alpha)
    }
}

/// Standalone page rasterization for callers that need owned RGBA8 rows.
pub fn render_page(scene: &TextScene, atlas: &Atlas, page: usize) -> (u32, u32, Vec<u8>) {
    let mut rasterizer = Rasterizer::new();
    let (w, h, bytes) = rasterizer.render_page_rgba(scene, atlas, page);
    (w, h, bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::atlas;
    use crate::render::scene::{fill_path, rounded_rect};
    use kurbo::Affine;

    #[test]
    fn reused_rasterizer_resizes_and_clears_previous_page() {
        let scene = |x, width, box_width| TextScene {
            width,
            height: 12,
            items: vec![
                fill_path(
                    rounded_rect(x, 2.0, box_width, 6.0, 0.0),
                    Affine::IDENTITY,
                    [1.0; 4],
                    1.0,
                )
                .unwrap(),
            ],
        };
        let first = scene(1.0, 12, 4.0);
        let second = scene(7.0, 24, 10.0);
        let mut rasterizer = Rasterizer::new();
        for scene in [&first, &second, &first] {
            let atlas = atlas::pack(scene, 128);
            let (w, h, alpha) = rasterizer.render_page_r8(scene, &atlas, 0);
            let (ow, oh, rgba) = render_page(scene, &atlas, 0);
            assert_eq!((w, h), (ow, oh));
            assert_eq!(alpha.len(), (w * h) as usize);
            assert_eq!(
                alpha,
                rgba.iter().skip(3).step_by(4).copied().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn curved_glyph_matches_independent_pixel_area() {
        fn pixels(path: BezPath) -> Vec<f64> {
            let scene = TextScene {
                width: 8,
                height: 4,
                items: vec![fill_path(path, Affine::IDENTITY, [1.0; 4], 1.0).unwrap()],
            };
            let atlas = atlas::pack(&scene, 128);
            let cell = atlas.cells[0];
            let mut rasterizer = Rasterizer::new();
            let (w, _, bytes) = rasterizer.render_page_r8(&scene, &atlas, 0);
            let mut out = Vec::with_capacity(32);
            for y in 0..4 {
                for x in 0..8 {
                    let sx = i64::from(x) - cell.device_x + i64::from(cell.x);
                    let sy = i64::from(y) - cell.device_y + i64::from(cell.y);
                    out.push(f64::from(bytes[sy as usize * w as usize + sx as usize]) / 255.0);
                }
            }
            out
        }

        // x = 8t, y = 16t(1-t), so the exact upper edge is
        // y(x) = 2x - x²/4. Integrate each pixel's vertical intersection
        // independently by midpoint quadrature in x.
        let mut curve = BezPath::new();
        curve.move_to((0.0, 0.0));
        curve.quad_to((4.0, 8.0), (8.0, 0.0));
        curve.close_path();
        let exact = pixels(curve.clone());
        let reference: Vec<f64> = (0..4)
            .flat_map(|y| {
                (0..8).map(move |x| {
                    (0..4096)
                        .map(|sub| {
                            let px = x as f64 + (sub as f64 + 0.5) / 4096.0;
                            (2.0 * px - px * px / 4.0 - y as f64).clamp(0.0, 1.0)
                        })
                        .sum::<f64>()
                        / 4096.0
                })
            })
            .collect();
        let max_error = |pixels: &[f64]| {
            pixels
                .iter()
                .zip(&reference)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max)
        };
        let mut coarse = BezPath::new();
        kurbo::flatten(curve.iter(), 0.25, |el| coarse.push(el));
        assert!(max_error(&exact) < 0.015, "{}", max_error(&exact));
        assert!(max_error(&pixels(coarse)) > 0.04);
    }
}
