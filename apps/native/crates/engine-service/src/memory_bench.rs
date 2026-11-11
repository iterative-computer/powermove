//! `--memory-bench --project=COPY.pmv --output=/private/result.json`
//! or `--fixture=generated/shapes`. Compare with `--baseline=previous.json`.
//! JSON contains numbers only: it is safe to compare without recording content.
use crate::bench::{Scratch, arg, scene};
use pm_engine_service::{FrameSpec, Headless, Moment, Output, Scene, memory};
use pm_protocol::common::Ticks;
use serde::{Deserialize, Serialize};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
struct Sample {
    stage: String,
    footprint: Option<u64>,
    caches: memory::CacheUsage,
    // This renderer-only harness does not start audio or V8. Null means
    // unmeasured, rather than claiming the application uses zero bytes.
    audio_bytes: Option<u64>,
    v8_heap_bytes: Option<u64>,
    elapsed_ms: u64,
}

fn sample(stage: &str, h: &Headless, start: Instant) -> Sample {
    Sample {
        stage: stage.into(),
        footprint: memory::phys_footprint(),
        caches: h.memory_usage(),
        audio_bytes: None,
        v8_heap_bytes: None,
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

pub fn run() -> anyhow::Result<()> {
    let _scratch = Scratch;
    let start = Instant::now();
    let (snapshot, catalog, duration, rate, _store) = scene()?;
    let mut paths = pm_export::render::MediaPaths::default();
    for (id, path) in catalog.originals {
        paths.insert(id, path);
    }
    let scene = Scene {
        comp: snapshot
            .open_comp()
            .ok_or_else(|| anyhow::anyhow!("no composition"))?,
        doc: snapshot.document,
        media: Arc::new(paths),
        content: 1,
    };
    let output = Output {
        width: arg("width", "1920").parse()?,
        height: arg("height", "1080").parse()?,
        display: 0,
        headroom: 1.,
        crop: None,
    };
    let seconds: u32 = arg("seconds", "20").parse()?;
    anyhow::ensure!((1..=300).contains(&seconds), "seconds must be 1..300");
    let mut h = Headless::new()?;
    let mut samples = vec![sample("open", &h, start)];
    let spec = |n: i64, moment| FrameSpec {
        time: Ticks::new(n).unwrap(),
        output,
        moment,
        scale: 1.,
        decode_deadline: None,
        exact: false,
    };
    h.preview(&scene, spec(0, Moment::Paused))?;
    samples.push(sample("frame0", &h, start));
    let playback = Instant::now();
    let frames = u64::from(seconds) * 30;
    for n in 0..frames {
        let t = rate.frame_start(n as i64).unwrap_or(0)
            % (duration * pm_time::TICKS_PER_SECOND as f64).max(1.) as i64;
        h.preview(&scene, spec(t, Moment::Playing))?;
        if n % 30 == 29 {
            samples.push(sample(&format!("play{}", n / 30 + 1), &h, start));
        }
        // Pace to real playback; render cost remains visible when overloaded.
        let due = Duration::from_secs_f64((n + 1) as f64 / 30.);
        if let Some(wait) = due.checked_sub(playback.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    let mut rng = 0x7ad4_813bu64;
    for _ in 0..50 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let t = ((rng >> 32) as f64 / u32::MAX as f64 * duration * pm_time::TICKS_PER_SECOND as f64)
            as i64;
        h.preview(&scene, spec(t, Moment::Paused))?;
    }
    samples.push(sample("seeks50", &h, start));
    if arg("pressure", "false") == "true" {
        let before =
            memory::phys_footprint().ok_or_else(|| anyhow::anyhow!("footprint unavailable"))?;
        h.memory_pressure(memory::Pressure::Critical);
        std::thread::sleep(Duration::from_secs(2));
        let after =
            memory::phys_footprint().ok_or_else(|| anyhow::anyhow!("footprint unavailable"))?;
        samples.push(sample("pressure2", &h, start));
        anyhow::ensure!(
            after as f64 <= before as f64 * 0.70,
            "pressure footprint did not fall 30% within 2s ({before} -> {after})"
        );
        h.memory_pressure(memory::Pressure::Normal);
    }
    std::thread::sleep(Duration::from_secs(2));
    h.release_idle_memory();
    std::thread::sleep(Duration::from_secs(3));
    samples.push(sample("idle5", &h, start));
    let json = serde_json::to_string_pretty(&samples)?;
    let dest = arg("output", "");
    if !dest.is_empty() {
        std::fs::write(dest, &json)?;
    }
    println!("{json}");
    let baseline = arg("baseline", "");
    if !baseline.is_empty() {
        let old: Vec<Sample> = serde_json::from_slice(&std::fs::read(baseline)?)?;
        for s in &samples {
            if let Some(before) = old.iter().find(|b| b.stage == s.stage) {
                let (Some(a), Some(b)) = (s.footprint, before.footprint) else {
                    anyhow::bail!("OS footprint unavailable");
                };
                anyhow::ensure!(
                    a as f64 <= b as f64 * 1.10,
                    "{}: footprint regressed by more than 10% ({b} -> {a})",
                    s.stage
                );
            }
        }
        anyhow::ensure!(old.len() == samples.len(), "baseline stages differ");
    }
    Ok(())
}
