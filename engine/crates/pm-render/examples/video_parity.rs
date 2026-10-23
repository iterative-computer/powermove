//! Compare synthetic app canvas captures made by scripts/video-parity.mjs.
//!   cargo run -p pm-render --example video_parity -- EVIDENCE_DIR
//! Writes native source images as raw RGBA8 and measured errors outside Git.
#[path = "../tests/support/doc.rs"]
mod doc;
use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer, VideoEnd};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("EVIDENCE_DIR"));
    let captured: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("chromium.json"))?)?;
    let mut renderer = Renderer::headless()?;
    let mut results = vec![];
    for row in captured["rows"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        if let Some(error) = row.get("error") {
            results.push(json!({"id": id, "chromiumError": error}));
            continue;
        }
        for (i, sample) in row["frames"].as_array().unwrap().iter().enumerate() {
            let w = sample["w"].as_u64().unwrap() as u32;
            let h = sample["h"].as_u64().unwrap() as u32;
            let duration = row["probe"]["duration"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(2.0);
            let mut project = doc::project(
                w,
                h,
                duration,
                doc::no_fill(),
                vec![doc::layer(
                    "v",
                    "video",
                    json!({"asset":"clip", "fit":"stretch", "w":w, "h":h, "trim":0, "speed":1}),
                    w as f64 / 2.0,
                    h as f64 / 2.0,
                    duration,
                )],
            );
            doc::asset(&mut project, "clip", "video", "mp4", w, h, duration);
            if let Some(file) = row["document"].as_str() {
                project = serde_json::from_slice(&std::fs::read(dir.join(file))?)?;
            }
            let mut resolver = MapResolver::default();
            for (id, asset) in project["assets"].as_object().unwrap() {
                if asset["kind"] == "video" {
                    resolver.paths.insert(
                        id.clone(),
                        dir.join(
                            row["playback"]["file"]
                                .as_str()
                                .unwrap_or_else(|| row["file"].as_str().unwrap()),
                        ),
                    );
                }
            }
            let live = row["reading"].as_str() == Some("live");
            let request = RenderRequest {
                export: !live,
                video_end: VideoEnd::Final,
                sampling: if live {
                    pm_render::sampling::Sampling::Playback
                } else {
                    pm_render::sampling::Sampling::Final
                },
                ..RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32)
            };
            let scope = pm_perf::Scope::begin();
            let started = std::time::Instant::now();
            let frame = match renderer.render(
                &doc::load(&project),
                project["compId"].as_str().unwrap(),
                pm_protocol::common::Ticks::new(
                    (sample["time"].as_f64().unwrap() * pm_time::TICKS_PER_SECOND as f64).round()
                        as i64,
                )
                .unwrap(),
                &request,
                &resolver,
            ) {
                Ok(frame) if frame.failures.is_empty() => frame,
                result => {
                    let error = match result {
                        Ok(frame) => format!("{:?}", frame.failures),
                        Err(error) => error.to_string(),
                    };
                    results.push(json!({"id":id, "sample":i, "nativeError":error}));
                    continue;
                }
            };
            let micros = started.elapsed().as_micros();
            let counts = scope.finish();
            let floats = renderer.read_rgba(&frame)?;
            std::fs::write(
                dir.join(format!("{id}-{i}.render.rgba32f")),
                floats
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )?;
            // The app's capture maps Float32Array values with JavaScript
            // Math.round: promote to binary64 before scaling by 255.
            // Scaling in f32 first creates false ties at e.g. 84.50000256.
            let rgba: Vec<u8> = floats
                .into_iter()
                .map(|v| (f64::from(v.clamp(0.0, 1.0)) * 255.0).round() as u8)
                .collect();
            std::fs::write(dir.join(format!("{id}-{i}.render.rgba")), &rgba)?;
            let reference = std::fs::read(
                dir.join(
                    sample["nativeReference"]
                        .as_str()
                        .or_else(|| sample["reference"].as_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("{id}-{i}.canvas")),
                ),
            )?;
            assert_eq!(rgba.len(), reference.len());
            let max = rgba
                .iter()
                .zip(&reference)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            let different = rgba
                .chunks_exact(4)
                .zip(reference.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            let counts: std::collections::BTreeMap<_, _> = pm_perf::Counter::ALL
                .iter()
                .map(|&c| (c.name(), counts.get(c)))
                .collect();
            let metric = json!({"id":id, "sample":i, "pixels":w*h,"max":max,"differentPixels":different,"renderUs":micros,"counts":counts});
            println!("{metric}");
            results.push(metric);
        }
    }
    std::fs::write(
        dir.join("native-parity.json"),
        serde_json::to_vec_pretty(&results)?,
    )?;
    if results.iter().any(|r| {
        r.get("chromiumError").is_some()
            || r.get("nativeError").is_some()
            || r["max"].as_u64().is_some_and(|v| v > 0)
    }) {
        Err("video parity has unresolved formats or pixels differing from Chromium; see native-parity.json".into())
    } else {
        Ok(())
    }
}
