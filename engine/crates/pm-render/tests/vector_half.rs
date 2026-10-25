//! Half-float stores of vector, path-mask and SVG paint hold the nearest
//! half of each computed value (`src/nearest_half.rs`). Metal's
//! render-target conversion truncates, so each value here is one that
//! truncation would store an ulp low.

use pm_protocol::color::Precision;
use pm_render::composite::path_masks::{PathCoverage, coverage_values};
use pm_render::vector::VectorSource;
use pm_render::vector::cache::VectorCache;
use pm_render::vector::svg_float::SvgPainter;
use pm_vector::kurbo::{Affine, Rect, Shape};
use pm_vector::render::scene::{CoverageItem, CoverageScene, FillRule, ItemStyle, PixelRect};
use pm_vector::render::svg::SvgDocument;
use pm_vector::{LayerMasks, MaskValues, PathValues, Vertex};

fn nearest(v: f32) -> f32 {
    pm_gpu::readback::half_to_f32(pm_gpu::readback::f32_to_half(v))
}

/// The half just below `v`'s nearest: what truncation stores when `v`
/// rounds up.
fn truncated(v: f32) -> f32 {
    let h = pm_gpu::readback::f32_to_half(v);
    pm_gpu::readback::half_to_f32(h - 1)
}

const GRAY: f32 = 9.0 / 255.0;

#[test]
fn fixture_values_round_up() {
    // Both values lie above their halfway point, so a truncating store
    // would differ from the nearest half.
    for v in [GRAY, 0.3] {
        assert!(nearest(v) > v && truncated(v) < v, "{v}");
    }
}

#[test]
fn vector_paint_stores_the_nearest_half() {
    let gpu = pm_gpu::Gpu::new("vector half").unwrap();
    let scene = CoverageScene {
        items: vec![CoverageItem {
            path: Rect::new(0.0, 0.0, 4.0, 4.0).to_path(1e-9),
            transform: Affine::IDENTITY,
            style: ItemStyle::Fill(FillRule::NonZero),
            color: [GRAY, GRAY, GRAY, 1.0],
        }],
    };
    let mut cache = VectorCache::new();
    let view = cache
        .resolve(
            &gpu.device,
            &gpu.queue,
            &VectorSource::Scene {
                scene,
                rect: PixelRect::new(0, 0, 4, 4),
            },
            Precision::Float16,
        )
        .unwrap();
    let texture = view.texture();
    assert_eq!(texture.format(), wgpu::TextureFormat::Rgba16Float);
    let pixels = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, texture).unwrap();
    assert_eq!(pixels[(4 + 1) * 4], nearest(GRAY));
}

#[test]
fn svg_float_output_stores_the_nearest_half() {
    let gpu = pm_gpu::Gpu::new("SVG half").unwrap();
    // A solid gray, and a white edge covering 0.3 of its pixel.
    let doc = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><rect width="4" height="4" fill="#090909"/><rect x="4" width="0.3" height="4" fill="#fff"/></svg>"##,
    )
    .unwrap();
    let mut painter = SvgPainter::new(&gpu.device).unwrap();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let (texture, _) = painter
        .render(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &doc,
            Affine::IDENTITY,
            PixelRect::new(0, 0, 8, 4),
            &[],
            Precision::Float16,
        )
        .unwrap()
        .expect("float path");
    gpu.queue.submit([encoder.finish()]);
    assert_eq!(texture.format(), wgpu::TextureFormat::Rgba16Float);
    let pixels = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &texture).unwrap();
    assert_eq!(pixels[(8 + 1) * 4], nearest(GRAY));
    assert_eq!(pixels[(8 + 4) * 4 + 3], nearest(0.3));
}

#[test]
fn path_mask_coverage_stores_the_nearest_half() {
    let gpu = pm_gpu::Gpu::new("path mask half").unwrap();
    // A closed path covering 0.3 of the first pixel column.
    let vertex = |x: f64, y: f64| Vertex {
        x,
        y,
        in_x: 0.0,
        in_y: 0.0,
        out_x: 0.0,
        out_y: 0.0,
    };
    let mask = MaskValues {
        path: Some(PathValues {
            vertices: vec![
                vertex(0.0, 0.0),
                vertex(0.3, 0.0),
                vertex(0.3, 4.0),
                vertex(0.0, 4.0),
            ],
            ..PathValues::default()
        }),
        ..MaskValues::default()
    };
    let LayerMasks::Path(masks) = LayerMasks::new(&[mask]) else {
        panic!("a path mask")
    };
    let values = coverage_values(
        &gpu.device,
        &gpu.queue,
        &PathCoverage {
            width: 4,
            height: 4,
            world: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            masks,
        },
    )
    .unwrap();
    assert_eq!(values[4], nearest(0.3), "{values:?}");
}
