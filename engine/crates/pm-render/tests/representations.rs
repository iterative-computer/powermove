//! The renderer can alternate preview and export representations in one
//! process without handing reduced pixels to an export.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::{PixelRect, Size};
use pm_protocol::frame::Representation;
use pm_render::sampling::Sampling;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::{asset, layer, load, no_fill, project};
use support::ticks;

#[test]
fn visible_image_tiles_match_whole_image_across_seams_and_obey_lease() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.png");
    let (w, h) = (1024u32, 768u32);
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.extend_from_slice(&[
                ((x * 13 + y * 3) % 256) as u8,
                ((x * 7 + y * 19) % 256) as u8,
                ((x ^ y) % 256) as u8,
                (100 + (x + y) % 156) as u8,
            ]);
        }
    }
    let file = std::fs::File::create(&path).unwrap();
    let mut png = png::Encoder::new(file, w, h);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header().unwrap().write_image_data(&pixels).unwrap();
    let mut image = layer("img", "image", json!({ "asset": "large", "fit": "stretch", "w": w, "h": h }),
        f64::from(w) / 2.0, f64::from(h) / 2.0, 1.0);
    image["compat"] = json!(2);
    let mut doc = project(w, h, 1.0, no_fill(), vec![image]);
    asset(&mut doc, "large", "image", "png", w, h, 0.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("large".into(), path);
    let doc = load(&doc);
    let mut renderer = Renderer::headless().unwrap();
    let lease = pm_media::decode::service::MediaLease { frame_cache: 32 << 20, decoders: usize::MAX };
    renderer.apply_media_lease(lease);
    let mut request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16);
    request.sampling = Sampling::Playback;
    let region = PixelRect { x: 238, y: 190, width: 340, height: 270 };
    request.region = Some(region);
    let cropped = renderer.render(&doc, "Ptest-root", ticks(0.0), &request, &resolver).unwrap();
    assert!(cropped.failures.is_empty(), "{:?}", cropped.failures);
    let tiles = renderer.read_rgba(&cropped).unwrap();
    let usage = renderer.media_usage();
    assert!(usage.frame_cache > 0, "the cropped view must populate visible tiles");
    assert!(usage.frame_cache <= lease.frame_cache, "tile cache exceeded its lease");
    request.region = None;
    request.export = true; // The source control cannot use viewer tiles.
    let whole = renderer.render(&doc, "Ptest-root", ticks(0.0), &request, &resolver).unwrap();
    assert!(whole.failures.is_empty(), "{:?}", whole.failures);
    let all = renderer.read_rgba(&whole).unwrap();
    let mut max_error = 0.0f32;
    for y in 0..region.height as usize {
        for x in 0..region.width as usize {
            for c in 0..4 {
                let crop_i = (y * region.width as usize + x) * 4 + c;
                let full_i = ((y + region.y as usize) * w as usize + x + region.x as usize) * 4 + c;
                max_error = max_error.max((tiles[crop_i] - all[full_i]).abs());
            }
        }
    }
    assert!(max_error <= 1.0 / 255.0 + 1e-6, "tile seam/color delta {max_error}");
    // The same cropped view chooses an ImageIO mip when the viewer asks for
    // a smaller representation. It must still use visible tiles and report
    // the pixels it actually drew, never the export's original frame.
    renderer.set_preview_decode_size(Some((512, 384)));
    request.export = false;
    request.region = Some(region);
    let reduced = renderer.render(&doc, "Ptest-root", ticks(0.0), &request, &resolver).unwrap();
    assert!(reduced.failures.is_empty(), "{:?}", reduced.failures);
    assert_eq!(renderer.media_representation(), Representation::ReducedResolution);
    let mip = renderer.read_rgba(&reduced).unwrap();
    assert_ne!(mip, tiles, "the reduced representation must supply the drawn tiles");
    assert!(renderer.media_usage().frame_cache <= lease.frame_cache);
    // A whole-image converted texture also belongs to the lease, even when
    // no decoded-video frame or tile occupies the service cache.
    let mut whole_renderer = Renderer::headless().unwrap();
    request.export = true;
    request.region = None;
    let full = whole_renderer.render(&doc, "Ptest-root", ticks(0.0), &request, &resolver).unwrap();
    assert!(full.failures.is_empty(), "{:?}", full.failures);
    let held = whole_renderer.media_usage();
    assert!(held.frame_cache > 0);
    assert_eq!(held.cache.bytes, held.frame_cache);
    let empty = whole_renderer.apply_media_lease(pm_media::decode::service::MediaLease {
        frame_cache: 0, decoders: usize::MAX,
    });
    assert_eq!(empty.cache.budget, 0);
    assert_eq!(empty.frame_cache, 0);
}

#[test]
fn prores_preview_and_export_are_isolated_in_renderer() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("clip.mov");
    let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
        .join("bin/ffmpeg");
    let encoded = std::process::Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=30",
            "-frames:v",
            "12",
            "-c:v",
            "prores_videotoolbox",
            "-profile:v",
            "2",
            "-pix_fmt",
            "yuv422p10le",
            "-y",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(encoded.success());

    let mut doc = project(
        160,
        90,
        4.0,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({ "asset": "clip", "fit": "stretch", "w": 160, "h": 90, "trim": 0, "speed": 1 }),
            80.0,
            45.0,
            4.0,
        )],
    );
    asset(&mut doc, "clip", "video", "mov", 160, 90, 4.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path);
    let doc = load(&doc);
    let mut renderer = Renderer::headless().unwrap();
    let preview = RenderRequest::at_scale(Size::new(160, 90), 1.0, Precision::Float16);
    let mut export = preview.clone();
    export.export = true;

    let draw = |renderer: &mut Renderer, request: &RenderRequest| {
        let frame = renderer
            .render(&doc, "Ptest-root", ticks(0.2), request, &resolver)
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        renderer.read_rgba(&frame).unwrap()
    };

    let preview_before = draw(&mut renderer, &preview);
    let full_before = draw(&mut renderer, &export);
    assert_eq!(renderer.media_representation(), Representation::Original);
    renderer.set_preview_decode_size(Some((80, 45)));
    let small = draw(&mut renderer, &preview);
    assert_eq!(
        renderer.media_representation(),
        Representation::ReducedResolution
    );
    assert!(small != full_before, "preview must draw a different decode");

    // The export flag overrides the preview preference without changing it.
    let full_after = draw(&mut renderer, &export);
    assert_eq!(renderer.media_representation(), Representation::Original);
    assert!(
        full_after == full_before,
        "export pixels changed after reduced preview"
    );

    renderer.set_preview_decode_size(None);
    let full_preview = draw(&mut renderer, &preview);
    assert_eq!(renderer.media_representation(), Representation::Original);
    assert!(
        full_preview == preview_before,
        "original display pixels changed after reduced preview"
    );
}
