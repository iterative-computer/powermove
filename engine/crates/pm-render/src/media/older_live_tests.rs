//! Exact direct-video WebGL readings for the original and Round 2 synthetic clips.
//! The original capture is opaque; Round 2 includes actual premultiplied alpha.

use super::*;
use pm_gpu::readback::read_raw;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io::Read};

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn live_raster(
    folder: &Path,
    frame: &serde_json::Value,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, String> {
    let file = frame["file"]
        .as_str()
        .ok_or("missing live raster filename")?;
    if file.contains('/') || file.contains('\\') || !file.ends_with(".live.rgba.zlib") {
        return Err(format!("invalid live raster filename: {file}"));
    }
    let compressed = std::fs::read(folder.join(file)).map_err(|e| format!("{file}: {e}"))?;
    let mut bytes = Vec::new();
    flate2::read::ZlibDecoder::new(&compressed[..])
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{file}: invalid zlib: {e}"))?;
    if bytes.len() != width * height * 4 {
        return Err(format!(
            "{file}: {} bytes, expected {}",
            bytes.len(),
            width * height * 4
        ));
    }
    if frame["sha256"].as_str() != Some(sha256(&bytes).as_str()) {
        return Err(format!("{file}: SHA-256 changed"));
    }
    Ok(bytes)
}

#[test]
fn chromium_older_direct_video_uploads_match_all_62_live_frames() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let gpu = pm_gpu::Gpu::new("Chromium original and Round 2 live video parity").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let mut media = MediaCache::new(&gpu.device);
    media.canvas_video = false;
    let mut failures = Vec::new();
    let mut checked = 0;
    for (set, expected_sources, expected_frames) in
        [("video-parity", 13, 38), ("video-parity-round2", 8, 24)]
    {
        let fixture = root.join(set);
        let folder = fixture.join("live-oracle");
        let old: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture.join("references.json")).unwrap())
                .unwrap();
        let oracle: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(oracle["schemaVersion"].as_u64(), Some(1));
        assert_eq!(
            oracle["sourceSet"].as_str(),
            Some(if set == "video-parity" {
                "original"
            } else {
                "round2"
            })
        );
        assert_eq!(
            oracle["captureVersions"]["chrome"].as_str(),
            Some("152.0.7977.54")
        );
        assert_eq!(
            oracle["captureVersions"]["electron"].as_str(),
            Some("44.0.0")
        );
        assert_eq!(
            oracle["captureVersions"]["v8"].as_str(),
            Some("15.2.124.13-electron.0")
        );
        let sources = oracle["sources"].as_array().expect("missing live sources");
        let frames = oracle["frames"].as_array().expect("missing live frames");
        let old_sources = old["sources"]
            .as_array()
            .expect("missing established sources");
        if sources.len() != expected_sources
            || frames.len() != expected_frames
            || old_sources.len() != expected_sources
        {
            failures.push(format!(
                "{set}: source/frame counts changed: {}/{}/{}",
                sources.len(),
                frames.len(),
                old_sources.len()
            ));
        }
        let mut seen_sources = HashSet::new();
        let mut seen_frames = HashSet::new();
        for (source, established) in sources.iter().zip(old_sources) {
            let Some(id) = source["id"].as_str() else {
                failures.push(format!("{set}: source without ID"));
                continue;
            };
            if !seen_sources.insert(id) {
                failures.push(format!("{set}: duplicate source {id}"));
            }
            let old_source_file = if set == "video-parity" {
                &established["original"]
            } else {
                &established["file"]
            };
            if established["id"].as_str() != Some(id) || old_source_file != &source["sourceFile"] {
                failures.push(format!("{set}/{id}: source identity changed"));
                continue;
            }
            let Some(source_file) = source["sourceFile"].as_str() else {
                failures.push(format!("{set}/{id}: missing source file"));
                continue;
            };
            let source_bytes = match std::fs::read(fixture.join(source_file)) {
                Ok(bytes) => bytes,
                Err(error) => {
                    failures.push(format!("{set}/{id}: source unavailable: {error}"));
                    continue;
                }
            };
            let source_sha = sha256(&source_bytes);
            if source["sourceSha256"].as_str() != Some(source_sha.as_str())
                || (set == "video-parity-round2"
                    && established["sourceSha256"].as_str() != Some(source_sha.as_str()))
            {
                failures.push(format!("{set}/{id}: original source SHA changed"));
            }
            let Some(media_file) = source["mediaFile"].as_str() else {
                failures.push(format!("{set}/{id}: missing playback file"));
                continue;
            };
            if media_file.contains('/') || media_file.contains('\\') {
                failures.push(format!("{set}/{id}: invalid playback filename"));
                continue;
            }
            let playback = source["playback"].is_object();
            if playback != id.starts_with("prores")
                || (playback
                    && (source["playback"]["proxy"].as_bool() != Some(true)
                        || source["playback"]["liveElementMatched"].as_bool() != Some(true)
                        || source["playback"]["container"].as_str() != Some("video/webm")))
            {
                failures.push(format!("{set}/{id}: playback provenance changed"));
            }
            let media_path = if set == "video-parity-round2" && playback {
                folder.join(media_file)
            } else {
                fixture.join(media_file)
            };
            let media_bytes = match std::fs::read(&media_path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    failures.push(format!("{set}/{id}: playback unavailable: {error}"));
                    continue;
                }
            };
            let media_sha = sha256(&media_bytes);
            if source["mediaSha256"].as_str() != Some(media_sha.as_str())
                || (set == "video-parity"
                    && established["sha256"].as_str() != Some(media_sha.as_str()))
                || (!playback && source_sha != media_sha)
            {
                failures.push(format!("{set}/{id}: playback SHA changed"));
            }
            if set == "video-parity-round2"
                && playback
                && established["playback"]["sha256"].as_str() == Some(media_sha.as_str())
            {
                failures.push(format!("{set}/{id}: expected the fresh import rendition"));
            }
        }
        if seen_sources.len() != expected_sources {
            failures.push(format!(
                "{set}: only {} distinct sources",
                seen_sources.len()
            ));
        }
        for frame in frames {
            let (Some(id), Some(sample), Some(width), Some(height), Some(time)) = (
                frame["source"].as_str(),
                frame["sample"].as_u64().map(|v| v as usize),
                frame["width"].as_u64().map(|v| v as usize),
                frame["height"].as_u64().map(|v| v as usize),
                frame["time"].as_f64(),
            ) else {
                failures.push(format!("{set}: incomplete live frame metadata"));
                continue;
            };
            let label = format!("{set}/{id}-{sample}");
            if !seen_frames.insert((id, sample)) {
                failures.push(format!("{label}: duplicate live frame"));
                continue;
            }
            let Some(source) = sources
                .iter()
                .find(|source| source["id"].as_str() == Some(id))
            else {
                failures.push(format!("{label}: missing bound source"));
                continue;
            };
            let timestamp = frame["timestampUs"].as_i64();
            let expected_timestamp = if id == "h264-handoff" {
                [500_000, 1_500_000].get(sample).copied()
            } else if id.starts_with("prores") || id.starts_with("vp9") {
                [83_000, 583_000, 1_083_000].get(sample).copied()
            } else {
                [83_333, 583_333, 1_083_333].get(sample).copied()
            };
            if source["times"]
                .as_array()
                .and_then(|a| a.get(sample))
                .and_then(|v| v.as_f64())
                != Some(time)
                || timestamp != expected_timestamp
                || frame["alphaMode"].as_str()
                    != Some(if set == "video-parity-round2" && id == "prores4444-709" {
                        "nonuniformPremultiplied"
                    } else {
                        "opaque"
                    })
            {
                failures.push(format!("{label}: time/PTS/alpha binding changed"));
            }
            let old_frames = if set == "video-parity" {
                &old["frames"]
            } else {
                &old["references"]
            };
            if !old_frames.as_array().is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry["source"].as_str() == Some(id)
                        && entry["sample"].as_u64() == Some(sample as u64)
                        && entry["time"].as_f64() == Some(time)
                        && entry["width"].as_u64() == Some(width as u64)
                        && entry["height"].as_u64() == Some(height as u64)
                })
            }) {
                failures.push(format!("{label}: established frame identity changed"));
            }
            let expected = match live_raster(&folder, frame, width, height) {
                Ok(bytes) => bytes,
                Err(error) => {
                    failures.push(format!("{label}: {error}"));
                    continue;
                }
            };
            let alpha: Vec<u8> = expected[3..].iter().step_by(4).copied().collect();
            if !(set == "video-parity-round2" && id == "prores4444-709")
                && alpha.iter().any(|&a| a != 255)
            {
                failures.push(format!("{label}: opaque live texture has nonopaque alpha"));
                continue;
            }
            if set == "video-parity-round2"
                && id == "prores4444-709"
                && (alpha.iter().copied().min() == alpha.iter().copied().max()
                    || alpha.iter().copied().min() == Some(255))
            {
                failures.push(format!("{label}: premultiplied alpha gradient missing"));
            }
            let media_file = source["mediaFile"].as_str().unwrap();
            let path = if set == "video-parity-round2" && id.starts_with("prores") {
                folder.join(media_file)
            } else {
                fixture.join(media_file)
            };
            for precision in [Precision::Unorm8, Precision::Float16, Precision::Float32] {
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                let target = match media.video(
                    &gpu.device,
                    &gpu.queue,
                    &pipelines,
                    &mut encoder,
                    &path,
                    1,
                    time,
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
                        failures.push(format!("{label} {precision:?}: native source missing"));
                        media.flush();
                        continue;
                    }
                    Err(error) => {
                        failures.push(format!(
                            "{label} {precision:?}: native decode gap: {error:?}"
                        ));
                        media.flush();
                        continue;
                    }
                };
                if target.format() != wgpu::TextureFormat::Rgba8Unorm {
                    failures.push(format!(
                        "{label} {precision:?}: wrong target format {:?}",
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
                            failures.push(format!(
                                "{label} {precision:?}: {changed} differing channels, maximum {max}"
                            ));
                        }
                    }
                    Ok(actual) => failures.push(format!(
                        "{label} {precision:?}: {} bytes, expected {}",
                        actual.len(),
                        expected.len()
                    )),
                    Err(error) => {
                        failures.push(format!("{label} {precision:?}: GPU readback: {error:?}"))
                    }
                }
                media.flush();
            }
            checked += 1;
        }
        if seen_frames.len() != expected_frames {
            failures.push(format!(
                "{set}: only {} unique live frames",
                seen_frames.len()
            ));
        }
    }
    if checked != 62 {
        failures.push(format!("only {checked}/62 live frames checked"));
    }
    assert!(
        failures.is_empty(),
        "Older live parity failures:\n{}",
        failures.join("\n")
    );
}
