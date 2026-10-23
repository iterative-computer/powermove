//! Exact comparison with the shipped app's video and 3D depth captures.
//! Usage: cargo run -p pm-render --example color_depth_media -- /external/capture
//! The capture directory, including native/diff PNGs, must be outside Git.
#[path = "../tests/support/mod.rs"]
mod support;

use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn input_dir() -> PathBuf {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("external capture directory"),
    );
    let canonical = path.canonicalize().expect("capture directory exists");
    assert!(
        !canonical.starts_with(support::repo_root()),
        "capture must be outside Git"
    );
    canonical
}

fn sample_id(value: &serde_json::Value) -> &str {
    let id = value["id"].as_str().expect("sample id");
    assert!(!id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    id
}

fn bytes_of(pixels: &[f32]) -> Vec<u8> {
    pixels
        .iter()
        .map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect()
}

fn reference(dir: &Path, id: &str, expected: usize) -> Vec<f32> {
    let raw = std::fs::read(dir.join(format!("{id}.rgba32f"))).expect("legacy float32 frame");
    assert_eq!(raw.len(), expected * 4, "{id}: raw frame size");
    raw.chunks_exact(4)
        .map(|four| f32::from_le_bytes(four.try_into().unwrap()))
        .collect()
}

fn main() {
    let dir = input_dir();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).expect("manifest"))
            .expect("valid manifest");
    let samples = manifest["samples"].as_array().expect("samples array");
    assert_eq!(
        samples.len(),
        15,
        "three depths × two times for each video fixture and one 3D time"
    );
    let mut renderer = Renderer::headless().expect("native renderer");
    assert!(
        renderer
            .set_definitions(&pm_render::all_builtin_definitions())
            .unwrap()
            .is_empty()
    );
    let mut failures = 0;
    let mut results = Vec::new();
    for sample in samples {
        renderer.trim(0);
        let id = sample_id(sample);
        let source_diagnostic = match sample["kind"].as_str().expect("sample kind") {
            "video" => {
                assert!(sample["visibleVideoPixels"].as_u64().unwrap() > 320 * 180 / 2);
                serde_json::Value::Null
            }
            "video-detail" => {
                assert!(
                    sample["colors"].as_u64().unwrap() > 64,
                    "{id}: color variety"
                );
                assert!(
                    sample["detailNonbackgroundPixels"].as_u64().unwrap() > 7200,
                    "{id}: detailed image visible"
                );
                assert!(
                    sample["detailMidtonePixels"].as_u64().unwrap() > 7200,
                    "{id}: midtone image visible"
                );
                let source = &sample["source"];
                let (source_w, source_h) = (
                    source["width"].as_u64().expect("source width"),
                    source["height"].as_u64().expect("source height"),
                );
                assert_eq!(
                    (source_w, source_h),
                    (160, 90),
                    "{id}: prepared source size"
                );
                let source_bytes = std::fs::read(dir.join(format!("{id}.source.rgba")))
                    .expect("prepared video source pixels");
                assert_eq!(
                    source_bytes.len(),
                    (source_w * source_h * 4) as usize,
                    "{id}: prepared source length"
                );
                let distinct: HashSet<[u8; 3]> = source_bytes
                    .chunks_exact(4)
                    .map(|p| [p[0], p[1], p[2]])
                    .collect();
                let midtones = source_bytes
                    .chunks_exact(4)
                    .filter(|p| p[..3].iter().all(|v| *v > 32 && *v < 224))
                    .count();
                assert!(
                    distinct.len() > 64 && midtones > 7200,
                    "{id}: prepared source lacks detailed mid-tones"
                );
                json!({"width":source_w,"height":source_h,"distinctColors":distinct.len(),"midtonePixels":midtones})
            }
            "scene3d" => {
                assert!(
                    sample["colors"].as_u64().unwrap() > 8,
                    "{id}: visible geometry"
                );
                serde_json::Value::Null
            }
            other => panic!("{id}: unexpected kind {other}"),
        };
        let (w, h) = (
            sample["width"].as_u64().unwrap() as u32,
            sample["height"].as_u64().unwrap() as u32,
        );
        assert_eq!((w, h), (320, 180), "{id}: capture size");
        let doc =
            pm_model::Document::new(Some(support::read_json(&dir.join(format!("{id}.json")))), 1);
        let comp = pm_eval::node::string(pm_eval::node::member(doc.root(), "compId"))
            .expect("composition id")
            .to_string_lossy();
        let mut resolver = MapResolver::default();
        for media in sample["media"].as_array().expect("media list") {
            let key = media["assetId"].as_str().expect("asset id");
            let file = PathBuf::from(media["path"].as_str().expect("asset path"));
            assert!(
                file.is_file(),
                "{id}: fixture media is missing: {}",
                file.display()
            );
            resolver.paths.insert(key.into(), file);
        }
        let req = RenderRequest::sized(Size::new(w, h), Precision::Float32);
        let render = renderer.render(
            &doc,
            &comp,
            support::ticks(sample["time"].as_f64().unwrap()),
            &req,
            &resolver,
        );
        let (native_float, render_error) = match render {
            Ok(frame) if frame.failures.is_empty() => {
                (Some(renderer.read_rgba(&frame).expect("readback")), None)
            }
            Ok(frame) => (None, Some(format!("render failures: {:?}", frame.failures))),
            Err(error) => (None, Some(format!("render error: {error:?}"))),
        };
        let legacy_float = reference(&dir, id, (w * h * 4) as usize);
        let legacy = bytes_of(&legacy_float);
        let (different, max_byte_delta, max_raw_cv) = if let Some(actual_float) = &native_float {
            let actual = bytes_of(actual_float);
            assert_eq!(actual.len(), legacy.len(), "{id}: rendered size");
            let raw_cv = actual_float
                .iter()
                .zip(&legacy_float)
                .map(|(a, b)| f64::from((a - b).abs()) * 255.0)
                .fold(0.0_f64, f64::max);
            assert!(raw_cv.is_finite(), "{id}: nonfinite raw float difference");
            let differing = actual
                .chunks_exact(4)
                .zip(legacy.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            let max = actual
                .iter()
                .zip(&legacy)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            support::write_png(&dir.join(format!("{id}.native.png")), w, h, &actual);
            let mut diff: Vec<u8> = actual
                .iter()
                .zip(&legacy)
                .map(|(a, b)| a.abs_diff(*b).saturating_mul(32))
                .collect();
            for pixel in diff.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
            support::write_png(&dir.join(format!("{id}.diff.png")), w, h, &diff);
            (differing, max, Some(raw_cv))
        } else {
            (w as usize * h as usize, 255, None)
        };
        let pass = render_error.is_none() && different == 0;
        failures += usize::from(!pass);
        println!(
            "{id}: different={different}, maxByteDelta={max_byte_delta}, maxRawCv={max_raw_cv:?}, error={render_error:?}"
        );
        results.push(
            json!({"id":id,"depth":sample["depth"],"kind":sample["kind"],"pass":pass,
            "differentPixels":different,"maxByteDelta":max_byte_delta,"maxRawCv":max_raw_cv,
            "source":source_diagnostic,"error":render_error}),
        );
    }
    std::fs::write(
        dir.join("media-parity.json"),
        serde_json::to_vec_pretty(&json!({"failures":failures,"samples":results})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        failures, 0,
        "Video/3D color-depth parity differs; inspect external captures and media-parity.json"
    );
}
