//! Device safety and visible pixels for ordinary and filtered SVG image boxes.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use pm_protocol::color::Precision;
use pm_render::vector::svg_float::SvgPainter;
use pm_render::vector::{VectorSource, cache::VectorCache, svg, svg_sample};
use pm_vector::kurbo::{Affine, Point};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::SvgDocument;

const RECT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="320" height="180" fill="#5A93F2"/></svg>"##;
const ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90" viewBox="0 0 160 90">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#4C8DFF"/><stop offset="1" stop-color="#E83A5E" stop-opacity=".6"/></linearGradient>
<clipPath id="c"><circle cx="120" cy="45" r="34"/></clipPath><filter id="b"><feGaussianBlur stdDeviation="2.5"/></filter></defs>
<rect x="4" y="4" width="152" height="82" rx="14" fill="url(#g)"/>
<g clip-path="url(#c)"><rect x="80" y="0" width="80" height="90" fill="#FFD23F"/><path d="M80 90 L160 0" stroke="#14161C" stroke-width="6"/></g>
<path fill-rule="evenodd" d="M20 20 h50 v50 h-50 z M32 32 h26 v26 h-26 z" fill="#FFFFFF" filter="url(#b)"/>
<path d="M10 80 Q 60 40 110 80" fill="none" stroke="#7CFFB2" stroke-width="3.5" stroke-dasharray="7 4" stroke-linecap="round"/>
</svg>"##;

fn probe(svg: &str, scale: f64, width: u32, height: u32, box_clip: bool) -> (bool, Option<String>) {
    let gpu = pm_gpu::Gpu::new("SVG device limit test").unwrap();
    let lost = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&lost);
    gpu.device
        .set_device_lost_callback(move |_, _| flag.store(true, Ordering::SeqCst));
    let doc = SvgDocument::parse(svg.as_bytes()).unwrap();
    let clip = if box_clip {
        vec![
            Point::new(0.0, 0.0),
            Point::new(f64::from(width), 0.0),
            Point::new(f64::from(width), f64::from(height)),
            Point::new(0.0, f64::from(height)),
        ]
    } else {
        Vec::new()
    };
    let mut painter = SvgPainter::new(&gpu.device).unwrap();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let result = painter
        .render(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &doc,
            Affine::scale(scale),
            PixelRect::new(0, 0, width, height),
            &clip,
            Precision::Float16,
        )
        .unwrap();
    let reason = painter.fallback_reason().map(str::to_owned);
    if let Some((texture, _)) = result {
        gpu.queue.submit([encoder.finish()]);
        let pixels = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &texture).unwrap();
        assert_eq!(pixels.len(), width as usize * height as usize * 4);
        assert!(pixels.iter().all(|v| v.is_finite()));
        if box_clip {
            let center = ((height / 2 * width + width / 2) * 4) as usize;
            assert!(pixels[center + 3] > 0.1, "SVG disappeared");
        }
    } else {
        assert!(reason.is_some(), "fallback must have a runtime reason");
        // The same document must remain renderable by the declared fallback.
        let fallback = doc.raster_device(
            Affine::scale(scale),
            PixelRect::new(0, 0, width, height),
            box_clip.then_some(clip.as_slice()),
        );
        assert_eq!(fallback.rgba.len(), width as usize * height as usize * 4);
    }
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    (!lost.load(Ordering::SeqCst), reason)
}

#[test]
fn ordinary_box_clips_keep_the_device_and_pixels_at_hd_to_4k() {
    for (scale, width, height) in [(4.5, 1440, 810), (6.0, 1920, 1080), (12.0, 3840, 2160)] {
        let (alive, reason) = probe(RECT, scale, width, height, true);
        assert!(alive, "GPU lost at {width}×{height}");
        assert!(reason.is_none(), "plain SVG used fallback: {reason:?}");
    }
}

#[test]
fn art_at_640x360_uses_native_float_without_device_loss() {
    let gpu = pm_gpu::Gpu::new("SVG ART 640x360 export test").unwrap();
    let lost = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&lost);
    gpu.device
        .set_device_lost_callback(move |_, _| flag.store(true, Ordering::SeqCst));
    let doc = SvgDocument::parse(ART.as_bytes()).unwrap();
    let mut painter = SvgPainter::new(&gpu.device).unwrap();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let clip = [
        Point::new(0.0, 0.0),
        Point::new(640.0, 0.0),
        Point::new(640.0, 360.0),
        Point::new(0.0, 360.0),
    ];
    let (texture, _) = painter
        .render(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &doc,
            Affine::translate((110.0, 80.0))
                * Affine::rotate(23.0_f64.to_radians())
                * Affine::scale(2.3),
            PixelRect::new(0, 0, 640, 360),
            &clip,
            Precision::Float32,
        )
        .unwrap()
        .unwrap_or_else(|| {
            panic!(
                "ART unexpectedly used resvg: {:?}",
                painter.fallback_reason()
            )
        });
    gpu.queue.submit([encoder.finish()]);
    let pixels = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &texture).unwrap();
    assert_eq!(pixels.len(), 640 * 360 * 4);
    assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] > 0.5));
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    assert!(!lost.load(Ordering::SeqCst));
}

#[test]
fn blur_budget_uses_a_named_fallback_without_losing_the_device() {
    for (deviation, scale, width, height) in [
        (1.0, 1.0, 960, 540),
        (2.0, 4.0 / 3.0, 1280, 720),
        (0.5, 2.0, 1920, 1080),
        (3.0, 6.0, 1920, 1080),
    ] {
        eprintln!("SVG blur probe: deviation={deviation}, scale={scale}, {width}×{height}");
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="960" height="540"><defs><filter id="b" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="{deviation}"/></filter></defs><circle cx="480" cy="270" r="150" fill="#3366ff" filter="url(#b)"/></svg>"##
        );
        let (alive, reason) = probe(&svg, scale, width, height, false);
        assert!(alive, "GPU lost with blur {deviation} at {width}×{height}");
        if let Some(reason) = reason {
            assert!(
                reason.contains("budget") || reason.contains("limit"),
                "unexpected fallback: {reason}"
            );
        }
    }
    let onscreen = r##"<svg xmlns="http://www.w3.org/2000/svg" width="960" height="540"><defs><filter id="b" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="3"/></filter></defs><circle cx="160" cy="90" r="150" fill="#3366ff" filter="url(#b)"/></svg>"##;
    let (alive, reason) = probe(onscreen, 6.0, 1920, 1080, false);
    assert!(alive, "GPU lost on visible moderate blur");
    eprintln!("visible over-budget blur fallback: {reason:?}");
    assert!(
        reason
            .as_deref()
            .is_some_and(|value| value.contains("budget")),
        "visible over-budget blur must use named fallback: {reason:?}"
    );
}

#[test]
fn sequential_large_filtered_exports_keep_the_device_and_pixels() {
    let gpu = pm_gpu::Gpu::new("SVG sequential filtered export test").unwrap();
    let lost = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&lost);
    gpu.device
        .set_device_lost_callback(move |_, _| flag.store(true, Ordering::SeqCst));
    let doc = SvgDocument::parse(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="288"><defs><filter id="blur"><feGaussianBlur stdDeviation="1"/></filter></defs><rect width="512" height="288" fill="#538fce" filter="url(#blur)"/></svg>"##.as_bytes(),
    )
    .unwrap();
    let mut painter = SvgPainter::new(&gpu.device).unwrap();
    for frame in 0..3 {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let started = Instant::now();
        let result = painter
            .render(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &doc,
                Affine::IDENTITY,
                PixelRect::new(0, 0, 512, 288),
                &[],
                Precision::Float32,
            )
            .unwrap();
        if let Some((texture, _)) = result {
            gpu.queue.submit([encoder.finish()]);
            let pixels = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &texture).unwrap();
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] > 0.5));
        } else {
            let reason = painter.fallback_reason().expect("filtered fallback reason");
            assert!(reason.contains("budget") || reason.contains("limit"));
            let raster = doc.raster_device(
                Affine::IDENTITY,
                PixelRect::new(0, 0, 512, 288),
                None,
            );
            assert_eq!(raster.rgba.len(), 512 * 288 * 4);
        }
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        eprintln!("SVG sequential filtered export {frame}: {:.3} ms (GPU complete)", started.elapsed().as_secs_f64() * 1_000.0);
        assert!(!lost.load(Ordering::SeqCst), "GPU lost after filtered export {frame}");
    }
}

fn svg_file(dir: &tempfile::TempDir, name: &str, svg: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, svg).unwrap();
    path
}

/// Resolves one frame of `path` (user box `width` × `height`) under
/// `transform` in a 256 × 144 target.
fn frame(
    gpu: &pm_gpu::Gpu,
    cache: &mut VectorCache,
    path: &std::path::Path,
    (width, height): (f64, f64),
    transform: Affine,
    playback: bool,
) {
    cache.begin_frame();
    let clip = [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)].map(|(x, y)| {
        let p = transform * Point::new(x, y);
        [p.x, p.y]
    });
    cache
        .resolve(
            &gpu.device,
            &gpu.queue,
            &VectorSource::SvgRaster {
                path: path.to_path_buf(),
                stamp: svg::stamp(path).unwrap(),
                transform: transform.as_coeffs(),
                rect: PixelRect::new(0, 0, 256, 144),
                clip,
                playback,
            },
            Precision::Float16,
        )
        .unwrap();
    assert!(cache.svg_fallback_reason().is_none(), "{:?}", cache.svg_fallback_reason());
}

#[test]
fn playback_reuses_the_source_raster_for_translation_and_rotation_but_not_scale() {
    let gpu = pm_gpu::Gpu::new("SVG transform cache test").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = svg_file(
        &dir,
        "animated.svg",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60"><rect width="100" height="60" fill="#537eb2"/></svg>"##,
    );
    let mut cache = VectorCache::new();
    for (x, angle, scale, expected) in [
        (50.0, 0.0, 1.0, (0, 1)),
        (70.0, 0.0, 1.0, (1, 1)),
        (90.0, 0.4, 1.0, (2, 1)),
        (90.0, 0.4, 2.0, (2, 2)),
    ] {
        let t = Affine::translate((x, 80.0)) * Affine::rotate(angle) * Affine::scale(scale);
        frame(&gpu, &mut cache, &path, (100.0, 60.0), t, true);
        assert_eq!(cache.svg_raster_cache_counts(), expected);
    }
    // Paused frames and exports paint at output resolution.
    let t = Affine::translate((110.0, 80.0)) * Affine::rotate(0.4);
    frame(&gpu, &mut cache, &path, (100.0, 60.0), t, false);
    assert_eq!(cache.svg_raster_cache_counts(), (2, 2));
}

#[test]
fn playback_reuses_clipped_text_and_filtered_sources() {
    let gpu = pm_gpu::Gpu::new("SVG playback source reuse").unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (name, svg, size, scale, angle) in [
        (
            "clipped",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="45"><defs><clipPath id="c"><circle cx="40" cy="22" r="20"/></clipPath></defs><rect width="80" height="45" fill="#538fce" clip-path="url(#c)"/></svg>"##,
            (80.0, 45.0),
            2.0,
            0.3,
        ),
        (
            "text",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="12"><text x="1" y="10" font-family="Helvetica" font-size="10" fill="#4C8DFF">Text</text></svg>"##,
            (32.0, 12.0),
            2.7,
            std::f64::consts::FRAC_PI_4,
        ),
        (
            "filtered",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="14"><defs><filter id="blur"><feGaussianBlur stdDeviation="1"/></filter></defs><circle cx="12" cy="7" r="5" fill="#538fce" filter="url(#blur)"/></svg>"##,
            (24.0, 14.0),
            1.5,
            0.0,
        ),
    ] {
        let path = svg_file(&dir, &format!("{name}.svg"), svg);
        let mut cache = VectorCache::new();
        for step in 0..3u32 {
            let t = Affine::translate((60.0 + 3.25 * f64::from(step), 30.0))
                * Affine::rotate(angle + 0.1 * f64::from(step))
                * Affine::scale(scale);
            frame(&gpu, &mut cache, &path, size, t, true);
            assert_eq!(cache.svg_raster_cache_counts(), (u64::from(step), 1), "{name}");
        }
    }
}

#[test]
fn large_playback_layers_paint_directly() {
    let gpu = pm_gpu::Gpu::new("SVG large playback layer").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = svg_file(&dir, "large.svg", RECT);
    let doc = SvgDocument::parse(RECT.as_bytes()).unwrap();
    let t = Affine::scale(12.0);
    let clip = [(0.0, 0.0), (320.0, 0.0), (320.0, 180.0), (0.0, 180.0)].map(|(x, y)| t * Point::new(x, y));
    let plan = svg_sample::plan_with_clip(&doc, &path, svg::stamp(&path).unwrap(), t, &clip, &gpu.device.limits());
    assert!(plan.is_none(), "a 4K-sized source exceeds the playback raster cap");
    let mut cache = VectorCache::new();
    frame(&gpu, &mut cache, &path, (320.0, 180.0), t, true);
    assert_eq!(cache.svg_raster_cache_counts(), (0, 0));
}

#[test]
fn cached_resvg_result_restores_its_fallback_reason() {
    let gpu = pm_gpu::Gpu::new("SVG fallback cache reason test").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let unsupported = dir.path().join("pattern.svg");
    std::fs::write(
        &unsupported,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><defs><pattern id="p" width="4" height="4" patternUnits="userSpaceOnUse"><rect width="2" height="2" fill="red"/></pattern></defs><rect width="32" height="32" fill="url(#p)"/></svg>"##,
    )
    .unwrap();
    let ordinary = dir.path().join("ordinary.svg");
    std::fs::write(&ordinary, RECT).unwrap();
    let source = |path: &std::path::Path, width: f64, height: f64| VectorSource::SvgRaster {
        path: path.to_path_buf(),
        stamp: svg::stamp(path).unwrap(),
        transform: Affine::IDENTITY.as_coeffs(),
        rect: PixelRect::new(0, 0, 32, 32),
        clip: [[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]],
        playback: false,
    };
    let pattern = source(&unsupported, 32.0, 32.0);
    let plain = source(&ordinary, 320.0, 180.0);
    assert!(!svg::document(&unsupported).unwrap().0.gpu_paintable());
    let mut cache = VectorCache::new();
    cache_resolve(&gpu, &mut cache, &pattern);
    let reason = cache.svg_fallback_reason().unwrap().to_owned();
    assert!(reason.contains("resvg"));
    cache_resolve(&gpu, &mut cache, &plain);
    assert!(cache.svg_fallback_reason().is_none());
    cache_resolve(&gpu, &mut cache, &pattern);
    assert_eq!(cache.svg_fallback_reason(), Some(reason.as_str()));
}

fn cache_resolve(gpu: &pm_gpu::Gpu, cache: &mut VectorCache, source: &VectorSource) {
    cache
        .resolve(&gpu.device, &gpu.queue, source, Precision::Float32)
        .unwrap();
}
