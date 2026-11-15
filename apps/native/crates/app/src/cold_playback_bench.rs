//! Explicitly invoked, windowless cold-open diagnostic. Never opens a real profile.
use super::*;
use pm_engine_service::{EngineService, Output, ViewerConfig};
use pm_protocol::{
    clock::TransportCommand,
    common::{PlaybackRate, TickRange, Ticks},
};
use std::time::{Duration, Instant};

pub(super) fn run() -> anyhow::Result<()> {
    let root = PathBuf::from(std::env::var("PM_COLD_ROOT")?);
    let mode = std::env::var("PM_COLD_MODE").unwrap_or_else(|_| "candidate".into());
    let profile = root.join(format!("profile-{mode}"));
    anyhow::ensure!(
        !profile.exists(),
        "benchmark requires an absent scratch profile"
    );
    std::fs::create_dir_all(profile.join("native/store"))?;
    std::fs::create_dir_all(profile.join("extension-import"))?;
    std::fs::write(
        profile.join("extension-import/preferences.json"),
        r#"{"declined":true}"#,
    )?;
    let started = Instant::now();
    let envelope: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("project.json"))?)?;
    let value = pm_model::json::parse(&serde_json::to_string(&envelope["proj"])?)?;
    let store = pm_kernel::DocumentStore::new_untitled(value);
    let snapshot = store.snapshot();
    let mut original = MediaCatalog::default();
    original.originals = serde_json::from_slice(&std::fs::read(root.join("media.json"))?)?;
    let assets = original.originals.keys().cloned().collect::<Vec<_>>();
    let service_media = ProMedia::new(profile, root.join("media"), Arc::new(AtomicBool::new(true)));
    let baseline = mode == "baseline";
    service_media
        .inner
        .baseline
        .store(baseline, Ordering::Release);
    if !baseline {
        service_media.wait_for_first_frame();
        service_media.prioritize("cold", &snapshot, 0.);
    }
    service_media.identify(
        "cold".into(),
        original
            .originals
            .iter()
            .map(|(id, p)| (id.clone(), p.clone()))
            .collect(),
    );
    let mut font_service = pm_text::host::FontService::new().map_err(anyhow::Error::msg)?;
    let fonts = pm_text::host::FontBroker::new();
    fonts.set_wake(Arc::new(|| {}));
    fonts.set_generation(font_service.generation());
    let engine = EngineService::start_with_fonts(
        pm_gpu::Gpu::new("cold-playback-diagnostic")?,
        pm_render::text::host::RenderFonts::new(fonts.clone()),
        None,
    )?;
    let comp_id = snapshot.render_comp();
    let c = pm_eval::Comp::resolve(&snapshot.document, &comp_id)
        .map_err(|_| anyhow::anyhow!("comp unavailable"))?;
    let duration = pm_eval::node::number(c.member("dur")).unwrap_or(1.);
    let rate = c.rate().map(|r| r.rate).unwrap_or(pm_time::Rate::FPS_30);
    let viewer = engine.viewer(ViewerConfig {
        composition: comp_id,
        snapshot: snapshot.clone(),
        media: original.clone(),
        output: Output {
            width: 1080,
            height: 1920,
            display: 0,
            headroom: 1.,
            crop: None,
        },
        layer: None,
        rate,
        end: None,
        audio: false,
    })?;
    let definitions: pm_protocol::definitions::DefinitionSetSnapshot =
        serde_json::from_slice(&std::fs::read(root.join("../definitions.json"))?)?;
    viewer.resources(Arc::new(pm_engine_service::resources::ResourceSnapshot {
        generation: 1,
        definitions: definitions.definitions,
        ..Default::default()
    }))?;
    viewer.quality(pm_engine_service::policy::Quality::Auto)?;
    viewer.command(TransportCommand::SetLoop {
        range: Some(TickRange {
            start: Ticks::ZERO,
            end: Ticks::new((duration * pm_protocol::common::TICKS_PER_SECOND as f64) as i64)?,
        }),
    })?;
    viewer.scrub(Ticks::ZERO)?;
    viewer.command(TransportCommand::Play {
        rate: PlaybackRate::NORMAL,
    })?;
    let mut bins = vec![(0u64, 0u64, 0u64); 30];
    let mut costs = Vec::new();
    let mut last = None;
    let mut last_time = None;
    let mut completions = 0u64;
    let mut first = None;
    let mut next = Instant::now();
    let mut next_media = Instant::now();
    let mut catalog = original.clone();
    let mut ready = BTreeMap::new();
    let mut peak_jobs = 0;
    while started.elapsed().as_secs_f64() < 30. {
        pm_text::host::serve(&mut font_service, fonts.take_asks());
        let now = Instant::now();
        if now >= next_media {
            let mut updated = original.clone();
            service_media.decorate("cold", &mut updated, &assets);
            for id in updated.proxies.keys() {
                ready
                    .entry(id.clone())
                    .or_insert(started.elapsed().as_secs_f64());
            }
            if updated != catalog {
                viewer.media(updated.clone())?;
                catalog = updated;
            }
            let status = viewer.status();
            let ticks = status
                .clock
                .as_ref()
                .map(|c| c.media_time.get())
                .or_else(|| status.frame.as_ref().map(|f| f.time.get()))
                .unwrap_or(0);
            let time = ticks as f64 / pm_protocol::common::TICKS_PER_SECOND as f64;
            if !baseline {
                service_media.prioritize("cold", &snapshot, time);
            }
            peak_jobs = peak_jobs.max(
                service_media
                    .snapshot()
                    .iter()
                    .filter(|t| t.state == TaskState::Running)
                    .count(),
            );
            next_media = now + Duration::from_millis(100);
        }
        if now >= next {
            viewer.display_tick(pm_engine_service::trace::now() + 1. / 120., 1. / 120.)?;
            next = now + Duration::from_secs_f64(1. / 120.);
        }
        let status = viewer.status();
        anyhow::ensure!(status.error.is_none(), "{:?}", status.error);
        if let Some(f) = status.frame
            && f.completion_host_seconds.is_some()
            && f.completion_host_seconds != last
        {
            last = f.completion_host_seconds;
            completions += 1;
            let target = f.time.get()
                / rate
                    .exact_frame_ticks()
                    .unwrap_or(pm_protocol::common::TICKS_PER_SECOND / 30);
            if last_time == Some(target) {
                continue;
            }
            last_time = Some(target);
            let elapsed = started.elapsed().as_secs_f64();
            first.get_or_insert(elapsed);
            service_media.first_frame_ready();
            if let Some(bin) = bins.get_mut(elapsed.floor() as usize) {
                bin.0 += 1;
                bin.1 += u64::from(f.stand_ins);
                bin.2 += u64::from(!f.gpu_ok);
                if let Some(ms) = f.worker_to_completion_ms {
                    costs.push(ms);
                }
            }
        }
        std::thread::sleep(Duration::from_micros(250));
    }
    viewer.command(TransportCommand::Pause)?;
    for task in service_media.snapshot() {
        if !task.state.terminal() {
            let _ = service_media.act(task.id, TaskAction::Cancel);
        }
    }
    costs.sort_by(f64::total_cmp);
    let frames = bins.iter().map(|b| b.0).sum::<u64>();
    let percentile = |q: f64| {
        costs
            .get(((costs.len() as f64 * q).ceil() as usize).saturating_sub(1))
            .copied()
    };
    let report = serde_json::json!({"mode":mode,"seconds":30,"completed_renders":completions,"frames":frames,"fps":frames as f64/30.,
        "missed_slots":900u64.saturating_sub(frames),"first_frame_s":first,"p50_ms":percentile(0.5),"p95_ms":percentile(0.95),
        "bins":bins.iter().enumerate().map(|(i,b)|serde_json::json!({"second":i,"frames":b.0,"missed":30u64.saturating_sub(b.0),"stand_ins":b.1,"gpu_failures":b.2})).collect::<Vec<_>>(),
        "preview_ready_s":ready,"peak_jobs":peak_jobs});
    std::fs::write(
        root.join(format!("{mode}.json")),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
