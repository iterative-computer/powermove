//! The `vello_cpu` coverage backend: the CPU fallback of section 26's
//! failure path (amendment 10). It renders the same atlas the GPU backend
//! renders, so the engine's float paint pass is shared.

use kurbo::Affine;
use peniko::Fill;
use peniko::color::AlphaColor;
use vello_cpu::{Pixmap, RenderContext, RenderSettings, Resources};

use crate::render::scene::{AtlasLayout, CoverageScene, FillRule, ItemStyle};

/// Coverage pages as 8-bit premultiplied RGBA (white × coverage), the format
/// Vello's GPU backend writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveragePages {
    /// One RGBA8 buffer per page, `width × height × 4` bytes, top-down.
    pub pages: Vec<Vec<u8>>,
}

impl CoveragePages {
    /// The coverage (alpha) of page `page` at `(x, y)`.
    pub fn coverage(&self, layout: &AtlasLayout, page: usize, x: u32, y: u32) -> u8 {
        let (w, _) = layout.pages[page];
        self.pages[page][((y * w + x) * 4 + 3) as usize]
    }
}

/// Renders every cell of `layout` with `vello_cpu` (single-threaded, u8
/// pipeline: coverage only needs 8 bits).
pub fn render_coverage(scene: &CoverageScene, layout: &AtlasLayout) -> CoveragePages {
    let mut pages = Vec::with_capacity(layout.pages.len());
    for (index, &(w, h)) in layout.pages.iter().enumerate() {
        let (w16, h16) = (w.min(u32::from(u16::MAX)) as u16, h.min(u32::from(u16::MAX)) as u16);
        let settings = RenderSettings {
            num_threads: 0,
            ..RenderSettings::default()
        };
        let mut ctx = RenderContext::new_with(w16, h16, settings);
        ctx.set_paint(AlphaColor::from_rgba8(255, 255, 255, 255));
        for cell in layout.cells.iter().filter(|c| c.page == index) {
            let item = &scene.items[cell.item];
            let to_cell = AtlasLayout::cell_transform(cell);
            // Keep the item inside its cell (its bounds are conservative,
            // but a clip makes the guarantee exact).
            let cell_rect = kurbo::Rect::new(
                f64::from(cell.x),
                f64::from(cell.y),
                f64::from(cell.x + cell.device.width),
                f64::from(cell.y + cell.device.height),
            );
            ctx.push_clip_layer(&kurbo::Shape::to_path(&cell_rect, 0.1));
            ctx.set_transform(to_cell * item.transform);
            match &item.style {
                ItemStyle::Fill(rule) => {
                    ctx.set_fill_rule(match rule {
                        FillRule::NonZero => Fill::NonZero,
                        FillRule::EvenOdd => Fill::EvenOdd,
                    });
                    ctx.fill_path(&item.path);
                }
                ItemStyle::Stroke(stroke) => {
                    ctx.set_stroke(stroke.clone());
                    ctx.stroke_path(&item.path);
                }
            }
            ctx.set_transform(Affine::IDENTITY);
            ctx.pop_layer();
        }
        ctx.flush();
        let mut pixmap = Pixmap::new(w16, h16);
        let mut resources = Resources::new();
        ctx.render(&mut pixmap, &mut resources);
        pages.push(pixmap.data_as_u8_slice().to_vec());
    }
    CoveragePages { pages }
}

/// Composites the scene from its coverage in f32 (the reference for the
/// GPU float paint pass): premultiplied source-over of `paint × coverage`,
/// item by item, into a `region`-sized premultiplied RGBA buffer.
pub fn paint_reference(
    scene: &CoverageScene,
    layout: &AtlasLayout,
    coverage: &CoveragePages,
    region: crate::render::scene::PixelRect,
) -> Vec<[f32; 4]> {
    let mut out = vec![[0.0f32; 4]; (region.width * region.height) as usize];
    for cell in &layout.cells {
        let paint = scene.items[cell.item].premultiplied();
        let Some(r) = cell.device.intersect(region) else {
            continue;
        };
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let cx = cell.x + (x - cell.device.x) as u32;
                let cy = cell.y + (y - cell.device.y) as u32;
                let cov = f32::from(coverage.coverage(layout, cell.page, cx, cy)) / 255.0;
                if cov == 0.0 {
                    continue;
                }
                let i = ((y - region.y) as u32 * region.width + (x - region.x) as u32) as usize;
                let d = out[i];
                let sa = paint[3] * cov;
                for c in 0..4 {
                    out[i][c] = paint[c] * cov + d[c] * (1.0 - sa);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::scene::{CoverageItem, PixelRect};
    use kurbo::{Rect, Shape};

    fn rect_item(r: Rect, color: [f32; 4]) -> CoverageItem {
        CoverageItem {
            path: r.to_path(0.1),
            transform: Affine::IDENTITY,
            style: ItemStyle::Fill(FillRule::NonZero),
            color,
        }
    }

    #[test]
    fn half_pixel_edges_have_half_coverage() {
        let mut scene = CoverageScene::default();
        scene.items.push(rect_item(Rect::new(2.5, 2.0, 10.0, 6.0), [1.0; 4]));
        let clip = PixelRect::new(0, 0, 16, 16);
        let layout = AtlasLayout::pack(&scene, clip, 256);
        let cov = render_coverage(&scene, &layout);
        let cell = layout.cells[0];
        assert_eq!(cell.device, PixelRect::new(2, 2, 8, 4));
        // Column 2 is half covered; column 3 is fully covered.
        let at = |x: i32, y: i32| {
            cov.coverage(&layout, 0, cell.x + (x - 2) as u32, cell.y + (y - 2) as u32)
        };
        assert!((i32::from(at(2, 3)) - 128).abs() <= 1, "{}", at(2, 3));
        assert_eq!(at(3, 3), 255);
    }

    #[test]
    fn items_composite_in_order() {
        let mut scene = CoverageScene::default();
        scene.items.push(rect_item(Rect::new(0.0, 0.0, 8.0, 8.0), [1.0, 0.0, 0.0, 1.0]));
        scene.items.push(rect_item(Rect::new(4.0, 0.0, 8.0, 8.0), [0.0, 0.0, 1.0, 0.5]));
        let clip = PixelRect::new(0, 0, 8, 8);
        let layout = AtlasLayout::pack(&scene, clip, 256);
        let cov = render_coverage(&scene, &layout);
        let out = paint_reference(&scene, &layout, &cov, clip);
        assert_eq!(out[0], [1.0, 0.0, 0.0, 1.0]);
        let p = out[6];
        assert!((p[0] - 0.5).abs() < 1e-6 && (p[2] - 0.5).abs() < 1e-6 && (p[3] - 1.0).abs() < 1e-6);
    }
}
