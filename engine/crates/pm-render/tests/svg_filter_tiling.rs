//! The desktop ART Cover placement stresses the filtered float scratch pool:
//! a 230% rotated cover with a blur, group clip, and output clip at 640×360.

use pm_protocol::color::Precision;
use pm_render::vector::svg_float::SvgPainter;
use pm_vector::kurbo::{Affine, Point};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::SvgDocument;

fn art() -> &'static str {
    include_str!("../../../../apps/desktop/e2e/engine-svg.spec.ts")
        .split_once("const ART = `")
        .expect("desktop ART fixture")
        .1
        .split_once("`;")
        .expect("desktop ART fixture end")
        .0
}

#[test]
fn rotated_cover_blur_fits_the_float_budget_and_has_no_tile_seam() {
    let gpu = pm_gpu::Gpu::new("SVG Cover tile regression").unwrap();
    gpu.device.set_device_lost_callback(|reason, message| {
        eprintln!("SVG Cover device lost: {reason:?}: {message}");
    });
    let doc = SvgDocument::parse(art().as_bytes()).unwrap();
    assert!(doc.gpu_paintable());
    let world = Affine::translate((420.0, 120.0))
        * Affine::rotate(23.0_f64.to_radians())
        * Affine::scale(2.3);
    let image_width = 120.0 * 160.0 / 90.0;
    let svg_to_device = world
        * Affine::translate((-image_width / 2.0, -60.0))
        * Affine::scale_non_uniform(image_width / 160.0, 120.0 / 90.0);
    let clip: Vec<Point> = [(-60.0, -60.0), (60.0, -60.0), (60.0, 60.0), (-60.0, 60.0)]
        .into_iter()
        .map(|point| world * Point::new(point.0, point.1))
        .collect();
    let mut painter = SvgPainter::new(&gpu.device).unwrap();
    let mut render = |rect: PixelRect| {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let (texture, _) = painter
            .render(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &doc,
                svg_to_device,
                rect,
                &clip,
                Precision::Float32,
            )
            .unwrap()
            .unwrap_or_else(|| {
                panic!(
                    "the supported Cover filter stays on the float path: {:?}",
                    painter.fallback_reason()
                )
            });
        gpu.queue.submit([encoder.finish()]);
        pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &texture).unwrap()
    };
    let full = render(PixelRect::new(0, 0, 640, 360));
    let alpha_area: f64 = full.chunks_exact(4).map(|pixel| f64::from(pixel[3])).sum();
    assert!(
        alpha_area > 50_000.0,
        "Cover disappeared: alpha area {alpha_area}"
    );
    assert!(full.iter().all(|value| value.is_finite()));

    // Move the tile origin across the authored blur and cover crop. The
    // overlap must reconstruct the same float pixels even though the two
    // renders split their scratch tiles at different positions.
    let crop = PixelRect::new(377, 77, 96, 96);
    let shifted = render(crop);
    let mut max_cv = 0.0_f64;
    for y in 0..crop.height as usize {
        for x in 0..crop.width as usize {
            let full_at = ((y + crop.y as usize) * 640 + x + crop.x as usize) * 4;
            let crop_at = (y * crop.width as usize + x) * 4;
            for c in 0..4 {
                max_cv =
                    max_cv.max(f64::from((full[full_at + c] - shifted[crop_at + c]).abs()) * 255.0);
            }
        }
    }
    assert!(max_cv <= 0.5, "Cover tile seam: {max_cv:.3} CV");
}

#[test]
fn large_unfiltered_clip_gradient_tiles_in_float_without_seams() {
    let gpu = pm_gpu::Gpu::new("SVG unfiltered tile regression").unwrap();
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360">
      <defs>
        <linearGradient id="ramp" gradientUnits="userSpaceOnUse" x1="0" x2="640" y1="0" y2="0">
          <stop stop-color="#000000"/><stop offset="1" stop-color="#ffffff"/>
        </linearGradient>
        <clipPath id="window"><rect x="40.25" y="20.25" width="560.5" height="319.5"/></clipPath>
      </defs>
      <g clip-path="url(#window)"><rect width="640" height="360" fill="url(#ramp)"/></g>
    </svg>"##;
    let doc = SvgDocument::parse(svg).unwrap();
    assert!(doc.gpu_paintable());
    let mut painter = SvgPainter::new(&gpu.device).unwrap();
    let mut render = |rect: PixelRect| {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let (texture, _) = painter
            .render(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &doc,
                Affine::IDENTITY,
                rect,
                &[],
                Precision::Float32,
            )
            .unwrap()
            .expect("supported clip and gradient stay on the float path");
        gpu.queue.submit([encoder.finish()]);
        pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &texture).unwrap()
    };
    let full = render(PixelRect::new(0, 0, 640, 360));
    let at = |x: usize, y: usize, channel: usize| full[(y * 640 + x) * 4 + channel] as f64;
    // Independent SVG user-space linear-gradient value at the pixel centre.
    let wanted = 123.5 / 640.0;
    assert!((at(123, 100, 0) - wanted).abs() * 255.0 < 0.05);
    assert!((at(123, 100, 0) * 255.0 - (wanted * 255.0).round()).abs() > 0.15);
    // A quarter-pixel clip edge must survive the 4× joint group resolve.
    assert!((at(40, 100, 3) - 0.75).abs() * 255.0 < 0.1);

    let crop = PixelRect::new(277, 77, 96, 96);
    let shifted = render(crop);
    let mut max_cv = 0.0_f64;
    for y in 0..crop.height as usize {
        for x in 0..crop.width as usize {
            let full_at = ((y + crop.y as usize) * 640 + x + crop.x as usize) * 4;
            let crop_at = (y * crop.width as usize + x) * 4;
            for c in 0..4 {
                max_cv =
                    max_cv.max(f64::from((full[full_at + c] - shifted[crop_at + c]).abs()) * 255.0);
            }
        }
    }
    assert!(
        max_cv <= 0.125,
        "unfiltered float tile seam: {max_cv:.4} CV"
    );
}
