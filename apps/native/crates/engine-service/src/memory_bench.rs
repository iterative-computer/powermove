//! `--memory-bench --project=COPY.pmv --output=/private/result.json`
//! or `--fixture=generated/shapes`. Compare with `--baseline=previous.json`.
//! JSON contains numbers only: it is safe to compare without recording content.
use crate::bench::{Scratch, arg, scene};
use pm_engine_service::{FrameSpec, Headless, Moment, Output, Scene, memory};
use pm_protocol::common::Ticks;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
struct Sample {
    stage: String,
    footprint: Option<u64>,
    caches: memory::CacheUsage,
    // Audio uses a simulated callback, never an output device. This harness
    // does not create the app's panel V8 realms; null means unmeasured.
    audio_bytes: Option<u64>,
    v8_heap_bytes: Option<u64>,
    #[serde(default)]
    audio: Option<pm_audio::source::CacheStats>,
    #[serde(default)]
    audio_preparation: Option<serde_json::Value>,
    #[serde(default)]
    cpu_time_ms: u64,
    #[serde(default)]
    allocator: Option<serde_json::Value>,
    elapsed_ms: u64,
}

fn sample(stage: &str, h: &Headless, audio: Option<&AudioProbe>, start: Instant) -> Sample {
    let sample = Sample {
        stage: stage.into(),
        footprint: memory::phys_footprint(),
        caches: h.memory_usage(),
        audio_bytes: audio.map(|a| a.playback.cache_bytes() as u64),
        v8_heap_bytes: None,
        audio: audio.map(|a| a.playback.cache_stats()),
        audio_preparation: audio
            .and_then(|a| a.playback.start_timing())
            .map(|s| serde_json::to_value(s.idle_pool).expect("audio counters")),
        cpu_time_ms: cpu_time_ms(),
        allocator: allocator_usage(),
        elapsed_ms: start.elapsed().as_millis() as u64,
    };
    eprintln!(
        "memory-bench {stage}: footprint={} audio={} wall_ms={} cpu_ms={}",
        sample.footprint.unwrap_or(0),
        sample.audio_bytes.unwrap_or(0),
        sample.elapsed_ms,
        sample.cpu_time_ms
    );
    sample
}

fn allocator_usage() -> Option<serde_json::Value> {
    unsafe extern "C" {
        fn mi_stats_get_json(size: usize, buffer: *mut std::ffi::c_char) -> *mut std::ffi::c_char;
        fn mi_free(pointer: *mut std::ffi::c_void);
    }
    // These are raw allocator statistics, not a resident heap total: builds
    // without detailed statistics leave some counters cumulative or zero.
    // This binary pins mimalloc 3. The returned JSON requires mi_free.
    let pointer = unsafe { mi_stats_get_json(0, std::ptr::null_mut()) };
    if pointer.is_null() {
        return None;
    }
    let parsed: Result<serde_json::Value, _> =
        serde_json::from_slice(unsafe { std::ffi::CStr::from_ptr(pointer) }.to_bytes());
    unsafe { mi_free(pointer.cast()) };
    let parsed = parsed.ok()?;
    let fields = [
        "committed",
        "reserved",
        "page_committed",
        "malloc_normal",
        "malloc_huge",
        "pages_abandoned",
    ];
    Some(serde_json::Value::Object(
        fields
            .into_iter()
            .filter_map(|key| parsed.get(key).map(|value| (key.into(), value.clone())))
            .collect(),
    ))
}

fn cpu_time_ms() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return 0;
    }
    let usage = unsafe { usage.assume_init() };
    let ms = |t: libc::timeval| t.tv_sec as u64 * 1000 + t.tv_usec as u64 / 1000;
    ms(usage.ru_utime) + ms(usage.ru_stime)
}

struct AudioProbe {
    playback: pm_audio::playback::Playback,
    cache: Arc<pm_audio::SourceCache>,
    stop: Arc<AtomicBool>,
    callback: Option<std::thread::JoinHandle<()>>,
}
impl AudioProbe {
    fn new(scene: &Scene) -> anyhow::Result<Self> {
        let rate = pm_audio::mixer::project_sample_rate(&scene.doc);
        let cache = pm_audio::SourceCache::new(24 << 20, 2);
        let (playback, mut side) = pm_audio::playback::Playback::start_headless(
            pm_audio::MixFormat::stereo(rate),
            cache.clone(),
            Default::default(),
            48_000,
            2,
            0,
        )?;
        playback.set_standard_evaluator(|| {
            pm_eval::Evaluator::new().with_expressions(Arc::new(pm_render::expr::PmExprEngine))
        });
        anyhow::ensure!(
            playback
                .set_project(scene.doc.clone(), &scene.comp, scene.media.as_ref())?
                .is_empty(),
            "audio sources could not be opened"
        );
        let stop = Arc::new(AtomicBool::new(false));
        let ended = stop.clone();
        let callback = std::thread::Builder::new()
            .name("pm-memory-bench-callback".into())
            .spawn(move || {
                let mut out = vec![0.; 512 * 2];
                let start = Instant::now();
                let mut frames = 0u64;
                while !ended.load(Ordering::Acquire) {
                    side.callback(&mut out, 2, pm_audio::clock::host_now_us());
                    frames += 512;
                    if let Some(wait) = Duration::from_secs_f64(frames as f64 / 48_000.)
                        .checked_sub(start.elapsed())
                    {
                        std::thread::sleep(wait);
                    }
                }
            })?;
        Ok(Self {
            playback,
            cache,
            stop,
            callback: Some(callback),
        })
    }
}
impl Drop for AudioProbe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(callback) = self.callback.take() {
            let _ = callback.join();
        }
        // Playback drops immediately after this; no new block requests follow.
        self.cache.shutdown();
    }
}

pub fn run() -> anyhow::Result<()> {
    let _scratch = Scratch;
    let start = Instant::now();
    let (snapshot, catalog, duration, _rate, _store) = scene()?;
    eprintln!(
        "memory-bench duration_seconds={duration} media_count={}",
        catalog.originals.len()
    );
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
    let mut font_service = objc2::rc::autoreleasepool(|_| pm_text::host::FontService::new())
        .map_err(anyhow::Error::msg)?;
    let fonts = pm_text::host::FontBroker::new();
    fonts.set_wake(Arc::new(|| {}));
    fonts.set_generation(font_service.generation());
    let mut h = Headless::with_fonts(pm_render::text::host::RenderFonts::new(fonts.clone()))?;
    let mut pump = || {
        objc2::rc::autoreleasepool(|_| {
            pm_text::host::serve(&mut font_service, fonts.take_asks());
        })
    };
    let samples = h.run_with_pump(
        move |h| -> anyhow::Result<Vec<Sample>> {
            let audio = (arg("audio", "true") == "true")
                .then(|| AudioProbe::new(&scene))
                .transpose()?;
            let mut samples = vec![sample("open", h, audio.as_ref(), start)];
            let spec = |n: i64, moment| FrameSpec {
                time: Ticks::new(n).unwrap(),
                output,
                moment,
                scale: 1.,
                decode_deadline: None,
                exact: false,
            };
            h.preview(&scene, spec(0, Moment::Paused))?;
            samples.push(sample("frame0", h, audio.as_ref(), start));
            if let Some(audio) = &audio {
                audio
                    .playback
                    .command(pm_protocol::clock::TransportCommand::Play {
                        rate: pm_protocol::common::PlaybackRate::NORMAL,
                    })?;
            }
            let playback = Instant::now();
            let mut last_frame = 0;
            let mut sampled_second = 0;
            while playback.elapsed() < Duration::from_secs(u64::from(seconds)) {
                // Drop late preview frames, like the viewer. Rendering a fixed 600
                // frames on a slow debug build let audio run minutes ahead of video.
                let n = (playback.elapsed().as_secs_f64() * 30.).floor() as i64;
                let t = n * pm_time::TICKS_PER_SECOND / 30
                    % (duration * pm_time::TICKS_PER_SECOND as f64).max(1.) as i64;
                last_frame = t;
                h.preview(&scene, spec(t, Moment::Playing))?;
                let second = playback.elapsed().as_secs().min(u64::from(seconds));
                if second > sampled_second {
                    samples.push(sample(&format!("play{second}"), h, audio.as_ref(), start));
                    sampled_second = second;
                }
                // Pace to real playback; render cost remains visible when overloaded.
                let due = Duration::from_secs_f64((n + 1) as f64 / 30.);
                if let Some(wait) = due.checked_sub(playback.elapsed()) {
                    std::thread::sleep(wait);
                }
            }
            if let Some(audio) = &audio {
                audio
                    .playback
                    .command(pm_protocol::clock::TransportCommand::Pause)?;
            }
            h.preview(&scene, spec(last_frame, Moment::Paused))?;
            samples.push(sample("stop0", h, audio.as_ref(), start));
            std::thread::sleep(Duration::from_secs(2));
            h.release_idle_memory();
            if let Some(audio) = &audio {
                audio.playback.memory_pressure();
            }
            samples.push(sample("stop2", h, audio.as_ref(), start));
            std::thread::sleep(Duration::from_secs(3));
            samples.push(sample("idle5", h, audio.as_ref(), start));
            let mut rng = 0x7ad4_813bu64;
            for _ in 0..50 {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let t = ((rng >> 32) as f64 / u32::MAX as f64
                    * duration
                    * pm_time::TICKS_PER_SECOND as f64) as i64;
                h.preview(&scene, spec(t, Moment::Paused))?;
            }
            samples.push(sample("seeks50", h, audio.as_ref(), start));
            if arg("pressure", "false") == "true" {
                let before = memory::phys_footprint()
                    .ok_or_else(|| anyhow::anyhow!("footprint unavailable"))?;
                h.memory_pressure(memory::Pressure::Critical);
                std::thread::sleep(Duration::from_secs(2));
                let after = memory::phys_footprint()
                    .ok_or_else(|| anyhow::anyhow!("footprint unavailable"))?;
                samples.push(sample("pressure2", h, audio.as_ref(), start));
                anyhow::ensure!(
                    after as f64 <= before as f64 * 0.70,
                    "pressure footprint did not fall 30% within 2s ({before} -> {after})"
                );
                h.memory_pressure(memory::Pressure::Normal);
            }
            std::thread::sleep(Duration::from_secs(2));
            h.release_idle_memory();
            std::thread::sleep(Duration::from_secs(3));
            samples.push(sample("seek_idle5", h, audio.as_ref(), start));
            drop(audio);
            drop(scene);
            drop(_store);
            h.release_project_memory();
            samples.push(sample("close0", h, None, start));
            std::thread::sleep(Duration::from_secs(2));
            memory::pressure_relief();
            samples.push(sample("close2", h, None, start));
            Ok(samples)
        },
        &mut pump,
    )?;
    let json = serde_json::to_string_pretty(&samples)?;
    let dest = arg("output", "");
    if !dest.is_empty() {
        std::fs::write(dest, &json)?;
    }
    println!("{json}");
    if arg("allocator-stats", "false") == "true" {
        unsafe extern "C" {
            fn mi_stats_print(out: *mut std::ffi::c_void);
        }
        unsafe {
            mi_stats_print(std::ptr::null_mut());
        }
    }
    let hold: u64 = arg("hold", "0").parse()?;
    anyhow::ensure!(hold <= 60, "hold must be 0..60 seconds");
    std::thread::sleep(Duration::from_secs(hold));
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
        // Slow debug renders can skip a playback second. Compare the
        // seconds both runs sampled, while requiring every lifecycle stage.
        anyhow::ensure!(
            old.iter()
                .filter(|s| !s.stage.starts_with("play"))
                .all(|s| samples.iter().any(|now| now.stage == s.stage)),
            "baseline stages differ"
        );
    }
    Ok(())
}
