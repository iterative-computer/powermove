//! Independent synthetic RGB oracle for Chromium's HDR Canvas color transform.
//! The CPU Canvas references are the app's actual `willReadFrequently` path;
//! GPU Canvas references are retained as separately bound evidence.

use super::hdr_compat::{Signal, ToneMap};
use pm_gpu::{
    readback::read_raw,
    target::{Target, working_usage},
};
use pm_protocol::color::Primaries;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io::Read, path::Path};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 143;
const UNIQUE_COLORS: usize = 36_374;
const INPUT_SHA: &str = "bd0b7a37372e6a9f61cdbc866affb1fc0e286f148024d68302c1370a4387bf25";
const CASES: [(&str, &str, &str, &str); 3] = [
    (
        "hlg",
        "bt2020",
        "739ad5606a63b645eb7ad0cfc14f8afcf1164f5b2aa9b67c37b0993a65d2be7b",
        "b1089d66883be5e921a57438461a29a2c13e6f31bdae3446853b74e3e68433b6",
    ),
    (
        "pq",
        "bt2020",
        "696b1411ed3118d5c4a4556f3787329cf483531e64b589b7692e5217389baa89",
        "594affdf124adaaad77a14431df755630af7b1b3938a2a0d6892f8f90650eb60",
    ),
    (
        "pq",
        "smpte432",
        "cb9828b5576a1be076ebd747ea26e37ffd5f7e0cf02ab297d19b78b825dae125",
        "576c7aba7afc6a68812adc3e7ae864facda991ec9f70800cf5d886fb816ab5ab",
    ),
];

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn raster(dir: &Path, entry: &serde_json::Value, expected_sha: &str) -> Result<Vec<u8>, String> {
    let file = entry["file"].as_str().ok_or("missing raster filename")?;
    if file.contains('/') || file.contains('\\') || !file.ends_with(".rgba.zlib") {
        return Err(format!("invalid raster filename: {file}"));
    }
    if entry["sha256"].as_str() != Some(expected_sha) {
        return Err(format!("{file}: manifest SHA differs from frozen oracle"));
    }
    let compressed = std::fs::read(dir.join(file)).map_err(|e| format!("{file}: {e}"))?;
    let mut bytes = Vec::new();
    flate2::read::ZlibDecoder::new(&compressed[..])
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{file}: invalid zlib: {e}"))?;
    if bytes.len() != (WIDTH * HEIGHT * 4) as usize {
        return Err(format!("{file}: wrong raw byte count {}", bytes.len()));
    }
    if sha256(&bytes) != expected_sha {
        return Err(format!("{file}: raster SHA changed"));
    }
    Ok(bytes)
}

#[test]
fn chromium_hdr_rgb_cpu_canvas_transform_matches_every_synthetic_color() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/video-parity-round4/hdr-transform-oracle");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["schemaVersion"].as_u64(), Some(1));
    assert_eq!(manifest["width"].as_u64(), Some(u64::from(WIDTH)));
    assert_eq!(manifest["height"].as_u64(), Some(u64::from(HEIGHT)));
    assert_eq!(
        manifest["uniqueColors"].as_u64(),
        Some(UNIQUE_COLORS as u64)
    );
    assert_eq!(
        manifest["versions"]["chrome"].as_str(),
        Some("152.0.7977.54")
    );
    assert_eq!(manifest["versions"]["electron"].as_str(), Some("44.0.0"));
    assert_eq!(
        manifest["versions"]["v8"].as_str(),
        Some("15.2.124.13-electron.0")
    );
    let input = raster(&dir, &manifest["input"], INPUT_SHA).unwrap();
    let unique: HashSet<[u8; 4]> = input[..UNIQUE_COLORS * 4]
        .chunks_exact(4)
        .map(|pixel| pixel.try_into().unwrap())
        .collect();
    assert_eq!(unique.len(), UNIQUE_COLORS, "synthetic colors changed");
    assert!(input.chunks_exact(4).all(|pixel| pixel[3] == 255));
    assert!(
        input[UNIQUE_COLORS * 4..]
            .chunks_exact(4)
            .all(|pixel| pixel == [255, 255, 255, 255])
    );

    let rows = manifest["rows"].as_array().expect("missing oracle rows");
    assert_eq!(rows.len(), CASES.len());
    let gpu = pm_gpu::Gpu::new("Chromium HDR RGB oracle").unwrap();
    let source = Target::new(
        &gpu.device,
        "synthetic HDR RGB input",
        WIDTH,
        HEIGHT,
        wgpu::TextureFormat::Rgba8Unorm,
        working_usage(),
    );
    gpu.queue.write_texture(
        source.texture.as_image_copy(),
        &input,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(WIDTH * 4),
            rows_per_image: Some(HEIGHT),
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    let tone = ToneMap::new(&gpu.device);
    let mut failures = Vec::new();
    for (row, &(transfer, primaries, cpu_sha, gpu_sha)) in rows.iter().zip(&CASES) {
        let label = format!("{transfer}/{primaries}");
        let tags = [&row["requested"], &row["actual"]];
        if row["signal"].as_str() != Some(transfer)
            || row["primaries"].as_str() != Some(primaries)
            || tags.iter().any(|tag| {
                tag["transfer"].as_str() != Some(transfer)
                    || tag["primaries"].as_str() != Some(primaries)
                    || tag["matrix"].as_str() != Some("rgb")
                    || tag["fullRange"].as_bool() != Some(true)
            })
        {
            failures.push(format!("{label}: requested/actual VideoFrame tags changed"));
        }
        let Some(probes) = row["probes"].as_array() else {
            failures.push(format!("{label}: missing Canvas probes"));
            continue;
        };
        if probes.len() != 2 {
            failures.push(format!("{label}: expected CPU and GPU Canvas probes"));
            continue;
        }
        let mut expected_cpu = None;
        for (probe, frequently, sha) in [(&probes[0], true, cpu_sha), (&probes[1], false, gpu_sha)]
        {
            let settings = &probe["settings"];
            if settings["willReadFrequently"].as_bool() != Some(frequently)
                || settings["colorSpace"].as_str() != Some("srgb")
                || settings["colorType"].as_str() != Some("unorm8")
                || settings["alpha"].as_bool() != Some(true)
                || settings["desynchronized"].as_bool() != Some(false)
                || settings["toneMapping"]["mode"].as_str() != Some("standard")
                || probe["colorSpace"].as_str() != Some("srgb")
            {
                failures.push(format!(
                    "{label}: Canvas probe context changed ({frequently})"
                ));
            }
            match raster(&dir, probe, sha) {
                Ok(bytes) if frequently => expected_cpu = Some(bytes),
                Ok(_) => {}
                Err(error) => failures.push(format!("{label}: {error}")),
            }
        }
        let Some(expected) = expected_cpu else {
            continue;
        };
        let signal = if transfer == "hlg" {
            Signal::Hlg
        } else {
            Signal::Pq
        };
        let gamut = if primaries == "bt2020" {
            Primaries::Bt2020
        } else {
            Primaries::DisplayP3
        };
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let Some(actual_target) = tone.run(&gpu.device, &mut encoder, &source, signal, gamut, None)
        else {
            failures.push(format!(
                "{label}: native tone map refused supported color tags"
            ));
            continue;
        };
        gpu.queue.submit([encoder.finish()]);
        match read_raw(&gpu.device, &gpu.queue, &actual_target.texture) {
            Ok(actual) if actual.len() == expected.len() => {
                let differing = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
                if differing != 0 {
                    let max = actual
                        .iter()
                        .zip(&expected)
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap();
                    failures.push(format!(
                        "{label}: {differing} differing channels, maximum {max}"
                    ));
                }
            }
            Ok(actual) => failures.push(format!(
                "{label}: native byte count {} != {}",
                actual.len(),
                expected.len()
            )),
            Err(error) => failures.push(format!("{label}: GPU readback failed: {error:?}")),
        }
    }
    assert!(
        failures.is_empty(),
        "HDR RGB oracle failures:\n{}",
        failures.join("\n")
    );
}
