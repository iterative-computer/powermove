//! Video timing for one clip, the native half of a paired comparison with
//! today's viewer (docs/engine/video-parity.md, "Performance"). Prints one
//! JSON line; keep reports outside Git.
//!
//!   cargo run --release -p pm-render --example video_bench -- CLIP [LAYERS]
//!
//! Output is 1920×1080 at the default depth with LAYERS (default 1) video
//! layers of CLIP stacked, each its own decoder. Phases, on one renderer
//! warmed on another path to the clip:
//! - first: a cold source to its first frame on the GPU, and the first
//!   frame after the background preparation the app runs at project open;
//! - scrub: 30 seeks to exact frames (P50/P95);
//! - playback: 5 s paced at the clip rate (`Sampling::Playback`); a frame
//!   is dropped when it completes after its presentation time;
//! - export: the same 5 s rendered as an export, as fast as possible.
//!
//! CPU is this process's user + system time (decode, conversion, render).
#[path = "../tests/support/doc.rs"]
mod doc;

use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_render::sampling::Sampling;
use pm_render::{MapResolver, RenderRequest, Renderer, VideoEnd};
use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[repr(C)]
struct Timeval {
    sec: i64,
    usec: i32,
}

#[repr(C)]
struct Rusage {
    utime: Timeval,
    stime: Timeval,
    rest: [i64; 14],
}

unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut Rusage) -> i32;
}

fn cpu_seconds() -> f64 {
    let mut r = Rusage {
        utime: Timeval { sec: 0, usec: 0 },
        stime: Timeval { sec: 0, usec: 0 },
        rest: [0; 14],
    };
    // SAFETY: RUSAGE_SELF (0) fills the struct, laid out as Darwin's.
    unsafe { getrusage(0, &mut r) };
    let t = |v: &Timeval| v.sec as f64 + f64::from(v.usec) * 1e-6;
    t(&r.utime) + t(&r.stime)
}

struct Bench {
    document: pm_model::Document,
    assets: MapResolver,
}

impl Bench {
    fn render(&self, renderer: &mut Renderer, seconds: f64, export: bool, sampling: Sampling) {
        let request = RenderRequest {
            export,
            sampling,
            video_end: VideoEnd::Final,
            ..RenderRequest::at_scale(Size::new(1920, 1080), 1.0, Precision::Float16)
        };
        let tick = Ticks::new((seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap();
        let frame = renderer
            .render(&self.document, "Ptest-root", tick, &request, &self.assets)
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        renderer.wait_idle();
        renderer.recycle(frame);
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn percentile(values: &mut [f64], p: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() as f64 - 1.0) * p).round() as usize]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let clip = PathBuf::from(args.next().expect("CLIP"));
    let layers: usize = args.next().map_or(1, |n| n.parse().expect("LAYERS"));
    let src = pm_media::decode::service::MediaSource::open(&clip).unwrap();
    let fps = src.rate().to_f64();
    let duration = src.pts.len() as f64 / fps;
    drop(src);
    let dur = duration.max(1.0);
    let mut project = doc::project(
        1920,
        1080,
        dur,
        doc::no_fill(),
        (0..layers)
            .map(|i| {
                doc::layer(
                    &format!("v{i}"),
                    "video",
                    json!({"asset":"clip","fit":"stretch","w":1920,"h":1080,"trim":0,"speed":1}),
                    960.0,
                    540.0,
                    dur,
                )
            })
            .collect(),
    );
    let ext = clip.extension().unwrap().to_str().unwrap();
    doc::asset(&mut project, "clip", "video", ext, 1920, 1080, dur);
    let document = doc::load(&project);
    let bench_of = |path: &std::path::Path| {
        let mut assets = MapResolver::default();
        assets.paths.insert("clip".into(), path.to_path_buf());
        Bench {
            document: document.clone(),
            assets,
        }
    };
    let bench = bench_of(&clip);
    // A link is another path to the clip: warming the renderer on it opens
    // its own source and decoder, so the timed first frame stays cold.
    struct Alias(PathBuf);
    impl Drop for Alias {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let alias = |name: &str| {
        let alias = Alias(std::env::temp_dir().join(format!(
            "pm-video-bench-{}-{name}.{ext}",
            std::process::id()
        )));
        std::os::unix::fs::symlink(std::fs::canonicalize(&clip).unwrap(), &alias.0).unwrap();
        alias
    };
    let (warm, prepared) = (alias("warm"), alias("prepared"));
    let span = duration.min(5.0);
    let frames = (span * fps).floor() as usize;

    // One renderer, as the app keeps: device, pipelines and the conversion
    // are warmed on the alias.
    let mut renderer = Renderer::headless().unwrap();
    bench_of(&warm.0).render(&mut renderer, 0.0, false, Sampling::Final);
    // The app prepares a project's videos when it opens (background probe,
    // index, decoder and selected frame); the first frame then draws.
    let prepared = bench_of(&prepared.0);
    let request = RenderRequest {
        video_end: VideoEnd::Final,
        ..RenderRequest::at_scale(Size::new(1920, 1080), 1.0, Precision::Float16)
    };
    let start = Instant::now();
    renderer
        .prepare_media(
            &prepared.document,
            "Ptest-root",
            Ticks::ZERO,
            &request,
            &prepared.assets,
        )
        .unwrap();
    assert!(renderer.finish_media_preparation().is_empty());
    let preparation = ms(start.elapsed());
    let start = Instant::now();
    prepared.render(&mut renderer, 0.0, false, Sampling::Final);
    let prepared_first = ms(start.elapsed());
    let start = Instant::now();
    bench.render(&mut renderer, 0.0, false, Sampling::Final);
    let first = ms(start.elapsed());

    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut scrub: Vec<f64> = (0..30)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let t = (seed % 10_000) as f64 / 10_000.0 * (duration - 1.0 / fps).max(0.0);
            let start = Instant::now();
            bench.render(&mut renderer, t, false, Sampling::Final);
            ms(start.elapsed())
        })
        .collect();

    bench.render(&mut renderer, 0.0, false, Sampling::Playback);
    let (cpu, start) = (cpu_seconds(), Instant::now());
    let mut dropped = 0;
    for i in 1..=frames {
        let due = Duration::from_secs_f64(i as f64 / fps);
        if start.elapsed() > due {
            // A player skips a frame whose time has passed.
            dropped += 1;
            continue;
        }
        bench.render(&mut renderer, i as f64 / fps, false, Sampling::Playback);
        if let Some(rest) = due.checked_sub(start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    let playback_cpu = cpu_seconds() - cpu;
    let playback_wall = start.elapsed().as_secs_f64();

    let (cpu, start) = (cpu_seconds(), Instant::now());
    for i in 0..frames {
        bench.render(&mut renderer, i as f64 / fps, true, Sampling::Final);
    }
    let export = ms(start.elapsed());
    let export_cpu = cpu_seconds() - cpu;

    println!(
        "{}",
        json!({
            "side": "engine", "clip": clip.file_name().unwrap().to_string_lossy(),
            "layers": layers, "fps": fps, "frames": frames,
            "firstMs": first, "preparationMs": preparation, "preparedFirstMs": prepared_first,
            "scrubP50Ms": percentile(&mut scrub, 0.5), "scrubP95Ms": percentile(&mut scrub, 0.95),
            "playbackDropped": dropped, "playbackCpuS": playback_cpu, "playbackWallS": playback_wall,
            "exportMs": export, "exportCpuS": export_cpu,
        })
    );
}
