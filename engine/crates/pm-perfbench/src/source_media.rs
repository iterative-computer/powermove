//! Source-media hot-path benches using the same public H.264 fixture as
//! `scrub_24`. These run independently of the baseline ratchet so a paired
//! app run can use the same frames and report its own browser/IPC overhead.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

use pm_perf::{Counter, Scope};
use pm_protocol::common::{Size, TICKS_PER_SECOND, Ticks};
use pm_protocol::snapshot::{SnapshotPixels, SourceFit};
use pm_render::Renderer;
use serde_json::json;

use crate::BenchReport;
use crate::fixtures;

const FRAMES: usize = 24;
const MONITOR: Size = Size::new(320, 180);
const STRIP: Size = Size::new(160, 90);

fn time(k: usize, scattered: bool) -> Ticks {
    let n = if scattered { (k * 7) % FRAMES } else { k };
    let seconds = n as f64 * 1.9 / FRAMES as f64 + 1.0 / 60.0;
    Ticks::new((seconds * TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

fn media_path() -> PathBuf {
    std::env::var_os("PM_SOURCE_BENCH_MEDIA")
        .map(PathBuf::from)
        .unwrap_or_else(fixtures::video_fixture)
}

fn times(scattered: bool) -> Vec<i64> {
    (0..FRAMES).map(|k| time(k, scattered).get()).collect()
}

fn report(
    name: &str,
    unit: &'static str,
    counters: pm_perf::Counters,
    global: pm_perf::Counters,
    wall_ms: Vec<f64>,
    notes: serde_json::Value,
) -> BenchReport {
    BenchReport {
        name: name.into(),
        units: FRAMES as u32,
        unit,
        counters,
        global,
        wall_ms,
        notes,
    }
}

/// Interactive source-monitor scrub: 24 scattered seeks, decoder/cache,
/// source pass and readback at the public fixture's native size.
pub fn source_scrub_24() -> BenchReport {
    pm_perf::count_globally(true);
    let path = media_path();
    assert!(path.is_file(), "public H.264 fixture is present");
    let mut renderer = Renderer::headless().expect("source renderer");
    renderer
        .source_rgba8(&path, Ticks::ZERO, MONITOR, SourceFit::Contain, None)
        .expect("warm source frame");
    let scope = Scope::begin();
    let mut wall_ms = Vec::with_capacity(FRAMES);
    let mut fingerprint = 0u64;
    for k in 0..FRAMES {
        let began = Instant::now();
        let frame = renderer
            .source_rgba8(&path, time(k, true), MONITOR, SourceFit::Contain, None)
            .expect("source scrub frame");
        wall_ms.push(began.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(
            frame.len(),
            MONITOR.width as usize * MONITOR.height as usize * 4
        );
        fingerprint = fingerprint
            .wrapping_mul(31)
            .wrapping_add(u64::from(frame[k * 4]));
    }
    let counters = scope.finish();
    let global = scope.finish_global();
    let result = report(
        "source_scrub_24",
        "seeks",
        counters,
        global,
        wall_ms,
        json!({ "fixture": "h264-aac.mp4", "output": [MONITOR.width, MONITOR.height],
            "fit": "contain", "order": "scattered x7 mod24", "timesTicks": times(true),
            "ticksPerSecond": TICKS_PER_SECOND, "mediaOverride": std::env::var_os("PM_SOURCE_BENCH_MEDIA").is_some(),
            "fingerprint": fingerprint,
            "scope": "native source decode, GPU draw and readback; excludes IPC and canvas" }),
    );
    assert_eq!(result.counters.get(Counter::Readbacks), FRAMES as u64);
    result
}

/// Batch source filmstrip: 24 ordered seeks and straight-alpha RGBA8 frames
/// written to one file, as the job's hot path does after host resolution.
pub fn source_filmstrip_24() -> BenchReport {
    pm_perf::count_globally(true);
    let path = media_path();
    assert!(path.is_file(), "public H.264 fixture is present");
    let dir = fixtures::TempDir::new("source-filmstrip");
    let output = dir.path().join("frames.rgba");
    let mut writer = BufWriter::new(File::create(&output).expect("filmstrip output"));
    let mut renderer = Renderer::headless().expect("filmstrip renderer");
    let scope = Scope::begin();
    let mut wall_ms = Vec::with_capacity(FRAMES);
    let mut fingerprint = 0u64;
    let frame_bytes = STRIP.width as usize * STRIP.height as usize * 4;
    for k in 0..FRAMES {
        let began = Instant::now();
        let frame = renderer
            .source_rgba8(&path, time(k, false), STRIP, SourceFit::Stretch, None)
            .expect("filmstrip source frame");
        let straight =
            pm_node::present::snapshot::encode_rgba8_unorm(&frame, SnapshotPixels::Straight);
        assert_eq!(straight.len(), frame_bytes);
        writer.write_all(&straight).expect("write filmstrip frame");
        wall_ms.push(began.elapsed().as_secs_f64() * 1_000.0);
        fingerprint = fingerprint
            .wrapping_mul(31)
            .wrapping_add(u64::from(straight[k * 4]));
    }
    writer.flush().expect("flush filmstrip");
    let counters = scope.finish();
    let global = scope.finish_global();
    let result = report(
        "source_filmstrip_24",
        "frames",
        counters,
        global,
        wall_ms,
        json!({ "fixture": "h264-aac.mp4", "output": [STRIP.width, STRIP.height],
            "fit": "stretch", "order": "ascending", "timesTicks": times(false),
            "ticksPerSecond": TICKS_PER_SECOND, "mediaOverride": std::env::var_os("PM_SOURCE_BENCH_MEDIA").is_some(),
            "bytes": FRAMES * frame_bytes,
            "fingerprint": fingerprint,
            "scope": "native job frame decode, GPU draw, straight RGBA8 and file write; excludes host capability and status IPC" }),
    );
    drop(writer);
    assert_eq!(
        std::fs::metadata(&output).unwrap().len() as usize,
        FRAMES * frame_bytes
    );
    assert_eq!(result.counters.get(Counter::Readbacks), FRAMES as u64);
    result
}
