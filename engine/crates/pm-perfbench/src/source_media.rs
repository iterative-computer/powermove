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

// Match the source job's owned RGBA output path: opaque premultiplied bytes
// are already straight, while translucent frames use the snapshot encoder.
fn straight_source_pixels(pixels: Vec<u8>) -> Vec<u8> {
    if pixels.chunks_exact(4).all(|pixel| pixel[3] == 255) {
        pixels
    } else {
        pm_node::present::snapshot::encode_rgba8_unorm(&pixels, SnapshotPixels::Straight)
    }
}

fn report(
    name: &str,
    unit: &'static str,
    counters: pm_perf::Counters,
    global: pm_perf::Counters,
    wall_ms: Vec<f64>,
    mut notes: serde_json::Value,
) -> BenchReport {
    notes
        .as_object_mut()
        .expect("source bench notes are an object")
        .insert("wallSamplesMs".into(), json!(wall_ms));
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
    let decode = renderer.source_scrub_stats();
    assert_eq!(decode.stand_ins, 0, "source seeks must use exact frames");
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
            "exactDecode": decode,
            "scope": "native source decode, GPU draw and readback; excludes IPC and canvas" }),
    );
    assert_eq!(result.counters.get(Counter::Readbacks), FRAMES as u64);
    result
}

/// Source monitor's coded-video path: the same scattered seeks as
/// `source_scrub_24`, without its RGBA conversion or GPU readback. The CSS
/// monitor is 320×180; decoded plane dimensions come from the codec.
pub fn source_compositor_scrub_24() -> BenchReport {
    pm_perf::count_globally(true);
    let path = media_path();
    assert!(path.is_file(), "public H.264 fixture is present");
    let mut renderer = Renderer::headless().expect("source compositor renderer");
    renderer
        .source_video_planes(&path, Ticks::ZERO, false)
        .expect("warm coded source frame")
        .expect("the fixture's reading is presented as coded planes");
    let scope = Scope::begin();
    let mut wall_ms = Vec::with_capacity(FRAMES);
    let mut fingerprint = 0u64;
    let mut coded = None;
    let mut color = None;
    for k in 0..FRAMES {
        let began = Instant::now();
        let (size, bytes, tags) = renderer
            .source_video_planes(&path, time(k, true), false)
            .expect("source compositor scrub frame")
            .expect("the fixture's reading is presented as coded planes");
        wall_ms.push(began.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(
            bytes.len() as u64,
            tags.byte_len(size.width, size.height)
                .expect("supported coded video format"),
            "packed source-video byte length"
        );
        assert!(size.width > 0 && size.height > 0);
        if let Some(previous) = coded {
            assert_eq!(previous, size, "coded fixture dimensions changed");
        } else {
            coded = Some(size);
        }
        if let Some(previous) = &color {
            assert_eq!(
                previous, &tags,
                "codec colour changed across fixture frames"
            );
        } else {
            color = Some(tags);
        }
        for byte in bytes {
            fingerprint = fingerprint.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
    }
    let counters = scope.finish();
    let global = scope.finish_global();
    let decode = renderer.source_scrub_stats();
    assert_eq!(decode.stand_ins, 0, "source seeks must use exact frames");
    let coded = coded.expect("the benchmark decoded frames");
    let result = report(
        "source_compositor_scrub_24",
        "seeks",
        counters,
        global,
        wall_ms,
        json!({ "fixture": "h264-aac.mp4", "presentationCss": [MONITOR.width, MONITOR.height],
            "codedSize": [coded.width, coded.height], "fit": "contain (CSS)",
            "order": "scattered x7 mod24", "timesTicks": times(true),
            "ticksPerSecond": TICKS_PER_SECOND, "mediaOverride": std::env::var_os("PM_SOURCE_BENCH_MEDIA").is_some(),
            "fingerprint": fingerprint, "exactDecode": decode,
            "sourceCodecColor": color.expect("the benchmark decoded colour tags"),
            "scope": "native exact decode and packed coded YUV planes; excludes IOSurface import, Electron IPC and video compositing" }),
    );
    assert_eq!(result.counters.get(Counter::Readbacks), 0);
    assert_eq!(result.counters.get(Counter::DrawCalls), 0);
    result
}

/// Batch source filmstrip: 24 ordered seeks and straight-alpha RGBA8 frames
/// written to one file, as the job's hot path does after host resolution.
pub fn source_filmstrip_24() -> BenchReport {
    filmstrip_24(false, false)
}

/// The same batch after a real source frame has prepared the decoder, shader
/// and bounded readback resources, as an idle source monitor can do.
pub fn source_filmstrip_prepared_24() -> BenchReport {
    filmstrip_24(true, false)
}

/// The production job's bounded GPU chunks, with one map per admission.
pub fn source_filmstrip_batched_24() -> BenchReport {
    filmstrip_24(false, true)
}

/// Bounded chunks after a genuine source-frame prepare.
pub fn source_filmstrip_batched_prepared_24() -> BenchReport {
    filmstrip_24(true, true)
}

fn filmstrip_24(prepared: bool, batched: bool) -> BenchReport {
    pm_perf::count_globally(true);
    let path = media_path();
    assert!(path.is_file(), "public H.264 fixture is present");
    let dir = fixtures::TempDir::new("source-filmstrip");
    let output = dir.path().join("frames.rgba");
    let mut writer = BufWriter::new(File::create(&output).expect("filmstrip output"));
    let mut renderer = Renderer::headless().expect("filmstrip renderer");
    if prepared {
        renderer
            .source_rgba8(&path, Ticks::ZERO, STRIP, SourceFit::Stretch, None)
            .expect("prepare real source frame");
        renderer.trim(64 << 20);
    }
    let scope = Scope::begin();
    let mut wall_ms = Vec::with_capacity(FRAMES);
    let mut chunk_wall_ms = Vec::new();
    let mut fingerprint = 0u64;
    let frame_bytes = STRIP.width as usize * STRIP.height as usize * 4;
    if batched {
        let chunk = Renderer::source_batch_limit(STRIP);
        for start in (0..FRAMES).step_by(chunk) {
            let end = (start + chunk).min(FRAMES);
            let ticks: Vec<_> = (start..end).map(|k| time(k, false)).collect();
            let began = Instant::now();
            let frames = renderer
                .source_rgba8_batch(&path, &ticks, STRIP, SourceFit::Stretch, None)
                .expect("bounded source batch");
            for (offset, frame) in frames.into_iter().enumerate() {
                let straight = straight_source_pixels(frame);
                assert_eq!(straight.len(), frame_bytes);
                writer.write_all(&straight).expect("write filmstrip frame");
                fingerprint = fingerprint
                    .wrapping_mul(31)
                    .wrapping_add(u64::from(straight[(start + offset) * 4]));
            }
            let elapsed = began.elapsed().as_secs_f64() * 1_000.0;
            chunk_wall_ms.push(elapsed);
            wall_ms.extend(std::iter::repeat_n(
                elapsed / (end - start) as f64,
                end - start,
            ));
        }
    } else {
        for k in 0..FRAMES {
            let began = Instant::now();
            let frame = renderer
                .source_rgba8_export(&path, time(k, false), STRIP, SourceFit::Stretch, None)
                .expect("filmstrip source frame");
            let straight = straight_source_pixels(frame);
            assert_eq!(straight.len(), frame_bytes);
            writer.write_all(&straight).expect("write filmstrip frame");
            wall_ms.push(began.elapsed().as_secs_f64() * 1_000.0);
            fingerprint = fingerprint
                .wrapping_mul(31)
                .wrapping_add(u64::from(straight[k * 4]));
        }
    }
    writer.flush().expect("flush filmstrip");
    let counters = scope.finish();
    let global = scope.finish_global();
    let result = report(
        match (batched, prepared) {
            (false, false) => "source_filmstrip_24",
            (false, true) => "source_filmstrip_prepared_24",
            (true, false) => "source_filmstrip_batched_24",
            (true, true) => "source_filmstrip_batched_prepared_24",
        },
        "frames",
        counters,
        global,
        wall_ms,
        json!({ "fixture": "h264-aac.mp4", "output": [STRIP.width, STRIP.height],
            "fit": "stretch", "order": "ascending", "timesTicks": times(false),
            "prepared": prepared,
            "batched": batched, "chunkWallMs": chunk_wall_ms,
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
    assert_eq!(
        result.counters.get(Counter::Readbacks),
        if batched {
            FRAMES.div_ceil(Renderer::source_batch_limit(STRIP)) as u64
        } else {
            FRAMES as u64
        }
    );
    result
}
