//! Synthetic native portion of EXT-RASTER-1. Release builds only for timing.
//! No V8 is linked: JS and whole production path stay explicitly unmeasured.
use pm_recorder::*;
use std::{hint::black_box, time::Instant};
fn p95(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[(v.len() * 95).div_ceil(100) - 1]
}
fn main() -> Result<()> {
    let mut frame = Canvas::new(1, 1920, 1080)?;
    let bytes: Vec<u8> = (0..1920 * 1080)
        .flat_map(|i| [(i % 251) as u8, ((i / 1920) % 251) as u8, 37, 255])
        .collect();
    let source = Sources::default().insert(
        SourceKey { id: 2, version: 1 },
        ImageData::from_rgba(1920, 1080, bytes)?,
    )?;
    let bounds = Rect::new(0., 0., 1920., 1080.);
    let mut layers: Vec<_> = (0..4)
        .map(|id| Canvas::new(10 + id, 16, 16))
        .collect::<Result<_>>()?;
    let mut record = Vec::new();
    let mut total = Vec::new();
    for iteration in 0..130 {
        let start = Instant::now();
        frame.set_composite("copy");
        frame.draw_image(source.clone(), bounds, bounds)?;
        frame.set_composite("source-over");
        for (i, layer) in layers.iter_mut().enumerate() {
            let t = Instant::now();
            layer
                .set_fill_color(if iteration % 2 == 0 {
                    "#4070c080"
                } else {
                    "#d0b020a0"
                })
                .unwrap();
            layer.clear_rect(0., 0., 16., 16.)?;
            layer.fill_rect(0.25, 0.25, 15.5, 15.5)?;
            layer.begin_path();
            layer.move_to(0., 0.);
            layer.bezier_curve_to(
                Point::new(16., 0.),
                Point::new(0., 16.),
                Point::new(16., 16.),
            );
            layer.line_to(0., 16.);
            layer.fill(FillRule::NonZero)?;
            if iteration >= 30 {
                record.push(t.elapsed().as_secs_f64() * 1000.);
            }
            let image = layer.snapshot()?;
            frame.draw_image(
                image,
                Rect::new(0., 0., 16., 16.),
                Rect::new((i * 20) as f64, 0., (i * 20 + 16) as f64, 16.),
            )?;
            layer.compact()?;
        }
        frame.flush()?;
        // Include unorm8 output; no PNG/JPEG, GPU upload, runtime or presenter.
        black_box(frame.snapshot()?);
        frame.compact()?;
        if iteration >= 30 {
            total.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    let record = p95(&mut record);
    let native = p95(&mut total);
    println!(
        "{{\"workload\":\"EXT-RASTER-1-native-synthetic\",\"frame_samples\":100,\"layer_samples\":400,\"rust_record_p95_ms\":{record:.6},\"native_with_readback_p95_ms\":{native:.6},\"native_within_4ms\":{},\"js_p95_ms\":null,\"whole_production_path_p95_ms\":null}}",
        native <= 4.
    );
    Ok(())
}
