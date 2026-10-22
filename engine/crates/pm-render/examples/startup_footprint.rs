//! Attribution probe: this process's settled physical footprint (graphics
//! and the rest, from `footprint`) after each step of bringing up a viewer
//! renderer, the native counterpart of the interactive utility at rest.
//!
//!   cargo run --release -p pm-render --example startup_footprint -- <document.json> [width height]
//!
//! The document is an asset-free generated corpus document read at run
//! time. Each checkpoint waits for the GPU's active-context charge to
//! lapse (about 1.5 s after the last submission). Writes nothing.

use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use std::process::Command;
use std::time::Duration;

/// (footprint, graphics) in MiB.
fn footprint() -> (f64, f64) {
    let out = Command::new("footprint")
        .args(["-p", &std::process::id().to_string(), "-f", "bytes"])
        .output()
        .expect("footprint");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut total = 0u64;
    let mut graphics = 0u64;
    for line in text.lines() {
        if let Some(rest) = line.split("Footprint:").nth(1) {
            total = rest.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
        }
        if line.contains("(graphics)") {
            graphics += line
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse::<u64>()
                .unwrap_or(0);
        }
    }
    (total as f64 / 1048576.0, graphics as f64 / 1048576.0)
}

fn checkpoint(name: &str, device: Option<&wgpu::Device>) {
    if let Some(device) = device {
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }
    std::thread::sleep(Duration::from_millis(2500));
    let (total, graphics) = footprint();
    let allocated = device
        .and_then(pm_gpu::device::allocated_bytes)
        .map_or("-".into(), |b| format!("{:.1}", b as f64 / 1048576.0));
    println!(
        "{name:<40} footprint={total:7.1} MiB graphics={graphics:6.1} other={:6.1} metal-allocated={allocated}",
        total - graphics
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a document.json path");
    let width: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1496);
    let height: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(904);
    let source = std::fs::read_to_string(&path).expect("document");
    let value = pm_model::json::parse(&source).expect("document JSON");
    let root = match &value {
        pm_model::Value::Object(o) => o
            .get("compId")
            .and_then(|v| match v {
                pm_model::Value::String(s) => Some(s.to_string()),
                _ => None,
            })
            .expect("a root composition id"),
        _ => panic!("a document object"),
    };
    let document = pm_model::Document::new(Some(value), 1);
    checkpoint("process start", None);
    let gpu = pm_gpu::Gpu::new("startup footprint probe").expect("Metal device");
    checkpoint("device", Some(&gpu.device));
    let mut renderer = pm_render::Renderer::new(gpu.device.clone(), gpu.queue.clone());
    checkpoint("renderer", Some(&gpu.device));
    let resolver = pm_render::MapResolver::default();
    let request = pm_render::RenderRequest::at_scale(
        Size::new(width, height),
        1.0,
        Precision::Float16,
    );
    let frame = renderer
        .render(&document, &root, Ticks::new(0).unwrap(), &request, &resolver)
        .expect("frame");
    println!("failures: {:?}", frame.failures);
    drop(frame);
    checkpoint("first frame (dropped)", Some(&gpu.device));
    for i in 1..10 {
        let t = Ticks::new(i * pm_time::TICKS_PER_SECOND / 10).unwrap();
        drop(renderer.render(&document, &root, t, &request, &resolver));
    }
    checkpoint("ten frames", Some(&gpu.device));
    drop(renderer);
    checkpoint("renderer dropped", Some(&gpu.device));
}
