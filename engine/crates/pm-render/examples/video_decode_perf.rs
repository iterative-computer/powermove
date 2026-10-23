//! Pair with apps/desktop/scripts/video-decode-perf.mjs on the same synthetic
//! sources and timestamps. The timed endpoint is GPU completion of a rendered
//! video layer (`Renderer::render` followed by `wait_idle`). Renderer/device
//! construction is excluded; cold samples prime the renderer on the real
//! source, then use a temporary alias to time a fresh media source/session.
//!   cargo run --release -p pm-render --example video_decode_perf -- FIXTURES REPORT
#[path = "../tests/support/doc.rs"]
mod doc;

use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer, VideoEnd};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Instant};

#[global_allocator]
static ALLOC: pm_perf::CountingAlloc = pm_perf::CountingAlloc;

/// A second path to the same synthetic source gives the warmed renderer a
/// fresh media source and decoder session. The link is removed on every exit.
struct SourceAlias(std::path::PathBuf);

impl SourceAlias {
    fn new(source: &Path, output: &Path, id: &str, round: usize) -> std::io::Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let extension = source.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
        let link = output.parent().unwrap_or(Path::new("/tmp")).join(format!(
            ".pm-video-perf-{}-{id}-{round}-{stamp}.{extension}",
            std::process::id()
        ));
        std::os::unix::fs::symlink(source, &link)?;
        Ok(Self(link))
    }
}

impl Drop for SourceAlias {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn counters(c: &pm_perf::Counters) -> BTreeMap<&'static str, u64> {
    pm_perf::Counter::ALL
        .iter()
        .map(|&counter| (counter.name(), c.get(counter)))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn measure(
    renderer: &mut Renderer,
    document: &pm_model::Document,
    comp_id: &str,
    request: &RenderRequest,
    resolver: &MapResolver,
    phase: &str,
    round: usize,
    seconds: f64,
) -> Result<Value, Box<dyn std::error::Error>> {
    let ticks = pm_protocol::common::Ticks::new(
        (seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64,
    )?;
    let scope = pm_perf::Scope::begin();
    pm_media::decode::startup::begin();
    let start = Instant::now();
    let frame = renderer.render(document, comp_id, ticks, request, resolver)?;
    if !frame.failures.is_empty() {
        return Err(format!("video render failures: {:?}", frame.failures).into());
    }
    let submit_ms = start.elapsed().as_secs_f64() * 1000.0;
    renderer.wait_idle();
    let total_ms = start.elapsed().as_secs_f64() * 1000.0;
    let local = scope.finish();
    let global = scope.finish_global();
    let stages = pm_media::decode::startup::finish();
    renderer.recycle(frame);
    Ok(json!({"phase":phase,"round":round,"time":seconds,
        "submitMs":submit_ms,"gpuWaitMs":total_ms-submit_ms,"totalMs":total_ms,
        "counts":counters(&local),"allThreads":counters(&global),
        "startupStages":stages}))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let fixtures = args.next().expect("FIXTURES directory");
    let output = args.next().expect("REPORT path outside Git");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("workspace root");
    let prewarm = match args.next().as_deref() {
        None => false,
        Some("--prewarm") => true,
        _ => return Err("optional third argument must be --prewarm".into()),
    };
    if args.next().is_some()
        || !Path::new(&fixtures).is_absolute()
        || !Path::new(&output).is_absolute()
        || Path::new(&output).starts_with(repo)
    {
        return Err(
            "usage: video_decode_perf ABSOLUTE_FIXTURES ABSOLUTE_REPORT [--prewarm]".into(),
        );
    }
    let dir = Path::new(&fixtures);
    let sources: Vec<Value> = serde_json::from_slice(&std::fs::read(dir.join("sources.json"))?)?;
    let capture: Value = serde_json::from_slice(&std::fs::read(dir.join("chromium.json"))?)?;
    pm_perf::count_globally(true);
    let mut report = json!({
        "endpoint":"Renderer::render(video layer) + Renderer::wait_idle; includes source open, FFmpeg/VideoToolbox decode, source conversion, and layer composite",
        "coldDefinition":"Renderer/device and video shader primed on the source, then a fresh path alias opens an independent media source and decoder session; alias removed on drop",
        "preparedImportColdDefinition":"Optional single import-prepared first render per row: Renderer::prepare_media queues source hash/probe/index and decoder startup; finish_media_preparation explicitly waits outside the first-render timer. Preparation wall time includes the wait; preparation-plus-frame is reported separately. Ordinary cold remains the regression gate",
        "allocationCountersInstalled":pm_perf::alloc_counting_installed(),
        "rows": []
    });
    for row in sources {
        let id = row["id"].as_str().ok_or("source id")?;
        let file = row["file"].as_str().ok_or("source file")?;
        let captured_row = capture["rows"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["id"].as_str() == Some(id)))
            .ok_or("missing captured row")?;
        let playback = captured_row.get("playback").cloned();
        let effective_file = playback
            .as_ref()
            .and_then(|value| value["file"].as_str())
            .unwrap_or(file);
        let times: Vec<f64> = row["times"]
            .as_array()
            .ok_or("source times")?
            .iter()
            .map(|t| t.as_f64().ok_or("source time"))
            .collect::<Result<_, _>>()?;
        if times.is_empty() {
            return Err(format!("{id}: empty timestamps").into());
        }
        let (w, h) = (
            row["probe"]["width"].as_u64().ok_or("width")? as u32,
            row["probe"]["height"].as_u64().ok_or("height")? as u32,
        );
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
                json!({"asset":"clip", "fit":"stretch",
                "w":w, "h":h, "trim":0, "speed":1}),
                w as f64 / 2.0,
                h as f64 / 2.0,
                duration,
            )],
        );
        doc::asset(&mut project, "clip", "video", "mp4", w, h, duration);
        let document = doc::load(&project);
        let comp_id = project["compId"].as_str().ok_or("compId")?;
        let mut resolver = MapResolver::default();
        resolver
            .paths
            .insert("clip".into(), dir.join(effective_file));
        let request = RenderRequest {
            export: true,
            video_end: VideoEnd::Final,
            ..RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32)
        };
        let mut samples = vec![];
        let mut error = None;
        let mut prepare_ms = None;
        for round in 0..3 {
            let mut renderer = Renderer::headless()?;
            if let Err(e) = measure(
                &mut renderer,
                &document,
                comp_id,
                &request,
                &resolver,
                "prime",
                round,
                times[(round + 1) % times.len()],
            ) {
                error = Some(format!("cold renderer prime: {e}"));
                break;
            }
            let alias = SourceAlias::new(&dir.join(effective_file), Path::new(&output), id, round)?;
            let mut cold_resolver = MapResolver::default();
            cold_resolver.paths.insert("clip".into(), alias.0.clone());
            match measure(
                &mut renderer,
                &document,
                comp_id,
                &request,
                &cold_resolver,
                "cold",
                round,
                times[round % times.len()],
            ) {
                Ok(s) => samples.push(s),
                Err(e) => {
                    error = Some(e.to_string());
                    break;
                }
            }
        }
        if prewarm && error.is_none() {
            let mut renderer = Renderer::headless()?;
            if let Err(e) = measure(
                &mut renderer,
                &document,
                comp_id,
                &request,
                &resolver,
                "prime",
                0,
                times[1 % times.len()],
            ) {
                error = Some(format!("prewarmed renderer prime: {e}"));
            } else {
                let alias =
                    SourceAlias::new(&dir.join(effective_file), Path::new(&output), id, 99)?;
                let mut prepared_resolver = MapResolver::default();
                prepared_resolver
                    .paths
                    .insert("clip".into(), alias.0.clone());
                let ticks = pm_protocol::common::Ticks::new(
                    (times[0] * pm_time::TICKS_PER_SECOND as f64).round() as i64,
                )?;
                let start = Instant::now();
                let prepared = renderer.prepare_media(
                    &document,
                    comp_id,
                    ticks,
                    &request,
                    &prepared_resolver,
                )?;
                let preparation_failures = renderer.finish_media_preparation();
                prepare_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
                if !prepared.failures.is_empty()
                    || !preparation_failures.is_empty()
                    || prepared.queued_videos != 1
                {
                    error = Some(format!(
                        "import preparation failed: {prepared:?}, worker: {preparation_failures:?}"
                    ));
                }
                if error.is_none() {
                    match measure(
                        &mut renderer,
                        &document,
                        comp_id,
                        &request,
                        &prepared_resolver,
                        "preparedImportCold",
                        0,
                        times[0],
                    ) {
                        Ok(mut s) => {
                            s["prepareMs"] = prepare_ms.unwrap().into();
                            s["prepareAndFrameMs"] =
                                (prepare_ms.unwrap() + s["totalMs"].as_f64().unwrap()).into();
                            samples.push(s);
                        }
                        Err(e) => error = Some(e.to_string()),
                    }
                }
            }
        }
        if error.is_none() {
            let mut renderer = Renderer::headless()?;
            // Prime every seek position, plus decoder, source, shader, and target caches.
            for (i, &time) in times.iter().enumerate() {
                if let Err(e) = measure(
                    &mut renderer,
                    &document,
                    comp_id,
                    &request,
                    &resolver,
                    "prime",
                    i,
                    time,
                ) {
                    error = Some(e.to_string());
                    break;
                }
            }
            if error.is_none() {
                for round in 0..18 {
                    match measure(
                        &mut renderer,
                        &document,
                        comp_id,
                        &request,
                        &resolver,
                        "warm",
                        round,
                        times[(round + 1) % times.len()],
                    ) {
                        Ok(s) => samples.push(s),
                        Err(e) => {
                            error = Some(e.to_string());
                            break;
                        }
                    }
                }
            }
        }
        // Frame identity is computed after timing, so the extra probe does
        // not warm the cold samples. `select` uses the renderer's source-time
        // frame rule for these unretimed synthetic clips.
        let indexed = pm_media::decode::service::MediaSource::open(&dir.join(effective_file))?;
        for sample in &mut samples {
            let seconds = sample["time"].as_f64().ok_or("sample time")?;
            let reference = captured_row["frames"]
                .as_array()
                .and_then(|frames| {
                    frames
                        .iter()
                        .find(|frame| frame["time"].as_f64() == Some(seconds))
                })
                .ok_or("missing captured frame")?;
            let expected = reference["timestamp"].as_i64().ok_or("captured PTS")?;
            sample["expectedPtsUs"] = expected.into();
            let tick = (seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64;
            if let Some(n) = indexed.select(tick) {
                sample["selectedFrame"] = n.into();
                sample["selectedPtsUs"] = ((indexed.pts[n] as f64
                    * indexed.index.timebase.to_f64()
                    * 1_000_000.0)
                    .round() as i64)
                    .into();
            }
            if sample["selectedPtsUs"].as_i64() != Some(expected) {
                error.get_or_insert_with(|| format!("wrong selected PTS at {seconds}s"));
            }
        }
        if samples.len() != 21 + usize::from(prewarm) {
            error.get_or_insert_with(|| "incomplete cold/warm samples".into());
        }
        let uptime = std::process::Command::new("uptime")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
        let mut result = json!({"id":id,"file":file,"effectiveFile":effective_file,
            "samples":samples,"load":uptime,"prepareMs":prepare_ms,
            "browserPlayback":playback});
        if let Some(error) = error {
            result["error"] = error.into();
        }
        println!(
            "{}",
            json!({"id":id,"samples":result["samples"].as_array().unwrap().len(),
            "error":result.get("error")})
        );
        report["rows"].as_array_mut().unwrap().push(result);
        std::fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    }
    if report["rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row.get("error").is_some())
    {
        Err("video decode benchmark has failed rows; see report".into())
    } else {
        Ok(())
    }
}
