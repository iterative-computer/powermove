//! The renderer can alternate preview and export representations in one
//! process without handing reduced pixels to an export.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_protocol::frame::Representation;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::{asset, layer, load, no_fill, project};
use support::ticks;

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
    assert!(full_after == full_before, "export pixels changed after reduced preview");

    renderer.set_preview_decode_size(None);
    let full_preview = draw(&mut renderer, &preview);
    assert_eq!(renderer.media_representation(), Representation::Original);
    assert!(full_preview == preview_before, "original display pixels changed after reduced preview");
}
