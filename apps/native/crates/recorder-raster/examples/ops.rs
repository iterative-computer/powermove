//! Native-only per-operation microbenchmark. No JS, GPU upload or presentation.
use pm_recorder_raster::{AlphaMask, Rasterizer, Shadow, recorder::*};
use std::{hint::black_box, time::Instant};
#[path = "../tests/scenes.rs"]
mod scenes;
fn measure(name: &str, mut op: impl FnMut()) {
    for _ in 0..20 {
        op();
    }
    let mut samples = Vec::new();
    for _ in 0..101 {
        let start = Instant::now();
        op();
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    println!("{name},{},{}", samples[50], samples[95]);
}
fn main() {
    println!("operation,median_ns,p95_ns");
    for name in scenes::NAMES {
        let c = scenes::record(name);
        let (w, h) = c.size();
        let mut r = Rasterizer::new(7, w, h).unwrap();
        // Full named scene is one op (command count printed in the docs),
        // surface is retained across samples; output is measured separately.
        measure(name, || {
            let k = r.key();
            black_box(r.replay(k, black_box(c.commands())).unwrap());
        });
    }
    let mut c = Canvas::new(7, 16, 16).unwrap();
    c.fill_rect(0., 0., 16., 16.).unwrap();
    let mut r = Rasterizer::new(7, 16, 16).unwrap();
    measure("fill_rect_16x16", || {
        black_box(r.replay(r.key(), c.commands()).unwrap());
    });
    let mut c = Canvas::new(7, 1920, 1080).unwrap();
    c.fill_rect(0., 0., 1920., 1080.).unwrap();
    let mut r = Rasterizer::new(7, 1920, 1080).unwrap();
    r.replay(r.key(), c.commands()).unwrap();
    measure("readback_1080p", || {
        black_box(r.image_data().unwrap());
    });
    measure("readback_1px_from_1080p", || {
        black_box(r.get_image_data(0, 0, 1, 1).unwrap());
    });
    let data = ImageData::from_rgba(1920, 1080, vec![128; 1920 * 1080 * 4]).unwrap();
    let command = Command::Put {
        image: std::sync::Arc::new(data),
        dx: 0,
        dy: 0,
        dirty: [0, 0, 16, 16],
    };
    measure("put_dirty_16x16", || {
        black_box(r.replay(r.key(), std::slice::from_ref(&command)).unwrap());
    });
    let mut copy = Canvas::new(7, 1920, 1080).unwrap();
    copy.set_composite("copy");
    copy.draw_image(
        Source {
            key: SourceKey {
                id: 123,
                version: 1,
            },
            pixels: std::sync::Arc::new(
                ImageData::from_rgba(1920, 1080, vec![128; 1920 * 1080 * 4]).unwrap(),
            ),
        },
        Rect::new(0., 0., 1920., 1080.),
        Rect::new(0., 0., 1920., 1080.),
    )
    .unwrap();
    measure("draw_image_copy_1080p", || {
        black_box(r.replay(r.key(), copy.commands()).unwrap());
    });
    measure("readback_copy_base_1080p", || {
        black_box(r.image_data().unwrap());
    });
    let mask = AlphaMask::new(16, 16, [8, 8], vec![1.; 256]).unwrap();
    measure("gaussian_mask_16x16_blur4", || {
        black_box(mask.blurred(4.).unwrap());
    });
    let mut r = Rasterizer::new(8, 48, 48).unwrap();
    measure("shadow_16x16_blur4", || {
        black_box(
            r.draw_shadow(
                r.key(),
                &mask,
                Shadow {
                    color: Color::BLACK,
                    offset: [2., 2.],
                    blur: 4.,
                },
                "source-over",
                &[],
            )
            .unwrap(),
        );
    });
}
