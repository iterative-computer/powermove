//! Dump native planes at the exact timestamps captured by video-parity.mjs.
//! Evidence and synthetic sources remain outside the repository. This example
//! neither creates temporary directories nor writes into the source directory.
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::vt::DecodePurpose;
use std::{path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: video_parity_dump EVIDENCE_DIR");
    let dir = Path::new(&dir);
    let capture: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("chromium.json"))?)?;
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 0,
        ..Default::default()
    });
    for row in capture["rows"].as_array().expect("capture rows") {
        let id = row["id"].as_str().unwrap();
        let src = MediaSource::open(&dir.join(row["file"].as_str().unwrap()))?;
        let mut session = svc.session(&src, 1, DecodePurpose::Display)?;
        for (i, frame) in row["frames"].as_array().into_iter().flatten().enumerate() {
            let seconds = frame["timestamp"].as_f64().unwrap() / 1e6;
            let n = src
                .index
                .frames
                .iter()
                .rposition(|f| f.pts as f64 * src.index.timebase.to_f64() <= seconds + 1e-5)
                .expect("captured presentation timestamp");
            let started = Instant::now();
            let f = session.frame(n)?;
            let decode_us = started.elapsed().as_micros();
            let (ya, cc) = f.reference()?;
            std::fs::write(
                dir.join(format!("{id}-{i}.native.json")),
                serde_json::to_vec(&serde_json::json!({
                    "frame": n, "pts": f.pts, "format": &*f.format, "color": f.color,
                    "tagged": src.probe.video.as_ref().unwrap().color.tagged,
                    "decoder": format!("{:?}", session.info()), "decodeUs": decode_us, "ya": ya, "cc": cc,
                }))?,
            )?;
        }
    }
    Ok(())
}
