//! Sequence timing and named media errors. Readable sequence originals
//! and persisted renditions retain the gate until complete Chromium parity
//! (including chroma filtering and integer rounding) is proved.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{Feature, MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;

#[test]
fn readable_sequence_renditions_retain_the_parity_gate() {
    let path = support::repo_root().join("apps/desktop/e2e/fixtures/vp9-opus.webm");
    let mut doc = project(
        160,
        90,
        1.0,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({"asset":"clip","w":160,"h":90,"fit":"stretch"}),
            80.0,
            45.0,
            1.0,
        )],
    );
    asset(&mut doc, "clip", "video", "webm", 160, 90, 1.0);
    doc["assets"]["clip"]["imageSequence"] = json!({"fps":10,"frames":10});
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path);
    for format in ["gif", "png", "apng", "webp", "avif", "webm"] {
        doc["assets"]["clip"]["format"] = json!(format);
        assert!(
            pm_render::unsupported_with_media(&load(&doc), "Ptest-root", &resolver)
                .iter()
                .any(|u| u.feature == Feature::ImageSequence)
        );
    }
}

#[test]
fn sequence_timing_keeps_the_last_slot_and_the_shipped_boundary_epsilon() {
    for fps in [12.0, 120.0, 23.976, 240.0] {
        let record = pm_model::Node::from_value(
            pm_model::json::parse(&json!({"imageSequence":{"fps":fps,"frames":3}}).to_string())
                .unwrap(),
            0,
        );
        for (time, index) in [
            (-1.0, 0),
            (0.0, 0),
            (1.0 / fps - 5e-8, 1),
            (2.0 / fps, 2),
            (3.0 / fps, 2),
        ] {
            assert_eq!(
                pm_render::media::sequence_playback_time(&record, time),
                Some(index as f64 / fps + 0.001)
            );
        }
    }
}

#[test]
fn offline_and_corrupt_media_are_named_failures_without_routing() {
    let mut renderer = Renderer::headless().unwrap();
    let mut doc = project(16, 16, 1.0, no_fill(), vec![]);
    let request = RenderRequest::at_scale(Size::new(16, 16), 1.0, Precision::Float16);
    let dir = std::env::temp_dir().join(format!("pm-render-media-errors-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bytes");
    for (kind, format, sequence) in [
        ("image", "png", false),
        ("image", "svg", false),
        ("video", "webm", false),
        ("video", "gif", true),
    ] {
        doc["layers"] = json!([layer(
            "source",
            kind,
            json!({"asset":"clip", "w":16,"h":16}),
            8.0,
            8.0,
            1.0
        )]);
        asset(&mut doc, "clip", kind, format, 16, 16, 1.0);
        if sequence {
            doc["assets"]["clip"]["imageSequence"] = json!({"fps":10,"frames":3});
        }
        let mut resolver = MapResolver::default();
        resolver.paths.insert("clip".into(), path.clone());
        for corrupt in [0, 1, 2] {
            if corrupt != 0 {
                let bytes: &[u8] = if corrupt == 1 {
                    b"not media"
                } else {
                    b"\x1a\x45\xdf\xa3broken WebM"
                };
                std::fs::write(&path, bytes).unwrap();
            }
            let document = load(&doc);
            let unsupported = renderer.unsupported_with_media(&document, "Ptest-root", &resolver);
            assert!(
                unsupported.is_empty(),
                "{kind} corrupt={corrupt}: {unsupported:?}"
            );
            let frame = renderer
                .render(
                    &document,
                    "Ptest-root",
                    support::ticks(0.0),
                    &request,
                    &resolver,
                )
                .unwrap();
            assert_eq!(frame.failures.len(), 1, "{kind}: {:?}", frame.failures);
            assert_eq!(frame.failures[0].layer, "source");
            assert!(!frame.failures[0].message.is_empty());
        }
        std::fs::remove_file(&path).unwrap();
    }
    std::fs::remove_dir(&dir).unwrap();
}
