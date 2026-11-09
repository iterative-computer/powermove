//! Windowless service scheduling diagnostic. Synthetic ticks, never scanout.
//!
//! `pm-engine-service --bench [--hz=60] [--seconds=60]` plays a fixture;
//! `--project=COPY.pmv [--definitions=FILE] [--width=1920 --height=1080]`
//! plays a real project (a private copy: its embedded media are extracted to
//! a scratch folder) and then seeks it at random, as the viewer does.
//! `--from=SECONDS` starts the run there instead of at the start;
//! `--quality=full|half|quarter` pins the resolution (Auto by default);
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

fn arg(name: &str, default: &str) -> String {
    std::env::args()
        .find_map(|a| a.strip_prefix(&format!("--{name}=")).map(str::to_owned))
        .unwrap_or(default.into())
}

fn percentile(sorted: &[f64], q: f64) -> Option<f64> {
    sorted
        .get(((sorted.len() as f64 * q).ceil() as usize).saturating_sub(1))
        .copied()
}

/// The fixture or the project copy named by `--project`, with its media.
#[cfg(feature = "kernel")]
fn scene() -> anyhow::Result<(
    pm_engine_service::Snapshot,
    MediaCatalog,
    f64,
    pm_time::Rate,
)> {
    let project = arg("project", "");
    if project.is_empty() {
        let value = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json"))?;
        let snapshot = pm_kernel::document::DocumentStore::new_untitled(value).snapshot();
        return Ok((snapshot, MediaCatalog::default(), 2., pm_time::Rate::FPS_30));
    }
    let opened = pm_kernel::open_project(std::path::Path::new(&project), Default::default())?;
    let store = pm_kernel::document::DocumentStore::open(opened);
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
    let root = snapshot
        .document
        .to_value()
        .ok_or_else(|| anyhow::anyhow!("empty project"))?;
    let canonical: serde_json::Value =
        serde_json::from_str(&pm_model::json::to_canonical_string(&root))?;
    let c = &canonical["comps"][&comp];
    let duration = c["dur"].as_f64().filter(|d| *d > 0.).unwrap_or(1.);
    let fps = c["fps"].as_f64().filter(|f| *f > 0.).unwrap_or(30.);
    let rate =
        pm_time::Rate::new((fps * 1000.).round() as i64, 1000).unwrap_or(pm_time::Rate::FPS_30);
    Ok((snapshot, media, duration, rate))
}

#[cfg(not(feature = "kernel"))]
fn scene() -> anyhow::Result<(Snapshot, MediaCatalog, f64, pm_time::Rate)> {
    let value = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json"))?;
    let snapshot = Snapshot {
        doc_id: 1,
        revision: 1,
        document: Arc::new(pm_model::Document::new(
            Some(pm_model::canonical::canonical_project(value)),
            1,
        )),
    };
    Ok((snapshot, MediaCatalog::default(), 2., pm_time::Rate::FPS_30))
}

/// Where the bench extracts a project's embedded media for one run.
fn scratch_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pm-engine-bench-{}", std::process::id()))
}

/// Removes the run's extracted media however the run ends.
struct Scratch;
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(scratch_dir());
    }
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
    let (snapshot, media, duration, rate) = scene()?;
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
    let start = Ticks::from_whole_seconds(from.floor() as i32);
    let end = Ticks::from_whole_seconds((from.floor() + seconds).min(duration).ceil() as i32);
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
    let started = Instant::now();
    let mut next = started;
    let mut last = 0.;
    let mut costs = Vec::new();
    let (mut failures, mut stand_ins, mut lowest_quality) = (0, 0, 1f64);
    while started.elapsed().as_secs_f64() < seconds + 1. {
        pm_text::host::serve(&mut font_service, fonts.take_asks());
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
    let mut seeks = Vec::new();
    let mut stuck = Vec::new();
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
    let mut sweep = |times: &mut dyn Iterator<Item = i64>| -> anyhow::Result<Vec<f64>> {
        let mut out = Vec::new();
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
                    break;
                }
                anyhow::ensure!(
                    input.elapsed() < Duration::from_secs(10),
                    "scrub stuck at {t}"
                );
                std::thread::sleep(Duration::from_micros(100));
            }
        }
        out.sort_by(f64::total_cmp);
        Ok(out)
    };
    let scrub_from = (from * pm_protocol::common::TICKS_PER_SECOND as f64) as i64 / frame * frame;
    let frames = (2. * rate.to_f64()).round() as i64;
    let forward = sweep(&mut (0..frames).map(|i| scrub_from + i * frame))?;
    let back = sweep(&mut (0..frames).rev().map(|i| scrub_from + i * frame))?;
    let cached = sweep(&mut (0..24).map(|i| scrub_from + (i % 4) * frame))?;
    let summary = |v: &[f64]| json!({"n":v.len(),"p50":percentile(v,0.5),"p95":percentile(v,0.95),"max":v.last()});
    let result = json!({
        "mode":"offscreen","clock":"synthetic host ticks; NOT a display link","hz":hz,"seconds":seconds,
        "output":[width,height],"first_frame_s":first_frame_s,
        "samples":costs.len(),
        "worker_to_completion_ms":{"p50":percentile(&costs,0.5),"p95":percentile(&costs,0.95),"max":costs.last()},
        "headroom_budget_ms":500./hz,"gpu_failures":failures,
        "skipped_targets":status.skipped_targets - skipped_before,"held_frames":status.held_frames,
        "stand_in_frames":stand_ins,"lowest_effective_quality":lowest_quality,
        "layer_failures":status.layer_failures.len(),
        "seek_ms":{"n":seeks.len(),"p50":percentile(&seeks,0.5),"p95":percentile(&seeks,0.95),"max":seeks.last(),"stuck":stuck},
        "scrub_ms":{"forward":summary(&forward),"back":summary(&back),"cached":summary(&cached),"budget_120hz":9.333},
        "fill":fill,
        "on_screen":"UNVERIFIED","build_profile":if cfg!(debug_assertions){"dev"}else{"release"}
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
