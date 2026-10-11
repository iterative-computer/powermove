//! Replays reference traces on wgpu and writes one PNG per recorded output.
//!
//!   s7-replay <ref-root> <out-root> [--hand] [scene ...]
use anyhow::{Context, Result};
use s7_replay::{Replayer, Trace, image::write_png};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let hand = args.iter().any(|a| a == "--hand");
    args.retain(|a| a != "--hand");
    let ref_root = PathBuf::from(args.first().context("ref root")?);
    let out_root = PathBuf::from(args.get(1).context("out root")?);
    let mut scenes: Vec<String> = args[2..].to_vec();
    if scenes.is_empty() {
        let mut all: Vec<String> = std::fs::read_dir(&ref_root)?.filter_map(|e| e.ok()).filter(|e| e.path().join("trace.json").exists()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        all.sort();
        scenes = all;
    }
    let (adapter, device, queue) = s7_replay::device()?;
    let errors: Arc<Mutex<Vec<String>>> = Arc::default();
    {
        let errors = errors.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| errors.lock().unwrap().push(format!("{e}"))));
    }
    eprintln!("adapter: {}", adapter.get_info().name);
    let mut summary = Vec::new();
    for scene in &scenes {
        let t0 = std::time::Instant::now();
        let trace = Trace::load(&ref_root.join(scene))?;
        let mut replayer = Replayer::new(device.clone(), queue.clone());
        if hand {
            replayer.set_override(Box::new(s7_replay::hand::override_standard));
        }
        let result = replayer.run(&trace);
        let out = out_root.join(scene);
        std::fs::create_dir_all(&out)?;
        let gpu_errors: Vec<String> = std::mem::take(&mut *errors.lock().unwrap());
        let mut entry = serde_json::json!({ "scene": scene, "stats": replayer.stats, "gpu_errors": gpu_errors, "ms": t0.elapsed().as_secs_f64() * 1000.0 });
        match result {
            Ok(frames) => {
                for f in &frames {
                    write_png(&out.join(format!("{}.png", f.name)), f.width, f.height, &f.rgba)?;
                }
                entry["frames"] = frames.iter().map(|f| f.name.clone()).collect::<Vec<_>>().into();
                eprintln!("{scene}: {} frames, {} draws, {} programs ({} failed), {} gpu errors, {:.0} ms", frames.len(), replayer.stats.draws, replayer.stats.programs, replayer.stats.program_failures.len(), entry["gpu_errors"].as_array().unwrap().len(), t0.elapsed().as_secs_f64() * 1000.0);
            }
            Err(e) => {
                eprintln!("{scene}: FAILED {e:#}");
                entry["error"] = format!("{e:#}").into();
            }
        }
        for f in &replayer.stats.program_failures {
            eprintln!("  program failure: {}", f.lines().next().unwrap_or(""));
        }
        for e in entry["gpu_errors"].as_array().unwrap().iter().take(3) {
            eprintln!("  gpu error: {}", e.as_str().unwrap_or("").lines().take(6).collect::<Vec<_>>().join(" | "));
        }
        std::fs::write(out.join("replay.json"), serde_json::to_string_pretty(&entry)?)?;
        summary.push(entry);
    }
    std::fs::write(out_root.join("replay-summary.json"), serde_json::to_string_pretty(&summary)?)?;
    Ok(())
}
