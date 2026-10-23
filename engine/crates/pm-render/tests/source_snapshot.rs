//! Source frames through the same decoder and source pass as layers.
#![cfg(target_os = "macos")]

use pm_protocol::common::{Size, Ticks};
use pm_protocol::snapshot::SourceFit;
use pm_render::Renderer;

#[test]
fn source_stretch_and_contain_preserve_rgba_and_transparent_bars() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.png");
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let file = std::fs::File::create(&path).unwrap();
    let mut png = png::Encoder::new(file, 2, 2);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let stretch = renderer
        .source_rgba8(
            &path,
            Ticks::ZERO,
            Size::new(2, 2),
            SourceFit::Stretch,
            None,
        )
        .unwrap();
    assert_eq!(stretch, pixels);
    let contain = renderer
        .source_rgba8(
            &path,
            Ticks::ZERO,
            Size::new(4, 2),
            SourceFit::Contain,
            None,
        )
        .unwrap();
    for y in 0..2 {
        assert_eq!(&contain[(y * 4) * 4..(y * 4 + 1) * 4], &[0, 0, 0, 0]);
        assert_eq!(
            &contain[(y * 4 + 1) * 4..(y * 4 + 3) * 4],
            &pixels[(y * 2) * 4..(y * 2 + 2) * 4]
        );
        assert_eq!(&contain[(y * 4 + 3) * 4..(y * 4 + 4) * 4], &[0, 0, 0, 0]);
    }
}

#[test]
fn source_snapshot_reads_premultiplied_alpha_for_snapshot_pixel_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alpha.png");
    let file = std::fs::File::create(&path).unwrap();
    let mut png = png::Encoder::new(file, 1, 1);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(&[255, 0, 0, 128])
        .unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let rgba = renderer
        .source_rgba8(
            &path,
            Ticks::ZERO,
            Size::new(1, 1),
            SourceFit::Stretch,
            None,
        )
        .unwrap();
    assert_eq!(rgba, [128, 0, 0, 128]);
}

#[test]
fn source_svg_uses_exact_size_chromium_raster() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.svg");
    std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="red"/></svg>"#).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let error = renderer
        .source_rgba8(
            &path,
            Ticks::ZERO,
            Size::new(2, 2),
            SourceFit::Stretch,
            None,
        )
        .unwrap_err();
    assert!(error.contains("Chromium raster"), "{error}");
    let output = Size::new(3, 2);
    let request =
        Renderer::source_svg_request(&path, "asset-1", output, SourceFit::Stretch, (2.0, 2.0))
            .unwrap();
    assert_eq!((request.key.width, request.key.height), (3, 2));
    assert_eq!(request.key.source_fit, Some(SourceFit::Stretch));
    assert_eq!(request.asset, "asset-1");
    assert!(
        renderer
            .source_svg_rgba8(&request.key, output, SourceFit::Stretch, None)
            .unwrap()
            .is_none()
    );
    let chromium = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 0, 255, 255, 255, 255, 0,
        255, 255,
    ];
    renderer
        .put_svg_raster(request.key.clone(), chromium.clone())
        .unwrap();
    assert_eq!(
        renderer
            .source_svg_rgba8(&request.key, output, SourceFit::Stretch, None)
            .unwrap()
            .unwrap(),
        chromium
    );
    let contain = Renderer::source_svg_request(
        &path,
        "asset-1",
        Size::new(5, 5),
        SourceFit::Contain,
        (2.0, 1.0),
    )
    .unwrap();
    assert_eq!((contain.key.width, contain.key.height), (5, 5));
    assert_eq!(contain.key.source_fit, Some(SourceFit::Contain));
    assert_ne!(contain.key.upload_key(), request.key.upload_key());
}

#[test]
fn source_svg_rejects_an_interpreted_color_override() {
    let mut renderer = Renderer::headless().unwrap();
    let key = pm_protocol::svg_raster::SvgRasterKey {
        content: "a".repeat(64),
        texture: None,
        source_fit: Some(SourceFit::Stretch),
        width: 1,
        height: 1,
    };
    let interpretation = pm_media::interpret::Interpretation {
        transfer: Some(pm_protocol::color::Transfer::Linear),
        ..Default::default()
    };
    let error = renderer
        .source_svg_rgba8(
            &key,
            Size::new(1, 1),
            SourceFit::Stretch,
            Some(&interpretation),
        )
        .unwrap_err();
    assert!(
        error.contains("interpretation cannot be applied"),
        "{error}"
    );
}

#[test]
fn untagged_vp9_browser_rendition_uses_chromiums_i420_bitmap() {
    // The public ProRes fixture is solid red. Today's Canvas2D view of its
    // VP9 browser rendition is (254, 0, 0, 255), while a direct floating YUV
    // conversion clips red to 255. Use the same converter flags as import.
    let dir = tempfile::tempdir().unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures/prores422.mov");
    let proxy = dir.path().join("browser.webm");
    let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .expect("source engine/env.sh")
        .join("bin/ffmpeg");
    let status = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(&source)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c:v",
            "libvpx-vp9",
            "-pix_fmt",
            "yuva420p",
            "-b:v",
            "0",
            "-crf",
            "18",
            "-g",
            "15",
            "-deadline",
            "good",
            "-cpu-used",
            "4",
            "-row-mt",
            "1",
            "-threads",
            "4",
            "-c:a",
            "libopus",
            "-b:a",
            "192k",
            "-f",
            "webm",
            "-y",
        ])
        .arg(&proxy)
        .status()
        .unwrap();
    assert!(status.success());
    let pixels = Renderer::headless()
        .unwrap()
        .source_rgba8(
            &proxy,
            Ticks::ZERO,
            Size::new(160, 90),
            SourceFit::Stretch,
            None,
        )
        .unwrap();
    assert_eq!(pixels.len(), 160 * 90 * 4);
    assert!(pixels.chunks_exact(4).all(|p| p == [254, 0, 0, 255]));
}

#[test]
fn source_scrub_reuses_decoded_video_and_counts_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.webm");
    let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .expect("source engine/env.sh")
        .join("bin/ffmpeg");
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=30:duration=1",
            "-an",
            "-c:v",
            "libvpx-vp9",
            "-b:v",
            "0",
            "-crf",
            "18",
            "-threads",
            "2",
            "-f",
            "webm",
            "-y",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let mut renderer = Renderer::headless().unwrap();
    let size = Size::new(64, 48);
    let times = [0, 7, 7, 7, 14, 14];
    let scope = pm_perf::Scope::begin();
    let began = std::time::Instant::now();
    let mut frames = Vec::new();
    for frame in times {
        let ticks = Ticks::new(frame * pm_protocol::common::TICKS_PER_SECOND / 30).unwrap();
        frames.push(
            renderer
                .source_rgba8(&path, ticks, size, SourceFit::Stretch, None)
                .unwrap(),
        );
    }
    let elapsed = began.elapsed();
    let counters = scope.finish();
    assert_eq!(frames[1], frames[2]);
    assert_eq!(frames[1], frames[3]);
    assert_eq!(frames[4], frames[5]);
    assert!(
        counters.get(pm_perf::Counter::MediaCacheHits) >= 3,
        "repeated seeks should reuse converted frames: {}",
        counters.to_json()
    );
    assert!(counters.get(pm_perf::Counter::DrawCalls) >= times.len() as u64);
    assert!(counters.get(pm_perf::Counter::Readbacks) >= times.len() as u64);
    let end = renderer
        .source_rgba8(
            &path,
            Ticks::from_whole_seconds(1),
            size,
            SourceFit::Stretch,
            None,
        )
        .unwrap();
    let last = renderer
        .source_rgba8(
            &path,
            Ticks::new(29 * pm_protocol::common::TICKS_PER_SECOND / 30).unwrap(),
            size,
            SourceFit::Stretch,
            None,
        )
        .unwrap();
    assert_eq!(end, last, "an End seek holds the last presented picture");
    eprintln!(
        "source scrub: 6 frames in {:.3} ms; {}",
        elapsed.as_secs_f64() * 1000.0,
        counters.to_json()
    );
}
