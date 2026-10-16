//! The CPU coverage fallback (`vello_cpu`, amendment 10): the same pages
//! the GPU path renders, as premultiplied RGBA8 white-on-transparent, so
//! alpha is coverage. S4 measured `vello_cpu` matching Vello's GPU
//! coverage to the third decimal of SSIM.

use kurbo::{BezPath, Shape};
use vello_cpu::color::PremulRgba8;
use vello_cpu::color::palette::css::WHITE;
use vello_cpu::{Pixmap, RenderContext, Resources};

use super::atlas::Atlas;
use super::scene::{ItemContent, PathStyle, TextScene};

/// Renders page `page` of `atlas` for `scene`; returns RGBA8 rows.
pub fn render_page(scene: &TextScene, atlas: &Atlas, page: usize) -> (u32, u32, Vec<u8>) {
    let (w, h) = atlas.pages[page];
    let (w16, h16) = (w.clamp(1, u32::from(u16::MAX)) as u16, h.clamp(1, u32::from(u16::MAX)) as u16);
    let mut ctx = RenderContext::new(w16, h16);
    let mut resources = Resources::new();
    ctx.set_paint(WHITE);
    for cell in atlas.cells.iter().filter(|c| c.page == page) {
        let ItemContent::Coverage { paths, .. } = &scene.items[cell.item].content else {
            continue;
        };
        let to_page = cell.transform();
        if cell.clip {
            ctx.reset_transform();
            let clip: BezPath = cell.rect().to_path(0.1);
            ctx.push_clip_layer(&clip);
        }
        for p in paths {
            ctx.set_transform(to_page * p.transform);
            match &p.style {
                PathStyle::Fill => {
                    ctx.set_fill_rule(vello_cpu::peniko::Fill::NonZero);
                    ctx.fill_path(&p.path);
                }
                PathStyle::Stroke(stroke) => {
                    ctx.set_stroke(stroke.clone());
                    ctx.stroke_path(&p.path);
                }
            }
        }
        if cell.clip {
            ctx.pop_layer();
        }
    }
    ctx.flush();
    let mut pixmap = Pixmap::new(w16, h16);
    for p in pixmap.data_mut() {
        *p = PremulRgba8 {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        };
    }
    ctx.render(&mut pixmap, &mut resources);
    let bytes = pixmap
        .data()
        .iter()
        .flat_map(|p| [p.r, p.g, p.b, p.a])
        .collect();
    (w, h, bytes)
}
