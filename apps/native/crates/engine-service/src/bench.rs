//! Windowless service scheduling diagnostic. Synthetic ticks, never scanout.
use pm_engine_service::{EngineService, Output, ViewerConfig};
use pm_protocol::{
    clock::TransportCommand,
    common::{PlaybackRate, TickRange, Ticks},
};
use serde_json::json;
use std::time::{Duration, Instant};
#[cfg(not(feature = "kernel"))]
use {pm_engine_service::Snapshot, std::sync::Arc};
pub fn run() -> anyhow::Result<()> {
    let arg = |name: &str, default: &str| {
        std::env::args()
            .find_map(|a| a.strip_prefix(&format!("--{name}=")).map(str::to_owned))
            .unwrap_or(default.into())
    };
    let hz: f64 = arg("hz", "120").parse()?;
    let seconds: f64 = arg("seconds", "60").parse()?;
    anyhow::ensure!(
        (1. ..=240.).contains(&hz) && (1. ..=300.).contains(&seconds),
        "invalid duration/rate"
    );
    let value = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json"))?;
    #[cfg(feature = "kernel")]
    let snapshot = pm_kernel::document::DocumentStore::new_untitled(value).snapshot();
    #[cfg(not(feature = "kernel"))]
    let snapshot = Snapshot {
        doc_id: 1,
        revision: 1,
        document: Arc::new(pm_model::Document::new(
            Some(pm_model::canonical::canonical_project(value)),
            1,
        )),
    };
    let service = EngineService::start(pm_gpu::Gpu::new("service-offscreen-diagnostic")?)?;
    let viewer = service.viewer(ViewerConfig {
        composition: snapshot.open_comp().unwrap(),
        snapshot,
        media: Default::default(),
        output: Output {
            width: 1280,
            height: 720,
            display: 0,
            headroom: 1.,
        },
        layer: None,
        rate: pm_time::Rate::FPS_30,
        end: None,
        audio: false,
    })?;
    viewer.command(TransportCommand::SetLoop {
        range: Some(TickRange {
            start: Ticks::ZERO,
            end: Ticks::from_whole_seconds(2),
        }),
    })?;
    viewer.command(TransportCommand::Play {
        rate: PlaybackRate::NORMAL,
    })?;
    let started = Instant::now();
    let mut next = started;
    let mut last = 0.;
    let mut costs = Vec::new();
    let mut failures = 0;
    while started.elapsed().as_secs_f64() < seconds + 1. {
        let now = Instant::now();
        if now >= next {
            viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
            next = now + Duration::from_secs_f64(1. / hz);
        }
        let status = viewer.status();
        anyhow::ensure!(status.error.is_none(), "{:?}", status.error);
        if let Some(f) = status.frame
            && let Some(completed) = f.completion_host_seconds
            && completed != last
        {
            last = completed;
            if started.elapsed().as_secs_f64() >= 1. {
                if let Some(ms) = f.worker_to_completion_ms {
                    costs.push(ms);
                }
                failures += u64::from(!f.gpu_ok);
            }
        }
        std::thread::sleep(Duration::from_micros(250));
    }
    costs.sort_by(f64::total_cmp);
    let p = |q: f64| {
        costs
            .get(((costs.len() as f64 * q).ceil() as usize).saturating_sub(1))
            .copied()
    };
    let result = json!({"mode":"offscreen","clock":"synthetic host ticks; NOT a display link","hz":hz,"seconds":seconds,"samples":costs.len(),"worker_to_completion_ms":{"p50":p(0.5),"p95":p(0.95)},"headroom_budget_ms":500./hz,"gpu_failures":failures,"skipped_targets":viewer.status().skipped_targets,"on_screen":"UNVERIFIED","build_profile":if cfg!(debug_assertions){"dev"}else{"release"}});
    std::fs::write(
        arg("report", "/tmp/es2-service-offscreen.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{result}");
    viewer.retire();
    Ok(())
}
