//! Strict source-byte regressions for the third public synthetic video corpus.
//! Every captured case is measured before the final strict assertion.

use super::*;
use pm_gpu::readback::read_raw;
use std::io::Read;

#[test]
fn chromium_round3_video_sources_match_all_captured_bytes() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video-parity-round3");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures.join("references.json")).unwrap()).unwrap();
    let sources = manifest["sources"].as_array().unwrap();
    let references = manifest["references"].as_array().unwrap();
    assert_eq!(
        sources.len(),
        16,
        "every round 3 recipe must be represented"
    );
    let captured = sources
        .iter()
        .filter(|source| source["status"] == "captured")
        .count();
    assert_eq!(
        references.len(),
        captured * 3,
        "all three timestamps of every app-decodable source must be compared"
    );
    // Explicit app-decoder errors remain in the manifest's gaps; no reference
    // is invented for them. Every captured source, including one the native
    // decoder cannot yet read, is required to produce exact output here.
    let gpu = pm_gpu::Gpu::new("Chromium round 3 video source parity").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let mut media = MediaCache::new(&gpu.device);
    let mut failures = Vec::new();
    for gap in manifest["gaps"].as_array().unwrap() {
        failures.push(format!("app capture gap: {gap}"));
    }
    for reference in references {
        let source = reference["source"].as_str().unwrap();
        let seconds = reference["time"].as_f64().unwrap();
        let width = reference["width"].as_u64().unwrap() as usize;
        let height = reference["height"].as_u64().unwrap() as usize;
        let media_path = fixtures.join(reference["media"].as_str().unwrap());
        for (canvas, field, reading) in [
            (true, "premultipliedCanvas", "Canvas"),
            (false, "premultipliedTexture", "Live"),
        ] {
            let compressed =
                std::fs::read(fixtures.join(reference[field]["file"].as_str().unwrap())).unwrap();
            let mut expected = Vec::new();
            flate2::read::ZlibDecoder::new(&compressed[..])
                .read_to_end(&mut expected)
                .unwrap();
            assert_eq!(
                expected.len(),
                width * height * 4,
                "{source} at {seconds}s {reading}"
            );
            media.canvas_video = canvas;
            for precision in [Precision::Unorm8, Precision::Float16, Precision::Float32] {
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                let target = match media.video(
                    &gpu.device,
                    &gpu.queue,
                    &pipelines,
                    &mut encoder,
                    &media_path,
                    1,
                    seconds,
                    true,
                    WorkingEncoding::Standard,
                    precision,
                    None,
                    pm_time::Rate::FPS_30,
                    false,
                    true,
                ) {
                    Ok(Some(target)) => target,
                    Ok(None) => {
                        failures.push(format!(
                            "{source} at {seconds}s {reading} {precision:?}: native source missing"
                        ));
                        media.flush();
                        continue;
                    }
                    Err(error) => {
                        failures.push(format!(
                        "{source} at {seconds}s {reading} {precision:?}: native decode gap: {error:?}"
                    ));
                        media.flush();
                        continue;
                    }
                };
                if target.format() != wgpu::TextureFormat::Rgba8Unorm {
                    failures.push(format!(
                        "{source} at {seconds}s {reading} {precision:?}: wrong output format: {:?}",
                        target.format()
                    ));
                    media.flush();
                    continue;
                }
                gpu.queue.submit([encoder.finish()]);
                let actual = match read_raw(&gpu.device, &gpu.queue, &target.texture) {
                    Ok(actual) => actual,
                    Err(error) => {
                        failures.push(format!(
                        "{source} at {seconds}s {reading} {precision:?}: readback failed: {error:?}"
                    ));
                        media.flush();
                        continue;
                    }
                };
                if actual.len() != expected.len() {
                    failures.push(format!(
                        "{source} at {seconds}s {reading} {precision:?}: expected {} bytes, got {}",
                        expected.len(),
                        actual.len()
                    ));
                    media.flush();
                    continue;
                }
                let changed = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
                let max = actual
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                if changed != 0 {
                    failures.push(format!(
                    "{source} at {seconds}s {reading} {precision:?}: {changed} differing channels, max {max}"
                ));
                }
                media.flush();
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Round 3 parity failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn vp9_alpha_interpretation_preserves_native_pixels_and_visible_domain() {
    use pm_media::interpret::AlphaChoice;
    let gpu = pm_gpu::Gpu::new("VP9 alpha interpretation").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let mut media = MediaCache::new(&gpu.device);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/video-parity-round3/vp9-alpha-709-r3.webm");
    media.canvas_video = false;
    for alpha in [AlphaChoice::Ignore, AlphaChoice::Straight] {
        let interpretation = Interpretation {
            alpha: Some(alpha),
            ..Default::default()
        };
        let mut readings = Vec::new();
        for compat in [false, true] {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let target = media
                .video(
                    &gpu.device,
                    &gpu.queue,
                    &pipelines,
                    &mut encoder,
                    &path,
                    1,
                    0.625,
                    true,
                    WorkingEncoding::Standard,
                    Precision::Float32,
                    Some(&interpretation),
                    pm_time::Rate::FPS_30,
                    false,
                    compat,
                )
                .unwrap()
                .unwrap();
            assert_eq!(target.format(), wgpu::TextureFormat::Rgba32Float);
            gpu.queue.submit([encoder.finish()]);
            readings.push(read_raw(&gpu.device, &gpu.queue, &target.texture).unwrap());
        }
        assert_eq!(readings[0], readings[1], "alpha={alpha:?}");
    }
}
