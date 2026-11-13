//! Windowless service scheduling diagnostic. Synthetic ticks, never scanout.
//!
//! `pm-engine-service --bench [--hz=60] [--seconds=60]` plays a fixture;
//! `--project=COPY.pmv [--definitions=FILE] [--width=1920 --height=1080]`
//! plays a real project (a private copy: its embedded media are extracted to
//! a scratch folder) and then seeks it at random, as the viewer does.
//! `--from=SECONDS` starts the run there instead of at the start;
//! `--loop=SECONDS` loops that much of it for the whole run;
//! `--quality=full|half|quarter` pins the resolution (Auto by default);
//! `--skim=SPEED` sets the skim sweep's media seconds per second (1);
//! `--fill=DIR` first renders the played range into a disk render cache in
//! `DIR` (Render Selection's child) and reports its bytes and read cost.
use pm_engine_service::{EngineService, MediaCatalog, Output, ViewerConfig};
use pm_protocol::{
    clock::TransportCommand,
    common::{PlaybackRate, TickRange, Ticks},
};
use serde_json::json;
use std::time::{Duration, Instant};
#[cfg(not(feature = "kernel"))]
use {pm_engine_service::Snapshot, std::sync::Arc};

pub(crate) fn arg(name: &str, default: &str) -> String {
    std::env::args()
        .find_map(|a| a.strip_prefix(&format!("--{name}=")).map(str::to_owned))
        .unwrap_or(default.into())
}

/// Media seconds a skim sweep covers.
const SKIM_SPAN_SECONDS: f64 = 2.;

/// Pointer events (60 Hz) for a skim sweep over `span` media seconds at
/// `speed` media seconds per wall second: the sweep takes `span / speed`.
fn skim_events(speed: f64, span: f64) -> usize {
    ((span / speed.max(1e-3)) * 60.).round().max(1.) as usize
}

/// Frames of a composition `duration` seconds long at `rate`.
fn comp_frames(duration: f64, rate: pm_time::Rate) -> i64 {
    (duration * rate.to_f64()).floor() as i64
}

fn percentile(sorted: &[f64], q: f64) -> Option<f64> {
    sorted
        .get(((sorted.len() as f64 * q).ceil() as usize).saturating_sub(1))
        .copied()
}

/// Gate: what the frames that completed a phase's samples were made of.
#[derive(Default, Clone, Copy)]
struct Tally {
    frames: u64,
    stand_ins: u64,
    reduced: u64,
    cache_hits: u64,
    gpu_bad: u64,
}
impl Tally {
    fn add(&mut self, f: &pm_engine_service::FrameReport, requested: f64) {
        self.frames += 1;
        self.stand_ins += u64::from(f.stand_ins);
        self.reduced += u64::from(f.scale + 1e-9 < requested);
        self.cache_hits += u64::from(f.cache_hit);
        self.gpu_bad += u64::from(!f.gpu_ok);
    }
    fn json(&self) -> serde_json::Value {
        json!({"frames":self.frames,"stand_ins":self.stand_ins,"reduced_resolution":self.reduced,"cache_hits":self.cache_hits,"gpu_failures":self.gpu_bad})
    }
}

/// How long the kernel took to open `--project` (µs): what a window waits
/// for before its first frame, without media extraction.
static OPEN_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The fixture or the project copy named by `--project`, with its media.
#[cfg(feature = "kernel")]
pub(crate) fn scene() -> anyhow::Result<(
    pm_engine_service::Snapshot,
    MediaCatalog,
    f64,
    pm_time::Rate,
    Option<pm_kernel::DocumentStore>,
)> {
    let fixture = arg("fixture", "");
    if !fixture.is_empty() {
        let dir = std::path::Path::new(&fixture);
        let store = pm_kernel::DocumentStore::open(pm_kernel::open_project(
            &dir.join("project.pmv"),
            Default::default(),
        )?);
        let snapshot = store.snapshot();
        let paths: std::collections::BTreeMap<String, String> =
            serde_json::from_slice(&std::fs::read(dir.join("media.json"))?)?;
        let mut media = MediaCatalog::default();
        for (id, path) in paths {
            media.originals.insert(id, dir.join(path));
        }
        return Ok((snapshot, media, 20., pm_time::Rate::FPS_30, Some(store)));
    }
    let project = arg("project", "");
    if project.is_empty() {
        let value = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json"))?;
        let snapshot = pm_kernel::document::DocumentStore::new_untitled(value).snapshot();
        return Ok((
            snapshot,
            MediaCatalog::default(),
            2.,
            pm_time::Rate::FPS_30,
            None,
        ));
    }
    let opening = Instant::now();
    let opened = pm_kernel::open_project(std::path::Path::new(&project), Default::default())?;
    let store = pm_kernel::document::DocumentStore::open(opened);
    OPEN_US.store(
        opening.elapsed().as_micros() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    let media_dir = scratch_dir();
    std::fs::create_dir_all(&media_dir)?;
    let mut media = MediaCatalog::default();
    for (i, entry) in store.file().media.iter().enumerate() {
        let dest = media_dir.join(format!("embedded-{i}"));
        entry.copy_to(&dest)?;
        media.originals.insert(entry.id.to_string(), dest);
    }
    let snapshot = store.snapshot();
    let comp = snapshot
        .open_comp()
        .ok_or_else(|| anyhow::anyhow!("the project has no open composition"))?;
    // Kernel snapshots can keep the open composition at the root. Resolve
    // it exactly as the renderer does; indexing only comps[compId] silently
    // reduced such projects to a one-second benchmark loop.
    let c = pm_eval::Comp::resolve(&snapshot.document, &comp)
        .map_err(|_| anyhow::anyhow!("composition unavailable"))?;
    let duration = pm_eval::node::number(c.member("dur"))
        .filter(|d| *d > 0.)
        .unwrap_or(1.);
    let rate = c.rate().map(|r| r.rate).unwrap_or(pm_time::Rate::FPS_30);
    Ok((snapshot, media, duration, rate, Some(store)))
}

#[cfg(not(feature = "kernel"))]
pub(crate) fn scene() -> anyhow::Result<(Snapshot, MediaCatalog, f64, pm_time::Rate, Option<()>)> {
    anyhow::ensure!(
        arg("project", "").is_empty() && arg("fixture", "").is_empty(),
        "project and corpus benchmarks require the kernel feature"
    );
    let value = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json"))?;
    let snapshot = Snapshot {
        doc_id: 1,
        revision: 1,
        document: Arc::new(pm_model::Document::new(
            Some(pm_model::canonical::canonical_project(value)),
            1,
        )),
    };
    Ok((
        snapshot,
        MediaCatalog::default(),
        2.,
        pm_time::Rate::FPS_30,
        None,
    ))
}

/// Where the bench extracts a project's embedded media for one run.
fn scratch_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pm-engine-bench-{}", std::process::id()))
}

/// Removes the run's extracted media however the run ends.
pub(crate) struct Scratch;
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(scratch_dir());
    }
}

/// Benchmark-only: decode counters and cache usage at each phase start.
static DIAG: std::sync::Mutex<Vec<serde_json::Value>> = std::sync::Mutex::new(Vec::new());
fn diag_mark(phase: &str, service: &EngineService) {
    let counters: serde_json::Map<String, serde_json::Value> = pm_media::decode::diag::snapshot()
        .into_iter()
        .map(|(k, v)| (k.to_owned(), json!(v)))
        .collect();
    let usage = serde_json::to_value(service.memory_usage()).unwrap_or_default();
    let lease = json!({
        "budget": pm_memory::budget(),
        "clients": pm_memory::clients()
            .into_iter()
            .map(|(weight, grant, reported)| json!({"weight":weight,"grant":grant,"reported":reported}))
            .collect::<Vec<_>>(),
        "reported": serde_json::to_value(pm_memory::usage()).unwrap_or_default(),
    });
    DIAG.lock()
        .unwrap()
        .push(json!({"phase":phase,"decode":counters,"usage":usage,"lease":lease}));
}

pub fn run() -> anyhow::Result<()> {
    let _scratch = Scratch;
    let hz: f64 = arg("hz", "120").parse()?;
    let seconds: f64 = arg("seconds", "60").parse()?;
    let width: u32 = arg("width", "1280").parse()?;
    let height: u32 = arg("height", "720").parse()?;
    anyhow::ensure!(
        (1. ..=240.).contains(&hz) && (1. ..=300.).contains(&seconds),
        "invalid duration/rate"
    );
    let (snapshot, media, duration, rate, _store) = scene()?;
    // The font registry answers on this thread, as the app's main thread does.
    let mut font_service = pm_text::host::FontService::new().map_err(anyhow::Error::msg)?;
    let fonts = pm_text::host::FontBroker::new();
    fonts.set_wake(std::sync::Arc::new(|| {}));
    fonts.set_generation(font_service.generation());
    let service = EngineService::start_with_fonts(
        pm_gpu::Gpu::new("service-offscreen-diagnostic")?,
        pm_render::text::host::RenderFonts::new(fonts.clone()),
        None,
    )?;
    let viewer = service.viewer(ViewerConfig {
        composition: snapshot
            .open_comp()
            .ok_or_else(|| anyhow::anyhow!("no composition"))?,
        snapshot,
        media,
        output: Output {
            width,
            height,
            display: 0,
            headroom: 1.,
            crop: None,
        },
        layer: None,
        rate,
        end: None,
        audio: false,
    })?;
    let definitions = arg("definitions", "");
    if !definitions.is_empty() {
        let set: pm_protocol::definitions::DefinitionSetSnapshot =
            serde_json::from_slice(&std::fs::read(&definitions)?)?;
        viewer.resources(std::sync::Arc::new(
            pm_engine_service::resources::ResourceSnapshot {
                generation: 1,
                definitions: set.definitions,
                ..Default::default()
            },
        ))?;
    }
    // `--quality=full|half|quarter` pins the resolution (Auto by default),
    // so two builds compare the same frames under different machine load.
    viewer.adapt_drags(true)?;
    viewer.quality(match arg("quality", "auto").as_str() {
        "full" => pm_engine_service::policy::Quality::Full,
        "half" => pm_engine_service::policy::Quality::Half,
        "quarter" => pm_engine_service::policy::Quality::Quarter,
        _ => pm_engine_service::policy::Quality::Auto,
    })?;
    // A frame drawn before playing settles fonts, definitions and media.
    let settle = Instant::now();
    viewer.scrub(Ticks::ZERO)?;
    while viewer.status().frame.is_none() && settle.elapsed() < Duration::from_secs(120) {
        pm_text::host::serve(&mut font_service, fonts.take_asks());
        viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
        std::thread::sleep(Duration::from_millis(2));
    }
    let first_frame_s = settle.elapsed().as_secs_f64();
    let from: f64 = arg("from", "0").parse()?;
    anyhow::ensure!(
        (0. ..duration).contains(&from),
        "--from is outside the composition"
    );
    // `--loop=SECONDS` plays a shorter range over and over for the whole run
    // (memory over a long playback); by default the range is the run.
    let span: f64 = arg("loop", &seconds.to_string()).parse()?;
    let start = Ticks::from_whole_seconds(from.floor() as i32);
    let end =
        Ticks::from_whole_seconds((from.floor() + span.min(seconds)).min(duration).ceil() as i32);
    if start != Ticks::ZERO {
        viewer.scrub(start)?;
    }
    viewer.command(TransportCommand::SetLoop {
        range: Some(TickRange { start, end }),
    })?;
    let fill = match arg("fill", "").as_str() {
        "" => serde_json::Value::Null,
        dir => fill_range(
            &viewer,
            &service,
            &mut font_service,
            &fonts,
            dir,
            start,
            end,
            hz,
        )?,
    };
    viewer.command(TransportCommand::Play {
        rate: PlaybackRate::NORMAL,
    })?;
    let skipped_before = viewer.status().skipped_targets;
    diag_mark("playback", &service);
    println!("PHASE playback");
    let started = Instant::now();
    let mut next = started;
    let mut late_ticks = 0u64;
    let mut missed_ticks = 0u64;
    let mut last = 0.;
    let mut costs = Vec::new();
    let (mut failures, mut stand_ins, mut lowest_quality) = (0, 0, 1f64);
    while started.elapsed().as_secs_f64() < seconds + 1. {
        pm_text::host::serve(&mut font_service, fonts.take_asks());
        let now = Instant::now();
        if now >= next {
            let behind = now.duration_since(next).as_secs_f64();
            late_ticks += u64::from(behind > 1. / hz);
            missed_ticks += (behind * hz).floor() as u64;
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
                stand_ins += u64::from(f.stand_ins);
                lowest_quality = lowest_quality.min(status.effective_quality);
            }
        }
        std::thread::sleep(Duration::from_micros(250));
    }
    let status = viewer.status();
    viewer.command(TransportCommand::Pause)?;
    costs.sort_by(f64::total_cmp);
    // Seeks: input to a completed frame at the new time, as the viewer scrubs.
    // Gate: the resolution the quality setting asks for when stopped.
    let requested = match arg("quality", "auto").as_str() {
        "half" => 0.5,
        "quarter" => 0.25,
        _ => 1.,
    };
    let mut seek_tally = Tally::default();
    let mut seek_ats = Vec::new();
    diag_mark("seek", &service);
    println!("PHASE seek");
    let mut seeks = Vec::new();
    let mut stuck = Vec::new();
    let mut seek_stand_ins = 0u64;
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let frame = rate
        .exact_frame_ticks()
        .unwrap_or(pm_protocol::common::TICKS_PER_SECOND / 30);
    for _ in 0..if arg("project", "").is_empty() { 0 } else { 30 } {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let t = duration * ((seed >> 33) as f64 / (1u64 << 31) as f64);
        let raw = (t * pm_protocol::common::TICKS_PER_SECOND as f64) as i64;
        let at = Ticks::new(raw / frame * frame)?;
        let input = Instant::now();
        viewer.scrub(at)?;
        loop {
            pm_text::host::serve(&mut font_service, fonts.take_asks());
            viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
            let status = viewer.status();
            if status
                .frame
                .as_ref()
                .is_some_and(|f| f.time == at && f.completion_host_seconds.is_some())
            {
                seeks.push(input.elapsed().as_secs_f64() * 1000.);
                seek_stand_ins += u64::from(status.frame.as_ref().unwrap().stand_ins);
                seek_tally.add(status.frame.as_ref().unwrap(), requested);
                seek_ats.push(at);
                break;
            }
            if input.elapsed() > Duration::from_secs(10) {
                stuck.push(json!({"at":at.get(),"frame":status.frame.map(|f| f.time.get()),"error":status.error}));
                break;
            }
            std::thread::sleep(Duration::from_micros(250));
        }
    }
    seeks.sort_by(f64::total_cmp);
    // A playhead drag: every frame of two seconds forward, then back over
    // them (media and glyphs warm), then among the last few (the service's
    // frame cache): input to a completed frame at the new time each.
    // Benchmark-only: per sweep, how many answering frames were stand-ins
    // or frame-cache hits (`sweep_counts`, in sweep order).
    let mut sweep_counts = Vec::new();
    // Gate: per sweep, the tally with reduced-resolution / GPU failures, and stuck scrubs.
    let mut gate_sweeps: Vec<(Tally, u64)> = Vec::new();
    let mut sweep = |times: &mut dyn Iterator<Item = i64>| -> anyhow::Result<Vec<f64>> {
        let mut out = Vec::new();
        let (mut stand_ins, mut hits) = (0u64, 0u64);
        let mut tally = Tally::default();
        let mut stuck_sweep = 0u64;
        for t in times {
            let at = Ticks::new(t)?;
            let input = Instant::now();
            viewer.scrub(at)?;
            loop {
                pm_text::host::serve(&mut font_service, fonts.take_asks());
                viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
                let status = viewer.status();
                if status
                    .frame
                    .as_ref()
                    .is_some_and(|f| f.time == at && f.completion_host_seconds.is_some())
                {
                    out.push(input.elapsed().as_secs_f64() * 1000.);
                    let f = status.frame.as_ref().unwrap();
                    stand_ins += u64::from(f.stand_ins);
                    hits += u64::from(f.cache_hit);
                    tally.add(f, requested);
                    break;
                }
                // Gate: a stuck scrub is counted, not fatal.
                if input.elapsed() >= Duration::from_secs(10) {
                    stuck_sweep += 1;
                    break;
                }
                std::thread::sleep(Duration::from_micros(100));
            }
        }
        sweep_counts.push(json!({"n":out.len(),"stand_ins":stand_ins,"cache_hits":hits}));
        gate_sweeps.push((tally, stuck_sweep));
        out.sort_by(f64::total_cmp);
        Ok(out)
    };
    let scrub_from = (from * pm_protocol::common::TICKS_PER_SECOND as f64) as i64 / frame * frame;
    let frames = (2. * rate.to_f64()).round() as i64;
    diag_mark("scrub_forward", &service);
    println!("PHASE scrub_forward");
    let forward = sweep(&mut (0..frames).map(|i| scrub_from + i * frame))?;
    diag_mark("scrub_back", &service);
    println!("PHASE scrub_back");
    let back = sweep(&mut (0..frames).rev().map(|i| scrub_from + i * frame))?;
    diag_mark("scrub_cached", &service);
    println!("PHASE scrub_cached");
    let cached = sweep(&mut (0..24).map(|i| scrub_from + (i % 4) * frame))?;
    // The playhead stops: its frame is drawn exact, at the requested
    // resolution (Auto drags a slow project at reduced resolution).
    let last = Ticks::new(scrub_from + 3 * frame)?;
    let stopped = Instant::now();
    let settle_ms = loop {
        pm_text::host::serve(&mut font_service, fonts.take_asks());
        viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
        let status = viewer.status();
        if status.effective_quality == requested
            && status
                .frame
                .as_ref()
                .is_some_and(|f| f.time == last && f.completion_host_seconds.is_some())
        {
            break stopped.elapsed().as_secs_f64() * 1000.;
        }
        anyhow::ensure!(
            stopped.elapsed() < Duration::from_secs(10),
            "the stopped frame was never drawn at the requested quality"
        );
        std::thread::sleep(Duration::from_micros(100));
    };
    // Gate: the settled frame must also be a real one: no stand-in, the
    // requested resolution, GPU work completed, no layer left out.
    let settle_report = {
        let waited = Instant::now();
        loop {
            pm_text::host::serve(&mut font_service, fonts.take_asks());
            viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
            let status = viewer.status();
            let f = status.frame.clone();
            let exact = f.as_ref().is_some_and(|f| {
                f.time == last
                    && f.completion_host_seconds.is_some()
                    && !f.stand_ins
                    && f.gpu_ok
                    && f.scale + 1e-9 >= requested
            }) && status.layer_failures.is_empty();
            if exact || waited.elapsed() > Duration::from_secs(10) {
                break json!({
                    "exact": exact,
                    "extra_wait_ms": waited.elapsed().as_secs_f64() * 1000.,
                    "stand_ins": f.as_ref().map(|f| f.stand_ins),
                    "scale": f.as_ref().map(|f| f.scale),
                    "requested_scale": requested,
                    "gpu_ok": f.as_ref().map(|f| f.gpu_ok),
                    "layer_failures": status.layer_failures.len(),
                });
            }
            std::thread::sleep(Duration::from_micros(100));
        }
    };
    // Skimming: pointer events at 60 Hz move the skimmer at `--skim` media
    // seconds per wall second over two media seconds forward, then back over
    // them; each frame shown counts from the event that asked for its time.
    let skim_speed: f64 = arg("skim", "1").parse()?;
    let mut skim_counts = Vec::new();
    let mut skim_pass = |forward: bool| -> anyhow::Result<Vec<f64>> {
        let events = skim_events(skim_speed, SKIM_SPAN_SECONDS);
        let span = (SKIM_SPAN_SECONDS * pm_protocol::common::TICKS_PER_SECOND as f64) as i64;
        let mut asked: std::collections::HashMap<i64, f64> = Default::default();
        let mut out = Vec::new();
        let mut delivered = std::collections::HashSet::new();
        let mut delivered_tally = Tally::default();
        let mut skim_stand_ins = 0u64;
        let mut seen = None;
        let started = Instant::now();
        let mut collect = |out: &mut Vec<f64>,
                           asked: &std::collections::HashMap<i64, f64>,
                           seen: &mut Option<(i64, u64)>| {
            if let Some(f) = viewer.status().frame
                && let Some(done) = f.completion_host_seconds
                && seen != &Some((f.time.get(), f.stamp.sequence))
            {
                *seen = Some((f.time.get(), f.stamp.sequence));
                if let Some(input) = asked.get(&f.time.get()) {
                    out.push((done - input) * 1000.);
                    if delivered.insert(f.time.get()) {
                        delivered_tally.add(&f, requested);
                    }
                    skim_stand_ins += u64::from(f.stand_ins);
                }
            }
        };
        for i in 0..=events {
            let due = started + Duration::from_secs_f64(i as f64 / 60.);
            while Instant::now() < due {
                pm_text::host::serve(&mut font_service, fonts.take_asks());
                viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
                collect(&mut out, &asked, &mut seen);
                std::thread::sleep(Duration::from_micros(200));
            }
            let along = i as f64 / events as f64;
            let offset = (along * span as f64) as i64;
            let t = if forward {
                scrub_from + offset
            } else {
                scrub_from + span - offset
            };
            let at = Ticks::new(t.max(0) / frame * frame)?;
            let input = pm_engine_service::trace::now();
            asked.entry(at.get()).or_insert(input);
            viewer.skim(Some(at), input)?;
        }
        let rest = Instant::now();
        while rest.elapsed() < Duration::from_millis(400) {
            pm_text::host::serve(&mut font_service, fonts.take_asks());
            viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
            collect(&mut out, &asked, &mut seen);
            std::thread::sleep(Duration::from_micros(200));
        }
        drop(collect);
        skim_counts.push(json!({"requested_unique":asked.len(),"delivered_unique":delivered.len(),"missed_unique":asked.len().saturating_sub(delivered.len()),"stand_ins":skim_stand_ins,"delivered_frames":delivered_tally.json()}));
        out.sort_by(f64::total_cmp);
        Ok(out)
    };
    diag_mark("skim_forward", &service);
    println!("PHASE skim_forward");
    let skim_forward = skim_pass(true)?;
    diag_mark("skim_back", &service);
    println!("PHASE skim_back");
    let skim_back = skim_pass(false)?;
    viewer.skim(None, pm_engine_service::trace::now())?;
    // Gate: pixel-exact check. At the settle frame and the first random-seek
    // positions the viewer is asked for a capture, which the engine delivers
    // only from an exact frame (paused, full resolution, no stand-in). The
    // pixels are fingerprinted so two builds can be compared frame by frame.
    // Benchmark-only `--drag=1`: a playhead drag as the app sends it, one
    // scrub every 4 ms (250 Hz pointer), moving `--drag-fps` frames per second
    // (120) for two seconds, never waiting for a frame. Counts distinct
    // pictures completed during the drag and how long each position took.
    // Benchmark-only `--drag=1`: playhead drags as the app sends them, one
    // scrub every 4 ms (250 Hz pointer), `--drag-fps` frames per second
    // (120) for two seconds forward from `--from`, then two seconds back,
    // never waiting for a frame. Counts distinct pictures completed during
    // each drag and how long each position took to appear.
    let drag = if arg("drag", "0") == "1" {
        let fps: f64 = arg("drag-fps", "120").parse()?;
        let total = 2.0f64;
        let frames_total = (comp_frames(duration, rate)).max(1);
        let span = ((total * fps) as i64).min(frames_total - 1);
        let mut out = serde_json::Map::new();
        for back in [false, true] {
            let name = if back { "drag_back" } else { "drag" };
            diag_mark(name, &service);
            println!("PHASE {name}");
            let stale_before = viewer.status().stale_rejections;
            let started = Instant::now();
            let mut asked: std::collections::HashMap<i64, Instant> = Default::default();
            let mut shown: Vec<(f64, i64, bool)> = Vec::new();
            let mut lat = Vec::new();
            let mut last_seq = None;
            let mut next_event = started;
            while started.elapsed().as_secs_f64() < total + 0.5 {
                pm_text::host::serve(&mut font_service, fonts.take_asks());
                let now = Instant::now();
                if now >= next_event && started.elapsed().as_secs_f64() < total {
                    let k = ((started.elapsed().as_secs_f64() * fps) as i64).min(span);
                    let k = if back { span - k } else { k };
                    let at = Ticks::new(scrub_from + k * frame)?;
                    asked.entry(at.get()).or_insert(now);
                    viewer.scrub(at)?;
                    next_event = now + Duration::from_millis(4);
                }
                viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
                if let Some(f) = viewer.status().frame
                    && f.completion_host_seconds.is_some()
                    && last_seq != Some((f.stamp.sequence, f.time.get()))
                {
                    last_seq = Some((f.stamp.sequence, f.time.get()));
                    let t = started.elapsed().as_secs_f64();
                    if shown.last().is_none_or(|s| s.1 != f.time.get()) {
                        shown.push((t, f.time.get(), f.stand_ins));
                        if let Some(a) = asked.get(&f.time.get()) {
                            lat.push(a.elapsed().as_secs_f64() * 1000.);
                        }
                    }
                }
                std::thread::sleep(Duration::from_micros(250));
            }
            lat.sort_by(f64::total_cmp);
            let during: Vec<_> = shown.iter().filter(|s| s.0 < total).collect();
            let mut gaps: Vec<f64> = during
                .windows(2)
                .map(|w| (w[1].0 - w[0].0) * 1000.)
                .collect();
            gaps.sort_by(f64::total_cmp);
            let stand = during.iter().filter(|s| s.2).count();
            out.insert(name.into(), json!({"fps":fps,"seconds":total,"positions_asked":asked.len(),
                "distinct_shown_during":during.len(),"distinct_per_s":during.len() as f64 / total,
                "exact_per_s":(during.len() - stand) as f64 / total,
                "stand_ins_shown":stand,
                "stale_rejections":viewer.status().stale_rejections - stale_before,
                "position_to_picture_ms":{"n":lat.len(),"p50":percentile(&lat,0.5),"p95":percentile(&lat,0.95)},
                "longest_gap_ms":gaps.last()}));
        }
        // The allowance returns after a drag: footprint and cache bytes 3 s on.
        diag_mark("drag_after", &service);
        println!("PHASE drag_after");
        let rest = Instant::now();
        while rest.elapsed() < Duration::from_secs(3) {
            pm_text::host::serve(&mut font_service, fonts.take_asks());
            viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
            std::thread::sleep(Duration::from_millis(5));
        }
        diag_mark("drag_after3s", &service);
        // Keep the original single-drag fields for earlier readers.
        let mut forward = out.get("drag").cloned().unwrap_or_default();
        if let Some(m) = forward.as_object_mut() {
            m.insert(
                "back".into(),
                out.get("drag_back").cloned().unwrap_or_default(),
            );
        }
        forward
    } else {
        serde_json::Value::Null
    };
    diag_mark("exact", &service);
    println!("PHASE exact");
    let mut exact_frames = Vec::new();
    let mut positions = vec![last];
    positions.extend(seek_ats.iter().take(5).copied());
    for at in positions {
        let started = Instant::now();
        viewer.scrub(at)?;
        let mut asked = false;
        let item = loop {
            pm_text::host::serve(&mut font_service, fonts.take_asks());
            viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
            let status = viewer.status();
            if let Some(f) = status.frame.as_ref().filter(|f| f.time == at) {
                if let Some(px) = &f.pixels {
                    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
                    let (cols, rows) = (16usize, 9usize);
                    let mut sum = vec![[0f64; 3]; cols * rows];
                    let mut count = vec![0u64; cols * rows];
                    let w = width as usize;
                    for (i, p) in px.iter().enumerate() {
                        for c in p {
                            hash = (hash ^ c.to_bits()).wrapping_mul(0x0000_0100_0000_01b3);
                        }
                        let cell = (i / w * rows / height as usize) * cols + (i % w) * cols / w;
                        for k in 0..3 {
                            sum[cell][k] += p[k];
                        }
                        count[cell] += 1;
                    }
                    let grid: Vec<f64> = sum
                        .iter()
                        .zip(&count)
                        .flat_map(|(s, n)| s.map(|v| (v / (*n).max(1) as f64 * 1e4).round() / 1e4))
                        .collect();
                    break json!({"time":at.get(),"delivered":true,"hash":format!("{hash:016x}"),"pixels":px.len(),"grid16x9_rgb":grid,"ms":started.elapsed().as_secs_f64()*1000.});
                }
                if !asked
                    && f.completion_host_seconds.is_some()
                    && !f.stand_ins
                    && status.effective_quality == requested
                {
                    viewer.capture_next()?;
                    asked = true;
                }
            }
            if let Some(why) = status.capture_failed {
                break json!({"time":at.get(),"delivered":false,"why":why});
            }
            if started.elapsed() > Duration::from_secs(20) {
                break json!({"time":at.get(),"delivered":false,"why":"timeout","asked":asked});
            }
            std::thread::sleep(Duration::from_micros(250));
        };
        exact_frames.push(item);
    }
    // Gate: nothing asked for four seconds (past the engine's two-second idle
    // release): the footprint it settles to.
    diag_mark("idle", &service);
    println!("PHASE idle");
    let idling = Instant::now();
    while idling.elapsed() < Duration::from_secs(4) {
        pm_text::host::serve(&mut font_service, fonts.take_asks());
        viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
        std::thread::sleep(Duration::from_millis(16));
    }
    diag_mark("end", &service);
    println!("PHASE end");
    let summary = |v: &[f64]| json!({"n":v.len(),"p50":percentile(v,0.5),"p95":percentile(v,0.95),"p99":percentile(v,0.99),"max":v.last()});
    let result = json!({
        "mode":"offscreen","clock":"synthetic host ticks; NOT a display link","hz":hz,"seconds":seconds,
        "output":[width,height],"open_s":OPEN_US.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6,"first_frame_s":first_frame_s,
        "samples":costs.len(),"late_ticks_over_interval":late_ticks,"missed_tick_intervals":missed_ticks,
        "worker_to_completion_ms":{"p50":percentile(&costs,0.5),"p95":percentile(&costs,0.95),"p99":percentile(&costs,0.99),"max":costs.last()},
        "headroom_budget_ms":500./hz,"gpu_failures":failures,
        "skipped_targets":status.skipped_targets - skipped_before,"held_frames":status.held_frames,
        "stand_in_frames":stand_ins,"lowest_effective_quality":lowest_quality,
        "layer_failures":status.layer_failures.len(),
        "seek_ms":{"n":seeks.len(),"p50":percentile(&seeks,0.5),"p95":percentile(&seeks,0.95),"p99":percentile(&seeks,0.99),"max":seeks.last(),"stuck":stuck,"stand_ins":seek_stand_ins},
        "scrub_ms":{"forward":summary(&forward),"back":summary(&back),"cached":summary(&cached),"settle_exact":settle_ms,"counts":sweep_counts,"budget_120hz":9.333},
        "gate":{"requested_scale":requested,"playback_stand_ins":stand_ins,
            "seek":seek_tally.json(),
            "scrub_forward":gate_sweeps[0].0.json(),"scrub_back":gate_sweeps[1].0.json(),"scrub_cached":gate_sweeps[2].0.json(),
            "stuck":{"seek":stuck.len(),"scrub_forward":gate_sweeps[0].1,"scrub_back":gate_sweeps[1].1,"scrub_cached":gate_sweeps[2].1},
            "settle":settle_report,"exact_capture":exact_frames},
        "skim_ms":{"counts":skim_counts,"speed":skim_speed,"forward":summary(&skim_forward),"back":summary(&skim_back)},"fill":fill,
        "drag":drag,"diag":DIAG.lock().unwrap().clone(),"on_screen":"UNVERIFIED","build_profile":if cfg!(debug_assertions){"dev"}else{"release"}
    });
    std::fs::write(
        arg("report", "/tmp/es2-service-offscreen.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{result}");
    viewer.retire();
    Ok(())
}

/// Renders `[start, end)` into a disk render cache in `dir` through the fill
/// child, as Render Selection does, then times reading its frames back.
#[allow(clippy::too_many_arguments)]
fn fill_range(
    viewer: &pm_engine_service::Viewer,
    service: &EngineService,
    font_service: &mut pm_text::host::FontService,
    fonts: &pm_text::host::FontBroker,
    dir: &str,
    start: Ticks,
    end: Ticks,
    hz: f64,
) -> anyhow::Result<serde_json::Value> {
    let disk = service.render_cache();
    std::fs::create_dir_all(dir)?;
    disk.set_storage(Some((dir.into(), 64 << 30)))?;
    let seconds = |t: Ticks| t.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64;
    let exe = std::env::current_exe()?;
    let wait = Instant::now();
    // The viewer observes its state when it draws (a frame at `end`, then
    // at `start` again); fingerprinting follows.
    viewer.scrub(end)?;
    viewer.scrub(start)?;
    let queued = loop {
        pm_text::host::serve(font_service, fonts.take_asks());
        viewer.display_tick(pm_engine_service::trace::now() + 1. / hz, 1. / hz)?;
        // No window publication: the bench's overrides (none) need no pin.
        match disk.start_fill(
            seconds(start),
            seconds(end),
            &exe,
            Default::default(),
            |_| {},
        ) {
            Ok(n) => break n,
            Err(e)
                if wait.elapsed() < Duration::from_secs(120)
                    && (e.contains("moment") || e.contains("Open a composition")) =>
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => anyhow::bail!(e),
        }
    };
    let filling = Instant::now();
    loop {
        pm_text::host::serve(font_service, fonts.take_asks());
        let bars = disk.bars();
        if !bars.running && (bars.done >= queued || bars.error.is_some()) {
            anyhow::ensure!(bars.error.is_none(), "{:?}", bars.error);
            break;
        }
        anyhow::ensure!(filling.elapsed() < Duration::from_secs(1800), "fill stuck");
        std::thread::sleep(Duration::from_millis(20));
    }
    let fill_s = filling.elapsed().as_secs_f64();
    // Every frame file of this fill: on-disk bytes, and one read (file,
    // checksum, decode) each, as the reader thread does.
    let mut files: Vec<_> = walk_frames(std::path::Path::new(dir));
    files.sort();
    let (mut bytes, mut reads) = (0u64, Vec::new());
    let (mut width, mut height) = (0, 0);
    for file in &files {
        let t = Instant::now();
        let data = std::fs::read(file)?;
        let payload = &data[48..];
        let word = |at: usize| u32::from_le_bytes(payload[at..at + 4].try_into().unwrap());
        (width, height) = (word(4), word(8));
        let _ = blake3::hash(payload);
        let frame = pm_engine_service::frame_codec::decode(payload, width, height)
            .ok_or_else(|| anyhow::anyhow!("a frame does not decode"))?;
        reads.push(t.elapsed().as_secs_f64() * 1000.);
        // Half-float RGBA, as the viewer uploads it: 8 bytes a pixel.
        anyhow::ensure!(frame.len() == width as usize * height as usize * 8);
        bytes += data.len() as u64;
    }
    reads.sort_by(f64::total_cmp);
    let raw = u64::from(width) * u64::from(height) * 8;
    Ok(json!({"queued":queued,"fill_s":fill_s,"frames":files.len(),
        "bytes_per_frame":bytes / (files.len().max(1) as u64),"raw_bytes_per_frame":raw,
        "read_ms":{"p50":percentile(&reads,0.5),"p95":percentile(&reads,0.95),"max":reads.last()}}))
}

fn walk_frames(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_frames(&path));
        } else if path.extension().is_some_and(|e| e == "frame") {
            out.push(path);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// The sweep moves at the reported speed (review: `--skim=1` covered
    /// two media seconds in one wall second, then rested).
    #[test]
    fn a_skim_sweep_moves_at_its_reported_speed() {
        for speed in [0.5, 1., 2., 4.] {
            let events = super::skim_events(speed, super::SKIM_SPAN_SECONDS);
            let wall = events as f64 / 60.;
            assert!(
                (super::SKIM_SPAN_SECONDS / wall - speed).abs() < 0.05,
                "{speed}: {events} events"
            );
        }
    }
}
