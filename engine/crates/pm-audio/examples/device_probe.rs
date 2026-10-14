//! Measures the master clock on the real output device, silently: opens
//! the default output through CoreAudio, "plays" an empty composition (the
//! device outputs zeros), and reports the device's format and latency and
//! how well consecutive clock samples agree (contract 6, "what Phase 0 must
//! confirm": cpal's presentation timestamps and latency are precise enough).
//!
//!   cargo run --release -p pm-audio --example device_probe [seconds]

use std::sync::Arc;
use std::time::{Duration, Instant};

use pm_audio::clock::host_now_us;
use pm_audio::playback::{Playback, PlaybackConfig};
use pm_audio::{MixFormat, SourceCache};
use pm_model::Document;
use pm_model::json::parse;
use pm_protocol::clock::{ClockSample, TransportCommand, TransportState};
use pm_protocol::common::{PlaybackRate, TICKS_PER_SECOND};

fn main() {
    let seconds: f64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5.0);
    let cache = SourceCache::new(16 << 20, 1);
    let mut playback = Playback::start(MixFormat::stereo(48_000), cache, PlaybackConfig::default())
        .expect("open the output device");
    let doc = Document::new(
        Some(parse(r#"{"id":"P","compId":"C","w":16,"h":16,"fps":30,"dur":600,"layers":[],"comps":{},"assets":{},"params":{}}"#).unwrap()),
        1,
    );
    let none = |_: &str| None;
    playback.set_project(Arc::new(doc), "C", &none).unwrap();
    let started = host_now_us();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    let mut samples: Vec<ClockSample> = vec![];
    let end = Instant::now() + Duration::from_secs_f64(seconds);
    let mut version = 0;
    while Instant::now() < end {
        let v = playback.clock_version();
        if v != version {
            version = v;
            if let Some(s) = playback.clock() {
                samples.push(s);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    playback.command(TransportCommand::Pause).unwrap();
    let device = playback.device().cloned().unwrap();
    let playing: Vec<&ClockSample> = samples
        .iter()
        .filter(|s| s.state == TransportState::Playing)
        .collect();
    // Each new anchor against the previous line: how far the line moved
    // (device clock against host clock, plus timestamp jitter).
    let mut jumps: Vec<f64> = vec![];
    for w in playing.windows(2) {
        if w[0].epoch != w[1].epoch {
            continue;
        }
        let predicted = w[0].media_time_at(w[1].host_time).get();
        jumps.push((w[1].media_time.get() - predicted) as f64 / TICKS_PER_SECOND as f64 * 1e6);
    }
    let first_play = playing
        .first()
        .map(|s| s.host_time.get() as i64 - started as i64);
    let mut abs: Vec<f64> = jumps.iter().map(|j| j.abs()).collect();
    abs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| {
        abs.get(((abs.len() as f64 - 1.0) * p) as usize)
            .copied()
            .unwrap_or(f64::NAN)
    };
    let span_s = match (playing.first(), playing.last()) {
        (Some(a), Some(b)) => (b.host_time.get() - a.host_time.get()) as f64 / 1e6,
        _ => 0.0,
    };
    let drift_ppm = match (playing.first(), playing.last()) {
        (Some(a), Some(b)) if span_s > 0.0 => {
            let media = (b.media_time.get() - a.media_time.get()) as f64 / TICKS_PER_SECOND as f64;
            (media / span_s - 1.0) * 1e6
        }
        _ => f64::NAN,
    };
    println!("device: {:?}", device);
    println!(
        "latency folded into host_time: {} µs (device + stream latency {} frames; safety offset {} frames not added)",
        device.latency_us(),
        device.latency_frames,
        device.safety_offset_frames
    );
    println!("stats: {:?}", playback.stats());
    if let Some(stream) = playback.stream() {
        println!(
            "callbacks: {}; cpal's playback estimate is callback + {} µs (it adds the I/O buffer of {} frames)",
            stream.callbacks(),
            stream.cpal_playback_delay_us(),
            device.buffer_frames
        );
    }
    println!(
        "clock samples while playing: {} over {span_s:.2} s ({:.1} per second)",
        playing.len(),
        playing.len() as f64 / span_s.max(1e-9)
    );
    println!(
        "first playing sample presents {:?} µs after Play was sent",
        first_play
    );
    println!(
        "re-anchor jumps |Δ| µs: p50 {:.2}, p95 {:.2}, max {:.2} (n = {})",
        pct(0.5),
        pct(0.95),
        pct(1.0),
        abs.len()
    );
    println!("device clock against host clock: {drift_ppm:+.1} ppm");
    playback.shutdown();
}
