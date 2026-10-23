//! Exact legacy-app HDR/P3 source readings and their independently captured
//! Canvas color-space controls. A gap is reported, never treated as a pass.

use super::*;
use pm_gpu::readback::read_raw;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Read;

const SOURCES: [&str; 7] = [
    "av1-10bit-2020-pq-r3",
    "av1-10bit-2020-hlg-r3",
    "hevc10-2020-pq-r3",
    "hevc10-p3-pq-r3",
    "vp9-10bit-2020-limited-r3",
    "hevc10-p3-sdr-r4",
    "hevc10-2020-sdr-r4",
];
const TIMES: [f64; 3] = [0.125, 0.625, 1.125];

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn raster(
    fixtures: &Path,
    entry: &serde_json::Value,
    size: usize,
    label: &str,
) -> Result<Vec<u8>, String> {
    let file = entry["file"]
        .as_str()
        .ok_or_else(|| format!("{label}: missing raster file"))?;
    let compressed = std::fs::read(fixtures.join(file))
        .map_err(|error| format!("{label}: cannot read {file}: {error}"))?;
    let mut bytes = Vec::new();
    flate2::read::ZlibDecoder::new(&compressed[..])
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{label}: invalid zlib in {file}: {error}"))?;
    if bytes.len() != size {
        return Err(format!(
            "{label}: {file} has {} raw bytes, expected {size}",
            bytes.len()
        ));
    }
    let expected = entry["sha256"]
        .as_str()
        .ok_or_else(|| format!("{label}: missing SHA-256 for {file}"))?;
    if sha256(&bytes) != expected {
        return Err(format!("{label}: SHA-256 changed for {file}"));
    }
    Ok(bytes)
}

#[test]
fn chromium_round4_hdr_p3_controls_match_all_captured_bytes() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video-parity-round4");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures.join("references.json")).unwrap()).unwrap();
    let sources = manifest["sources"].as_array().unwrap();
    let references = manifest["references"].as_array().unwrap();
    let mut failures = Vec::new();
    if sources.len() != SOURCES.len() {
        failures.push(format!(
            "source count {} differs from {} recipes",
            sources.len(),
            SOURCES.len()
        ));
    }
    if references.len() != SOURCES.len() * TIMES.len() {
        failures.push(format!(
            "reference count {} differs from 7 sources × 3 times",
            references.len()
        ));
    }
    for (index, id) in SOURCES.iter().enumerate() {
        let Some(source) = sources.get(index) else {
            failures.push(format!("missing source manifest row {id}"));
            continue;
        };
        if source["id"].as_str() != Some(id) {
            failures.push(format!(
                "wrong source order/ID at row {index}: expected {id}"
            ));
        }
        if source["times"].as_array().is_none_or(|times| {
            times.len() != TIMES.len()
                || times
                    .iter()
                    .zip(TIMES)
                    .any(|(actual, expected)| actual.as_f64() != Some(expected))
        }) {
            failures.push(format!("{id}: timestamps differ from {TIMES:?}"));
        }
        let Some(file) = source["file"].as_str() else {
            failures.push(format!("{id}: missing media filename"));
            continue;
        };
        match std::fs::read(fixtures.join(file)) {
            Ok(bytes) if source["sourceSha256"].as_str() == Some(sha256(&bytes).as_str()) => {}
            Ok(_) => failures.push(format!("{id}: source SHA-256 changed for {file}")),
            Err(error) => failures.push(format!("{id}: cannot read {file}: {error}")),
        }
    }
    if let Some(gaps) = manifest["gaps"].as_array() {
        for gap in gaps {
            failures.push(format!("app capture/probe gap: {gap}"));
        }
    } else {
        failures.push("missing app gap list".into());
    }

    let gpu = pm_gpu::Gpu::new("Chromium HDR/P3 video source parity").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let mut media = MediaCache::new(&gpu.device);
    let mut seen = HashSet::new();
    for reference in references {
        let Some(source) = reference["source"].as_str() else {
            failures.push("reference missing source ID".into());
            continue;
        };
        let Some(sample) = reference["sample"].as_u64().map(|value| value as usize) else {
            failures.push(format!("{source}: reference missing sample index"));
            continue;
        };
        if !SOURCES.contains(&source) || sample >= TIMES.len() {
            failures.push(format!("unknown source/sample: {source}-{sample}"));
            continue;
        }
        if !seen.insert((source.to_owned(), sample)) {
            failures.push(format!("duplicate reference: {source}-{sample}"));
            continue;
        }
        let label = format!("{source}-{sample}");
        let Some(source_row) = sources
            .iter()
            .find(|row| row["id"].as_str() == Some(source))
        else {
            failures.push(format!("{label}: missing source row"));
            continue;
        };
        let (Some(width), Some(height), Some(seconds)) = (
            reference["width"].as_u64().map(|v| v as usize),
            reference["height"].as_u64().map(|v| v as usize),
            reference["time"].as_f64(),
        ) else {
            failures.push(format!("{label}: missing dimensions or timestamp"));
            continue;
        };
        if reference["media"].as_str() != source_row["file"].as_str()
            || source_row["width"].as_u64() != Some(width as u64)
            || source_row["height"].as_u64() != Some(height as u64)
            || seconds != TIMES[sample]
            || reference["timestamp"].as_f64().is_none_or(|pts| {
                (pts / 1_000_000.0 - (sample as f64 * 0.5 + 1.0 / 12.0)).abs() > 0.02
            })
        {
            failures.push(format!("{label}: media/dimensions/PTS binding changed"));
        }
        if reference["canvasSettings"]["colorSpace"].as_str() != Some("srgb")
            || reference["canvasSettings"]["willReadFrequently"].as_bool() != Some(true)
            || reference["webglColorSpace"]["drawingBuffer"].as_str() != Some("srgb")
            || reference["webglColorSpace"]["unpack"].as_str() != Some("srgb")
        {
            failures.push(format!(
                "{label}: legacy Canvas/WebGL color-space path changed"
            ));
        }
        let size = width * height * 4;
        for field in ["rgba", "canvas"] {
            if let Err(error) = raster(
                &fixtures,
                &reference[field],
                size,
                &format!("{label} {field}"),
            ) {
                failures.push(error);
            }
        }
        for (name, space) in [
            ("srgbGpu", "srgb"),
            ("displayP3Cpu", "display-p3"),
            ("displayP3Gpu", "display-p3"),
        ] {
            let probe = &reference["probes"][name];
            if probe["settings"]["colorSpace"].as_str() != Some(space)
                || probe["imageDataColorSpace"].as_str() != Some(space)
                || probe["settings"]["willReadFrequently"].as_bool() != Some(name == "displayP3Cpu")
            {
                failures.push(format!(
                    "{label} {name}: Canvas color-space evidence changed"
                ));
            }
            for field in ["canvas", "premult"] {
                if let Err(error) = raster(
                    &fixtures,
                    &probe[field],
                    size,
                    &format!("{label} {name} {field}"),
                ) {
                    failures.push(error);
                }
            }
        }
        let Some(file) = reference["media"].as_str() else {
            failures.push(format!("{label}: missing media file"));
            continue;
        };
        let path = fixtures.join(file);
        for (canvas, field, reading) in [(true, "premult", "Canvas"), (false, "live", "Live")] {
            let expected = match raster(
                &fixtures,
                &reference[field],
                size,
                &format!("{label} {reading}"),
            ) {
                Ok(bytes) => bytes,
                Err(error) => {
                    failures.push(error);
                    continue;
                }
            };
            media.canvas_video = canvas;
            for precision in [Precision::Float16, Precision::Float32] {
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                let target = match media.video(
                    &gpu.device,
                    &gpu.queue,
                    &pipelines,
                    &mut encoder,
                    &path,
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
                            "{label} {reading} {precision:?}: native source missing"
                        ));
                        media.flush();
                        continue;
                    }
                    Err(error) => {
                        failures.push(format!(
                            "{label} {reading} {precision:?}: native decode gap: {error:?}"
                        ));
                        media.flush();
                        continue;
                    }
                };
                if target.format() != wgpu::TextureFormat::Rgba8Unorm {
                    failures.push(format!(
                        "{label} {reading} {precision:?}: wrong target format {:?}",
                        target.format()
                    ));
                    media.flush();
                    continue;
                }
                gpu.queue.submit([encoder.finish()]);
                match read_raw(&gpu.device, &gpu.queue, &target.texture) {
                    Ok(actual) if actual.len() == expected.len() => {
                        let changed = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
                        if changed != 0 {
                            let max = actual
                                .iter()
                                .zip(&expected)
                                .map(|(a, b)| a.abs_diff(*b))
                                .max()
                                .unwrap();
                            failures.push(format!("{label} {reading} {precision:?}: {changed} differing channels, max {max}"));
                        }
                    }
                    Ok(actual) => failures.push(format!(
                        "{label} {reading} {precision:?}: {} bytes, expected {}",
                        actual.len(),
                        expected.len()
                    )),
                    Err(error) => failures.push(format!(
                        "{label} {reading} {precision:?}: readback failed: {error:?}"
                    )),
                }
                media.flush();
            }
        }
    }
    for source in SOURCES {
        for sample in 0..TIMES.len() {
            if !seen.contains(&(source.to_owned(), sample)) {
                failures.push(format!("missing captured reference: {source}-{sample}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Round 4 parity failures:\n{}",
        failures.join("\n")
    );
}
