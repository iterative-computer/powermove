//! Exact app-captured regression for transformed, translucent video layers.

use crate::{MapResolver, RenderRequest, Renderer, VideoEnd, sampling::Sampling};
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

#[test]
fn transformed_midtone_video_matches_app_float_readback() {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video-composition-parity");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(fixtures.join("references.json")).unwrap()).unwrap();
    let document: Value =
        serde_json::from_slice(&std::fs::read(fixtures.join("document.json")).unwrap()).unwrap();
    let references = manifest["references"].as_array().unwrap();
    assert_eq!(
        references.len(),
        6,
        "all captured depth/time references are present"
    );
    let width = manifest["width"].as_u64().unwrap() as u32;
    let height = manifest["height"].as_u64().unwrap() as u32;
    let source = fixtures.join(manifest["source"].as_str().unwrap());
    let source_digest = Sha256::digest(std::fs::read(&source).unwrap());
    assert_eq!(
        format!("{source_digest:x}"),
        manifest["sourceSha256"].as_str().unwrap()
    );
    let mut renderer = Renderer::headless().unwrap();
    let mut compared = 0;

    for precision in [Precision::Float16, Precision::Float32] {
        let depth = match precision {
            Precision::Float16 => 16,
            Precision::Float32 => 32,
        };
        let mut project = document.clone();
        project["colorDepth"] = depth.into();
        let parsed = pm_model::json::parse(&project.to_string()).unwrap();
        let model = pm_model::Document::new(Some(parsed), 1);
        let mut resolver = MapResolver::default();
        resolver
            .paths
            .insert("synthetic-video".into(), source.clone());
        let request = RenderRequest {
            export: true,
            video_end: VideoEnd::Final,
            sampling: Sampling::Final,
            ..RenderRequest::at_scale(Size::new(width, height), 1.0, precision)
        };

        for reference in references.iter().filter(|r| r["depth"] == depth) {
            compared += 1;
            let seconds = reference["time"].as_f64().unwrap();
            let compressed =
                std::fs::read(fixtures.join(reference["file"].as_str().unwrap())).unwrap();
            let mut expected = Vec::new();
            flate2::read::ZlibDecoder::new(&compressed[..])
                .read_to_end(&mut expected)
                .unwrap();
            assert_eq!(expected.len(), (width * height * 16) as usize);
            let digest = Sha256::digest(&expected);
            assert_eq!(format!("{digest:x}"), reference["sha256"].as_str().unwrap());

            let tick =
                Ticks::new((seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap();
            let frame = renderer
                .render(
                    &model,
                    "synthetic-video-composition-root",
                    tick,
                    &request,
                    &resolver,
                )
                .unwrap();
            assert!(
                frame.failures.is_empty(),
                "{depth}-bit at {seconds}s: {:?}",
                frame.failures
            );
            let actual_f32 = renderer.read_rgba(&frame).unwrap();
            let actual: Vec<u8> = actual_f32.iter().flat_map(|v| v.to_le_bytes()).collect();
            assert_eq!(
                actual, expected,
                "{depth}-bit at {seconds}s differs from the actual app Float32 readback"
            );

            let rgba8 = &reference["rgba8"];
            let compressed = std::fs::read(fixtures.join(rgba8["file"].as_str().unwrap())).unwrap();
            let mut captured_rgba = Vec::new();
            flate2::read::ZlibDecoder::new(&compressed[..])
                .read_to_end(&mut captured_rgba)
                .unwrap();
            assert_eq!(captured_rgba.len(), (width * height * 4) as usize);
            let rgba_digest = Sha256::digest(&captured_rgba);
            assert_eq!(
                format!("{rgba_digest:x}"),
                rgba8["sha256"].as_str().unwrap()
            );
            // The app's Float32Array values are promoted to JS Number before
            // Math.round(value * 255). Scaling in f32 fabricates half ties.
            let native_rgba: Vec<u8> = actual_f32
                .iter()
                .map(|value| (f64::from(value.clamp(0.0, 1.0)) * 255.0).round() as u8)
                .collect();
            assert_eq!(
                native_rgba, captured_rgba,
                "{depth}-bit at {seconds}s RGBA8"
            );
        }
    }
    assert_eq!(compared, 4, "both timestamps at both supported precisions");
}
