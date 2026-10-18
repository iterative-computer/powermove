//! Playback against contract 6 and section 15 with a simulated device: the
//! clock line predicts what the device presents (also when the device's
//! sample clock drifts against the host clock), what plays equals the
//! offline render from the same sample (deterministic pre-roll), and pause,
//! seek, loop, and scrub behave.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use common::*;
use pm_audio::clock::host_now_us;
use pm_audio::playback::{DeviceSide, Playback, PlaybackConfig};
use pm_audio::{MixFormat, SourceCache};
use pm_protocol::clock::{ClockSample, SeekMode, TransportCommand, TransportState};
use pm_protocol::common::{HostMicros, PlaybackRate, TICKS_PER_SECOND, TickRange, Ticks};

const SR: u32 = 48_000;
const BUFFER: usize = 512;
const LATENCY_US: u32 = 7_000;

struct CpuContention {
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl CpuContention {
    fn start() -> Arc<Self> {
        // At most two contention threads across all four concurrent tests.
        static LOAD: Mutex<Weak<CpuContention>> = Mutex::new(Weak::new());
        let mut shared = LOAD.lock().unwrap();
        if let Some(load) = shared.upgrade() {
            return load;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let threads = (0..2)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut value = 1u64;
                    while !stop.load(Ordering::Relaxed) {
                        for _ in 0..4096 {
                            value = std::hint::black_box(
                                value.wrapping_mul(6364136223846793005).wrapping_add(1),
                            );
                        }
                        std::thread::yield_now();
                    }
                })
            })
            .collect();
        let load = Arc::new(Self { stop, threads });
        *shared = Arc::downgrade(&load);
        load
    }
}

impl Drop for CpuContention {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            thread.join().unwrap();
        }
    }
}

fn stress_enabled() -> bool {
    std::env::var_os("PM_AUDIO_PLAYBACK_STRESS").is_some()
}

fn ticks(s: f64) -> Ticks {
    Ticks::new((s * TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

/// A device whose sample clock runs `ppm` fast against the host clock.
struct SimDevice {
    _load: Option<Arc<CpuContention>>,
    side: DeviceSide,
    rate: u32,
    channels: usize,
    ppm: f64,
    start_host: u64,
    frames: u64,
    out: Vec<f32>,
    /// Every buffer: (host µs of its first frame at the output, samples).
    log: Vec<(u64, Vec<f32>)>,
}

impl SimDevice {
    fn new(side: DeviceSide, rate: u32, channels: usize, ppm: f64) -> SimDevice {
        SimDevice {
            _load: stress_enabled().then(CpuContention::start),
            side,
            rate,
            channels,
            ppm,
            start_host: host_now_us(),
            frames: 0,
            out: vec![0.0; BUFFER * channels],
            log: vec![],
        }
    }

    fn host_of(&self, frame: u64) -> u64 {
        self.start_host + (frame as f64 * 1e6 / (self.rate as f64 * (1.0 + self.ppm * 1e-6))) as u64
    }

    /// Runs `buffers` callbacks paced at the device's real rate.
    ///
    /// The device's timeline is simulated (host times come from its frame
    /// count), so on a loaded machine, an unoptimized build especially, it
    /// waits up to 40 ms for the render thread to have a buffer queued
    /// instead of reading an underrun no real-time device would see. When
    /// nothing is queued for a reason (paused, ended) it proceeds after the
    /// wait. The clock under test only reads the host times passed in.
    fn run(&mut self, buffers: usize) {
        let need = BUFFER.div_ceil(pm_audio::ring::CHUNK) + 1;
        for _ in 0..buffers {
            let host = self.host_of(self.frames);
            // Pace: the callback for a buffer comes when the previous one is due.
            let now = host_now_us();
            if host > now + 2_000 {
                std::thread::sleep(Duration::from_micros(host - now - 2_000));
            }
            let patience = std::time::Instant::now();
            while self.side.queued_chunks() < need && patience.elapsed() < Duration::from_millis(40)
            {
                std::thread::sleep(Duration::from_micros(500));
            }
            self.side.callback(&mut self.out, self.channels, host);
            self.log.push((host, self.out.clone()));
            self.frames += BUFFER as u64;
        }
    }
}

fn setup(
    file: &std::path::Path,
    dur: f64,
    device_rate: u32,
    ppm: f64,
) -> (Playback, SimDevice, Rig) {
    let doc = project(30.0, dur, &[audio("A", "s", 0.0, dur, "", "")], &[], "");
    setup_doc(doc, &[("s", file)], device_rate, 2, ppm)
}

/// Playback of `doc` (composition `C`) on a simulated device, decoding on
/// two workers as the app does.
fn setup_doc(
    doc: pm_model::Document,
    media: &[(&str, &std::path::Path)],
    device_rate: u32,
    device_channels: usize,
    ppm: f64,
) -> (Playback, SimDevice, Rig) {
    let mut rig = Rig::new(media);
    rig.cache = SourceCache::new(64 << 20, 2);
    if stress_enabled() {
        rig.cache.set_decode_delay(Duration::from_millis(8));
    }
    let cache: Arc<SourceCache> = rig.cache.clone();
    let (playback, side) = Playback::start_headless(
        MixFormat::stereo(SR),
        cache,
        PlaybackConfig::default(),
        device_rate,
        device_channels,
        LATENCY_US,
    )
    .unwrap();
    let media = rig.media.clone();
    let resolver = move |id: &str| media.get(id).cloned();
    let problems = playback.set_project(Arc::new(doc), "C", &resolver).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    (
        playback,
        SimDevice::new(side, device_rate, device_channels, ppm),
        rig,
    )
}

/// A ramp source makes every sample's media position readable from its value.
fn ramp_file(dir: &std::path::Path, seconds: usize) -> std::path::PathBuf {
    let file = dir.join("ramp.wav");
    let n = SR as usize * seconds;
    // Left: position / 2^24 (exact in f32); right: a constant marker.
    let left: Vec<f32> = (0..n).map(|i| i as f32 / 16_777_216.0).collect();
    write_wav(&file, SR, &[left, vec![0.5; n]]);
    file
}

fn position_of(v: f32) -> i64 {
    (v as f64 * 16_777_216.0).round() as i64
}

#[test]
fn the_clock_predicts_what_the_device_presents() {
    let dir = temp_dir("clock");
    let file = ramp_file(&dir, 8);
    for ppm in [0.0, 150.0, -150.0] {
        let (playback, mut dev, _rig) = setup(&file, 8.0, SR, ppm);
        playback
            .command(TransportCommand::Seek {
                time: ticks(1.0),
                mode: SeekMode::Exact,
            })
            .unwrap();
        playback
            .command(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            })
            .unwrap();
        dev.run(260); // about 2.8 s
        let mut worst_us = 0f64;
        let mut checked = 0;
        let mut first: Option<i64> = None;
        let mut clock: Option<ClockSample> = None;
        for (host, buf) in &dev.log {
            // The marker channel says this buffer plays media.
            if buf[1] != 0.5 {
                continue;
            }
            let pos = position_of(buf[0]);
            first.get_or_insert(pos);
            // The latest clock published by then (samples are published from
            // the callback; re-read after each buffer would race, so use the
            // final line for the earlier buffers only when same epoch).
            let latest = playback.clock().unwrap();
            if clock.is_none_or(|c| c.epoch != latest.epoch) {
                clock = Some(latest);
            }
            let c = clock.unwrap();
            let heard = HostMicros::new(host + LATENCY_US as u64).unwrap();
            let predicted = c.media_time_at(heard).get() as f64 / TICKS_PER_SECOND as f64;
            let actual = pos as f64 / SR as f64;
            worst_us = worst_us.max((predicted - actual).abs() * 1e6);
            checked += 1;
        }
        let first = first.expect("media played");
        assert_eq!(
            first, SR as i64,
            "playback starts at the seek position (ppm {ppm})"
        );
        assert!(checked > 200, "{checked} buffers");
        // Contract 6 asks for the A/V sync bound (±10 ms); the line holds to
        // microseconds, drift included, because it re-anchors every 100 ms.
        assert!(
            worst_us < 2_500.0,
            "ppm {ppm}: worst prediction error {worst_us:.1} µs"
        );
        eprintln!(
            "ppm {ppm:+}: {checked} buffers, worst |predicted − presented| {worst_us:.1} µs (final line)"
        );
        assert_eq!(playback.stats().starved_renders, 0);
        drop(playback);
    }
}

/// Every clock sample, applied to the buffers it covers, predicts them to
/// within a microsecond plus the drift since its anchor.
#[test]
fn published_samples_re_anchor_against_drift() {
    let dir = temp_dir("drift");
    let file = ramp_file(&dir, 6);
    let ppm = 300.0;
    let (playback, mut dev, _rig) = setup(&file, 6.0, SR, ppm);
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    // Record samples as they are published, between buffers.
    let mut samples: Vec<ClockSample> = vec![];
    let mut worst_us = 0f64;
    let mut seen = 0;
    for _ in 0..200 {
        dev.run(1);
        let (host, buf) = dev.log.last().unwrap().clone();
        if let Some(s) = playback.clock()
            && samples.last().is_none_or(|l| l.seq != s.seq)
        {
            samples.push(s);
        }
        if buf[1] != 0.5 || samples.is_empty() {
            continue;
        }
        let s = samples.last().unwrap();
        if s.state != TransportState::Playing {
            continue;
        }
        let predicted = s
            .media_time_at(HostMicros::new(host + LATENCY_US as u64).unwrap())
            .get() as f64
            / TICKS_PER_SECOND as f64;
        let actual = position_of(buf[0]) as f64 / SR as f64;
        worst_us = worst_us.max((predicted - actual).abs() * 1e6);
        seen += 1;
    }
    let playing: Vec<&ClockSample> = samples
        .iter()
        .filter(|s| s.state == TransportState::Playing)
        .collect();
    // At least 4 a second (contract 6): 200 buffers ≈ 2.1 s.
    assert!(playing.len() >= 9, "{} samples", playing.len());
    for w in playing.windows(2) {
        assert!(w[1].host_time.get() - w[0].host_time.get() <= 250_000);
        assert_eq!(w[1].epoch, w[0].epoch);
    }
    // 300 ppm over at most ~110 ms between anchors: about 33 µs, plus the
    // integer-µs rounding of host times.
    assert!(worst_us < 60.0, "worst {worst_us:.1} µs");
    eprintln!(
        "300 ppm drift: {} samples over {seen} buffers, worst error {worst_us:.1} µs",
        playing.len()
    );
}

/// What the device plays equals the offline render from the same sample.
#[test]
fn playback_equals_the_offline_render() {
    let dir = temp_dir("equal");
    let f44 = dir.join("t44.wav");
    write_wav(
        &f44,
        44_100,
        &[
            tone(44_100, 6.0, 997.0, 0.5),
            tone(44_100, 6.0, 1503.0, 0.3),
        ],
    );
    let (playback, mut dev, rig) = setup(&f44, 6.0, SR, 0.0);
    playback
        .command(TransportCommand::Seek {
            time: ticks(2.5),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(150);
    let played: Vec<f32> = dev
        .log
        .iter()
        .flat_map(|(_, b)| b.iter().step_by(2).copied())
        .collect();
    let start = played.iter().position(|v| *v != 0.0).expect("audio played");
    let doc = project(30.0, 6.0, &[audio("A", "s", 0.0, 6.0, "", "")], &[], "");
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let n = 40_000;
    let offline = rig.render(
        &mut m,
        (2.5 * SR as f64) as i64,
        (2.5 * SR as f64) as i64 + n as i64,
        4096,
    );
    // The first played sample can be an exact zero crossing; align on the
    // offline render's first non-zero sample instead.
    let lead = offline[0].iter().position(|v| *v != 0.0).unwrap();
    let played = &played[start - lead..start - lead + n];
    assert!(
        played == &offline[0][..],
        "played audio differs from the offline render"
    );
}

#[test]
fn pause_seek_loop_and_scrub() {
    let dir = temp_dir("transport");
    let file = ramp_file(&dir, 6);
    let (playback, mut dev, _rig) = setup(&file, 6.0, SR, 0.0);
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(100);
    // Pause: the clock stops where the listener is.
    playback.command(TransportCommand::Pause).unwrap();
    let stopped = playback.clock().unwrap();
    assert_eq!(stopped.state, TransportState::Stopped);
    let (last_host, last_buf) = dev
        .log
        .iter()
        .rev()
        .find(|(_, b)| b[1] == 0.5)
        .unwrap()
        .clone();
    let heard_now = position_of(last_buf[0]) as f64 / SR as f64
        + (host_now_us() as f64 - (last_host + LATENCY_US as u64) as f64) / 1e6;
    let paused_at = stopped.media_time.get() as f64 / TICKS_PER_SECOND as f64;
    assert!(
        (paused_at - heard_now).abs() < 0.03,
        "paused at {paused_at}, heard {heard_now}"
    );
    dev.run(20);
    assert!(
        dev.log[dev.log.len() - 5..]
            .iter()
            .all(|(_, b)| b.iter().all(|v| *v == 0.0)),
        "silent while paused"
    );

    // Seek while playing: a new epoch, and the next media is the target.
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(40);
    let before = playback.clock().unwrap().epoch;
    let mark = dev.log.len();
    playback
        .command(TransportCommand::Seek {
            time: ticks(4.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
    dev.run(60);
    let after = playback.clock().unwrap();
    assert!(after.epoch > before);
    let jumped = dev.log[mark..]
        .iter()
        .filter(|(_, b)| b[1] == 0.5)
        .map(|(_, b)| position_of(b[0]))
        .find(|&p| p >= 4 * SR as i64 - 1)
        .expect("plays from the seek");
    assert_eq!(
        jumped,
        4 * SR as i64,
        "the first buffer after the seek starts on its sample"
    );

    // Loop [1.0, 1.25): positions wrap exactly.
    playback
        .command(TransportCommand::SetLoop {
            range: Some(TickRange {
                start: ticks(1.0),
                end: ticks(1.25),
            }),
        })
        .unwrap();
    playback
        .command(TransportCommand::Seek {
            time: ticks(1.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
    let mark = dev.log.len();
    dev.run(120);
    let positions: Vec<i64> = dev.log[mark..]
        .iter()
        .filter(|(_, b)| b[1] == 0.5)
        .flat_map(|(_, b)| {
            b.chunks(2)
                .filter(|f| f[1] == 0.5)
                .map(|f| position_of(f[0]))
                .collect::<Vec<_>>()
        })
        .collect();
    let first = positions.iter().position(|&p| p == SR as i64).unwrap();
    let mut wraps = 0;
    for w in positions[first..].windows(2) {
        if w[1] != w[0] + 1 {
            assert_eq!(
                (w[0], w[1]),
                (SR as i64 * 5 / 4 - 1, SR as i64),
                "loop wrap"
            );
            wraps += 1;
        }
    }
    assert!(wraps >= 3, "{wraps} wraps");
    let looping = playback.clock().unwrap();
    assert!(looping.loop_range.is_some());
    playback
        .command(TransportCommand::SetLoop { range: None })
        .unwrap();
    playback.command(TransportCommand::Pause).unwrap();

    // Scrub: a grain at the new position, fading in and out, then silence.
    playback
        .command(TransportCommand::AudioScrub { enabled: true })
        .unwrap();
    let mark = dev.log.len();
    playback
        .command(TransportCommand::Seek {
            time: ticks(3.0),
            mode: SeekMode::Scrub,
        })
        .unwrap();
    dev.run(30);
    let grain: Vec<f32> = dev.log[mark..]
        .iter()
        .flat_map(|(_, b)| b.chunks(2).map(|f| f[1]).collect::<Vec<_>>())
        .collect();
    let on = grain.iter().filter(|v| **v != 0.0).count();
    assert!((2_800..=2_900).contains(&on), "grain of {on} samples");
    let peak = grain.iter().fold(0f32, |m, v| m.max(*v));
    assert_eq!(peak, 0.5);
    let first = grain.iter().position(|v| *v != 0.0).unwrap();
    assert!(grain[first] < 0.01, "the grain fades in");
    let scrubbed = playback.clock().unwrap();
    assert!(matches!(
        scrubbed.state,
        TransportState::Stopped | TransportState::Shuttling
    ));
}

/// A device at 44.1 kHz under a 48 kHz project: rubato converts, the clock
/// counts device frames, and a tone keeps its pitch.
#[test]
fn a_device_at_another_rate() {
    let dir = temp_dir("device-rate");
    let file = dir.join("tone.wav");
    write_wav(
        &file,
        SR,
        &[tone(SR, 4.0, 1000.0, 0.5), tone(SR, 4.0, 1000.0, 0.5)],
    );
    let (playback, mut dev, _rig) = setup(&file, 4.0, 44_100, 0.0);
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(150);
    let played: Vec<f32> = dev
        .log
        .iter()
        .flat_map(|(_, b)| b.iter().step_by(2).copied())
        .collect();
    let start = played.iter().position(|v| v.abs() > 0.1).unwrap() + 4410;
    let window = &played[start..start + 44_100];
    let crossings = window
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    assert!(
        (crossings as i64 - 1000).abs() <= 1,
        "{crossings} cycles in one second"
    );
    let clock = playback.clock().unwrap();
    assert_eq!(clock.sample_rate, 44_100);
    assert_eq!(playback.device_rate(), 44_100);
}

/// A device change (here, its latency) re-anchors on a new epoch and
/// playback continues from what was heard.
#[test]
fn a_device_change_re_anchors() {
    let dir = temp_dir("device-change");
    let file = ramp_file(&dir, 6);
    let (playback, mut dev, _rig) = setup(&file, 6.0, SR, 0.0);
    let state = std::sync::Arc::new(std::sync::Mutex::new((7u32, LATENCY_US)));
    let seen = state.clone();
    playback.set_device_watch(move || Some(*seen.lock().unwrap()));
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(100);
    let before = playback.clock().unwrap();
    *state.lock().unwrap() = (7, 30_000);
    dev.run(160);
    let after = playback.clock().unwrap();
    assert!(after.epoch > before.epoch, "a new epoch");
    assert_eq!(after.output_latency_us, 30_000);
    assert_eq!(after.state, TransportState::Playing);
    // Media continued: no jump back to the start.
    let positions: Vec<i64> = dev
        .log
        .iter()
        .filter(|(_, b)| b[1] == 0.5)
        .map(|(_, b)| position_of(b[0]))
        .collect();
    assert!(
        positions.windows(2).all(|w| w[1] >= w[0] - SR as i64 / 10),
        "playback went back"
    );
    // And the new line predicts the device with the new latency.
    let (host, buf) = dev.log.last().unwrap();
    let predicted = after
        .media_time_at(HostMicros::new(host + 30_000).unwrap())
        .get() as f64
        / TICKS_PER_SECOND as f64;
    let actual = position_of(buf[0]) as f64 / SR as f64;
    assert!(
        (predicted - actual).abs() < 0.001,
        "{predicted} vs {actual}"
    );
}

/// A live edit while playing (a fader move) is heard as a 12 ms glide, not a
/// step, and playback keeps its place: no restart, no new epoch.
#[test]
fn a_live_edit_glides() {
    let dir = temp_dir("retune");
    let file = ramp_file(&dir, 4);
    let (playback, mut dev, rig) = setup(&file, 4.0, SR, 0.0);
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(40);
    let epoch = playback.clock().unwrap().epoch;
    let doc = project(
        30.0,
        4.0,
        &[audio("A", "s", 0.0, 4.0, r#","gain":0.5"#, "")],
        &[],
        "",
    );
    let media = rig.media.clone();
    let resolver = move |id: &str| media.get(id).cloned();
    playback.set_project(Arc::new(doc), "C", &resolver).unwrap();
    dev.run(60);
    assert_eq!(
        playback.clock().unwrap().epoch,
        epoch,
        "an edit does not restart"
    );
    // The marker channel, as heard.
    let marker: Vec<f32> = dev
        .log
        .iter()
        .flat_map(|(_, b)| b.chunks(2).map(|f| f[1]).collect::<Vec<_>>())
        .collect();
    let first = marker
        .iter()
        .position(|v| *v == 0.5)
        .expect("playback started");
    let heard = &marker[first..];
    assert_eq!(*heard.last().unwrap(), 0.25, "the new level is heard");
    let glide = (SR * pm_audio::RETUNE_GLIDE_MS / 1000) as f32;
    let largest = heard
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0f32, f32::max);
    assert!(
        largest <= 0.25 / glide * 1.001,
        "largest step {largest}, a glide step {}",
        0.25 / glide
    );
    let gliding = heard.iter().filter(|v| **v < 0.5 && **v > 0.25).count();
    assert!(
        (gliding as f32 - glide).abs() <= 1.0,
        "{gliding} samples between the levels"
    );
}

/// Speed-changed and time-remapped clips decode ahead like any other: the
/// pre-roll waits for their source blocks, the look-ahead queues them, and
/// what plays equals the offline render from the first sample (no starved
/// render, no dropout). The source is AAC, so decoding takes real time.
#[test]
fn time_mapped_clips_play_as_rendered_offline() {
    time_mapped_playback(false);
}

#[test]
fn time_mapped_clips_play_as_rendered_offline_under_load() {
    time_mapped_playback(true);
}

/// A one-chunk ring underrun explains the reported 79,744/80,000
/// difference even when the decoder never misses. Keep this distinction
/// explicit: the callback reports a ring underrun, not a source starvation.
#[test]
fn a_partial_callback_underrun_is_reported_and_recovers_in_order() {
    use pm_audio::ring::{CHUNK, Consumer, Ring, Tag};
    let ring = Ring::new(400, 1);
    let mut consumer = Consumer::new(ring.clone());
    let reference: Vec<f32> = (1..=80_128).map(|n| n as f32).collect();
    let tag = Tag {
        frames: CHUNK as u32,
        ..Tag::default()
    };
    assert!(ring.push(tag, &reference[..CHUNK]));
    let mut heard = vec![0.0; 80_000];
    consumer.fill(&mut heard[..BUFFER], 1);
    for chunk in reference[CHUNK..].chunks_exact(CHUNK) {
        assert!(ring.push(tag, chunk));
    }
    consumer.fill(&mut heard[BUFFER..], 1);
    assert_eq!(ring.underruns(), 1);
    assert_eq!(&heard[..CHUNK], &reference[..CHUNK]);
    assert!(heard[CHUNK..BUFFER].iter().all(|&s| s == 0.0));
    assert_eq!(&heard[BUFFER..], &reference[CHUNK..80_000 - CHUNK]);
    assert_eq!(
        heard.iter().zip(&reference).filter(|(a, b)| a != b).count(),
        79_744
    );
}

fn time_mapped_playback(loaded: bool) {
    let _load = loaded.then(CpuContention::start);
    let dir = temp_dir(if loaded {
        "time-mapped-loaded"
    } else {
        "time-mapped"
    });
    let file = dir.join("tone.m4a");
    generate(
        &file,
        "aevalsrc=0.125*sin(2*PI*441*t)|0.125*sin(2*PI*660*t):s=44100:d=20",
        &["-c:a", "aac", "-b:a", "160k"],
    );
    let remap = keys(&[(0.0, 2.0), (6.0, 10.0)]);
    let cases: [(&str, String, f64); 3] = [
        (
            "audio at speed 1.5",
            audio("A", "s", 0.0, 12.0, r#","speed":1.5"#, ""),
            3.0,
        ),
        (
            "audio at speed 0.75",
            audio("A", "s", 0.0, 12.0, r#","speed":0.75"#, ""),
            1.0,
        ),
        (
            "a remapped soundtrack",
            layer(
                "V",
                "video",
                0.0,
                6.0,
                &format!(
                    r#"{{"asset":"s","timeRemap":true,"sourceTime":{remap},"embeddedAudio":true}}"#
                ),
                "",
            ),
            2.0,
        ),
    ];
    for (name, layer, at) in cases {
        let doc = || project(30.0, 12.0, std::slice::from_ref(&layer), &[], "");
        let (playback, mut dev, rig) = setup_doc(doc(), &[("s", &file)], SR, 2, 0.0);
        if loaded {
            rig.cache.set_decode_delay(Duration::from_millis(8));
        }
        playback
            .command(TransportCommand::Seek {
                time: ticks(at),
                mode: SeekMode::Exact,
            })
            .unwrap();
        playback
            .command(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            })
            .unwrap();
        dev.run(200);
        let stats = playback.stats();
        drop(playback);
        let played: Vec<f32> = dev
            .log
            .iter()
            .flat_map(|(_, b)| b.iter().step_by(2).copied())
            .collect();
        let start = played
            .iter()
            .position(|v| *v != 0.0)
            .unwrap_or_else(|| panic!("{name}: nothing played"));
        let n = 80_000;
        let from = (at * SR as f64).round() as i64;
        let mut m = rig.mixer(doc(), "C", MixFormat::stereo(SR));
        let offline = rig.render(&mut m, from, from + n as i64, 4096);
        let lead = offline[0].iter().position(|v| *v != 0.0).unwrap();
        assert!(start >= lead && played.len() >= start - lead + n, "{name}");
        let played = &played[start - lead..start - lead + n];
        let right: Vec<f32> = dev
            .log
            .iter()
            .flat_map(|(_, b)| b.iter().skip(1).step_by(2).copied())
            .collect();
        let right = &right[start - lead..start - lead + n];
        let differ = played
            .iter()
            .zip(&offline[0])
            .filter(|(a, b)| a != b)
            .count();
        let worst = played
            .iter()
            .zip(&offline[0])
            .fold(0f32, |w, (a, b)| w.max((a - b).abs()));
        eprintln!(
            "{name}: {} starved renders, {differ} of {n} samples differ from the offline render (worst {worst})",
            stats.starved_renders
        );
        eprintln!(
            "{name}: stats {stats:?}, first difference {:?}",
            played.iter().zip(&offline[0]).position(|(a, b)| a != b)
        );
        assert_eq!(stats.starved_renders, 0, "{name}: starved renders");
        assert_eq!(stats.underruns, 0, "{name}: device ring underruns");
        assert_eq!(
            differ, 0,
            "{name}: played audio differs from the offline render"
        );
        assert_eq!(right, &offline[1], "{name}: right channel differs");
    }
}

/// Straight-line fit `x ≈ a + b·j` over `(j, x)` points.
fn fit(points: &[(f64, f64)]) -> (f64, f64, f64) {
    let n = points.len() as f64;
    let (mj, mx) = points
        .iter()
        .fold((0.0, 0.0), |(a, b), (j, x)| (a + j / n, b + x / n));
    let (mut sjx, mut sjj) = (0.0, 0.0);
    for (j, x) in points {
        sjx += (j - mj) * (x - mx);
        sjj += (j - mj) * (j - mj);
    }
    let b = sjx / sjj;
    let a = mx - b * mj;
    let worst = points
        .iter()
        .fold(0f64, |w, (j, x)| w.max((a + b * j - x).abs()));
    (a, b, worst)
}

/// The DC gain of the rate converter: the marker channel (0.5) as heard,
/// over frames away from any edge or wrap (the median frame value).
fn dc_gain(heard: &[(f32, f32)]) -> f64 {
    let mut m: Vec<f64> = heard
        .iter()
        .map(|f| f.1 as f64)
        .filter(|v| (*v - 0.5).abs() < 0.001)
        .collect();
    m.sort_by(f64::total_cmp);
    let g = m[m.len() / 2] / 0.5;
    eprintln!("converter DC gain {:.3} ppm off unity", (g - 1.0) * 1e6);
    g
}

/// The heard media positions of a ramp, one per device frame, from the
/// first frame of media on.
fn heard_ramp(dev: &SimDevice, from: usize) -> Vec<(f32, f32)> {
    let frames: Vec<(f32, f32)> = dev.log[from..]
        .iter()
        .flat_map(|(_, b)| b.chunks(2).map(|f| (f[0], f[1])).collect::<Vec<_>>())
        .collect();
    let first = frames.iter().position(|f| f.1 > 0.1).expect("media played");
    frames[first..].to_vec()
}

/// A loop on a device at 44.1 kHz under a 48 kHz project: every pass is
/// heard whole (each 0.25 s pass lasts exactly 11 025 device frames and
/// ends where the next begins: nothing is cut at the wrap), the converter
/// runs through the wrap as one stream (no restart from silence), and the
/// clock line, wrapping by the loop range, names what is heard.
#[test]
fn loops_whole_at_another_device_rate() {
    let dir = temp_dir("loop-44k");
    let file = ramp_file(&dir, 6);
    let device = 44_100u32;
    let (playback, mut dev, _rig) = setup(&file, 6.0, device, 0.0);
    playback
        .command(TransportCommand::SetLoop {
            range: Some(TickRange {
                start: ticks(1.0),
                end: ticks(1.25),
            }),
        })
        .unwrap();
    playback
        .command(TransportCommand::Seek {
            time: ticks(1.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(300);
    let clock = playback.clock().unwrap();
    let stats = playback.stats();
    drop(playback);
    assert!(clock.loop_range.is_some());
    // Media positions per device frame (fractional project samples).
    let lead = dev.log.iter().position(|(_, b)| b[1] > 0.1).unwrap();
    let heard = heard_ramp(&dev, lead);
    // The converter's sinc passes DC at a gain a few ppm off 1, which scales
    // the ramp: read the gain off the constant marker channel and divide it
    // out, so positions are exact.
    let gain = dc_gain(&heard);
    let x: Vec<f64> = heard
        .iter()
        .map(|f| f.0 as f64 * 16_777_216.0 / gain)
        .collect();
    let (lo, hi) = (SR as f64, SR as f64 * 1.25);
    // Wraps: the steepest fall of each junction.
    let mut wraps: Vec<usize> = vec![];
    for j in 1..x.len() {
        if x[j - 1] - x[j] > 4_000.0 {
            match wraps.last_mut() {
                Some(w) if j - *w < 64 => {
                    if x[j - 1] - x[j] > x[*w - 1] - x[*w] {
                        *w = j;
                    }
                }
                _ => wraps.push(j),
            }
        }
    }
    assert!(wraps.len() >= 8, "{} wraps", wraps.len());
    let ratio = SR as f64 / device as f64;
    // Fit each whole pass away from the band-limited junctions (the filter
    // reaches 256 project samples, about 235 device frames, each side).
    let margin = 400;
    let mut lines = vec![];
    for w in wraps.windows(2) {
        let pts: Vec<(f64, f64)> = (w[0] + margin..w[1] - margin)
            .map(|j| (j as f64, x[j]))
            .collect();
        let (a, b, worst) = fit(&pts);
        assert!(
            (b - ratio).abs() < 1e-6,
            "a pass advances {b} project samples a frame, expected {ratio}"
        );
        // f32 rounding in the filter: positions near 60 000 carry 2⁻⁸.
        assert!(worst < 0.1, "a pass is off a straight line by {worst}");
        lines.push((a, b));
    }
    let mut worst_gap = 0f64;
    let mut worst_len = 0f64;
    for k in 0..lines.len() {
        let (a, b) = lines[k];
        let begins = (lo - a) / b;
        let ends = (hi - a) / b;
        if let Some(&(a2, b2)) = lines.get(k + 1) {
            // Where this pass reaches the loop end, the next is at its start,
            // and passes begin 11 025 device frames apart.
            let next = (lo - a2) / b2;
            worst_gap = worst_gap.max((next - ends).abs() * b);
            worst_len = worst_len.max((next - begins - 11_025.0).abs());
        }
    }
    eprintln!(
        "44.1 kHz loop: {} passes, worst pass period error {worst_len:.4} device frames, \
         worst discontinuity at a wrap {worst_gap:.4} project samples",
        lines.len()
    );
    assert!(
        worst_len < 0.01,
        "passes begin {worst_len} frames off 11 025 apart"
    );
    assert!(
        worst_gap < 0.01,
        "{worst_gap} project samples are missing or repeated at a wrap"
    );
    // The clock names what is heard, through the wraps.
    let mut worst_clock = 0f64;
    let mut frame = 0usize;
    for (host, buf) in &dev.log[lead..] {
        let at = frame;
        frame += buf.len() / 2;
        if at < 2 * margin || wraps.iter().any(|&w| at.abs_diff(w) < margin) {
            continue;
        }
        if at >= x.len() {
            break;
        }
        let predicted = clock
            .media_time_at(HostMicros::new(host + LATENCY_US as u64).unwrap())
            .get() as f64
            * SR as f64
            / TICKS_PER_SECOND as f64;
        worst_clock = worst_clock.max((predicted - x[at]).abs());
    }
    eprintln!("44.1 kHz loop: worst clock error {worst_clock:.3} project samples");
    assert!(worst_clock < 0.25, "the clock is {worst_clock} samples off");
    assert_eq!(stats.starved_renders, 0);
}

/// The end of a run on a device at 44.1 kHz: the converter's partial chunk
/// and filter tail are flushed, so the last 0.5 s of a 48 kHz project lasts
/// exactly 22 050 device frames and reaches the composition's end.
#[test]
fn a_run_ends_whole_at_another_device_rate() {
    let dir = temp_dir("end-44k");
    let file = ramp_file(&dir, 2);
    let device = 44_100u32;
    let (playback, mut dev, _rig) = setup(&file, 2.0, device, 0.0);
    playback
        .command(TransportCommand::Seek {
            time: ticks(1.5),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(70);
    drop(playback);
    let lead = dev.log.iter().position(|(_, b)| b[1] > 0.1).unwrap();
    let heard = heard_ramp(&dev, lead);
    let last = heard.iter().rposition(|f| f.1 > 0.1).unwrap();
    assert!(
        heard[last + 1..].iter().all(|f| f.0 == 0.0 && f.1 == 0.0),
        "silence after the run"
    );
    let gain = dc_gain(&heard[..last]);
    let x: Vec<f64> = heard
        .iter()
        .map(|f| f.0 as f64 * 16_777_216.0 / gain)
        .collect();
    let ratio = SR as f64 / device as f64;
    let pts: Vec<(f64, f64)> = (400..last - 400).map(|j| (j as f64, x[j])).collect();
    let (a, b, _) = fit(&pts);
    eprintln!(
        "44.1 kHz run end: {} device frames heard; the last at media {:.2} (end {})",
        last + 1,
        a + b * last as f64,
        2 * SR
    );
    assert_eq!(last + 1, 22_050, "device frames heard");
    // The stream's first frame sits at rubato's phase, (delay + 1)·p/d − 256
    // project samples in (delay = ⌊512·d/p / 2⌋ = 235): 0.8707.
    let delay = (512 * device as u64 / (2 * SR as u64)) as f64;
    let phase = (delay + 1.0) * ratio - 256.0;
    assert!(
        (a - 1.5 * SR as f64 - phase).abs() < 0.05,
        "the first frame at {a}, expected {}",
        1.5 * SR as f64 + phase
    );
    assert!((b - ratio).abs() < 1e-6, "{b} project samples a frame");
    // The last frame is the last whole frame before the composition's end.
    assert!(a + b * last as f64 > 2.0 * SR as f64 - 1.1);
}

/// A mono device hears the bus folded, ½(L + R), not its left channel.
#[test]
fn a_mono_device_hears_the_fold() {
    let dir = temp_dir("mono-device");
    let file = dir.join("pair.wav");
    write_wav(
        &file,
        SR,
        &[vec![0.3; SR as usize * 2], vec![0.1; SR as usize * 2]],
    );
    let doc = project(30.0, 2.0, &[audio("A", "s", 0.0, 2.0, "", "")], &[], "");
    let (playback, mut dev, _rig) = setup_doc(doc, &[("s", &file)], SR, 1, 0.0);
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(40);
    drop(playback);
    let heard: Vec<f32> = dev
        .log
        .iter()
        .flat_map(|(_, b)| b.iter().copied())
        .filter(|v| *v != 0.0)
        .collect();
    assert!(heard.len() > 10_000);
    assert!(
        heard.iter().all(|v| (*v - 0.2).abs() < 1e-6),
        "{:?}",
        &heard[..4]
    );
}

/// A pause sent right after a seek, before the device has presented the
/// seek's first buffer, stops at the seek target, not on the old line.
#[test]
fn a_pause_during_a_seek_pre_roll_stops_at_the_target() {
    let dir = temp_dir("pause-preroll");
    let file = ramp_file(&dir, 6);
    let (playback, mut dev, _rig) = setup(&file, 6.0, SR, 0.0);
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(100);
    playback
        .command(TransportCommand::Seek {
            time: ticks(4.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback.command(TransportCommand::Pause).unwrap();
    let stopped = playback.clock().unwrap();
    assert_eq!(stopped.state, TransportState::Stopped);
    assert_eq!(stopped.media_time, ticks(4.0), "paused at the seek target");
}

/// A run from a playhead before the loop (the work area set, the playhead
/// before it, play): the audio plays straight into the loop and then wraps,
/// and every published sample names what the device presents. Samples
/// before the loop carry no loop range (a line that wrapped by it would put
/// the listener inside the loop); samples inside carry it. On a device at
/// the project rate and through the converter at 44.1 kHz.
#[test]
fn a_run_from_before_the_loop_is_on_the_clock() {
    let dir = temp_dir("pre-loop");
    let file = ramp_file(&dir, 6);
    for device in [SR, 44_100] {
        let (playback, mut dev, _rig) = setup(&file, 6.0, device, 0.0);
        let (lo, hi) = (2.0, 2.5);
        playback
            .command(TransportCommand::SetLoop {
                range: Some(TickRange {
                    start: ticks(lo),
                    end: ticks(hi),
                }),
            })
            .unwrap();
        playback
            .command(TransportCommand::Seek {
                time: ticks(1.0),
                mode: SeekMode::Exact,
            })
            .unwrap();
        playback
            .command(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            })
            .unwrap();
        let mut samples: Vec<ClockSample> = vec![];
        let mut worst_s = 0f64;
        let (mut before, mut inside) = (0, 0);
        // About 2.3 s at 512 frames a buffer: 1 s before the loop, then
        // more than two passes.
        let buffers = (2.3 * device as f64 / BUFFER as f64) as usize;
        let exact = device == SR;
        for _ in 0..buffers {
            dev.run(1);
            let (host, buf) = dev.log.last().unwrap().clone();
            if let Some(s) = playback.clock()
                && samples.last().is_none_or(|l| l.seq != s.seq)
            {
                samples.push(s);
            }
            let marked = if exact {
                buf[1] == 0.5
            } else {
                (buf[1] - 0.5).abs() < 0.01
            };
            let Some(s) = samples.last() else { continue };
            if !marked || s.state != TransportState::Playing {
                continue;
            }
            let actual = position_of(buf[0]) as f64 / SR as f64;
            // Through the converter, the sinc smears each wrap over about
            // 5 ms; and its DC gain scales the ramp by a few ppm.
            if !exact && ((actual - lo).abs() < 0.01 || (actual - hi).abs() < 0.01) {
                continue;
            }
            let predicted = s
                .media_time_at(HostMicros::new(host + LATENCY_US as u64).unwrap())
                .get() as f64
                / TICKS_PER_SECOND as f64;
            worst_s = worst_s.max((predicted - actual).abs());
            if actual < lo {
                before += 1;
                assert_eq!(s.loop_range, None, "before the loop at {actual}");
            } else {
                inside += 1;
            }
        }
        let playing: Vec<&ClockSample> = samples
            .iter()
            .filter(|s| s.state == TransportState::Playing)
            .collect();
        assert!(
            before > 60 && inside > 60,
            "{before} before, {inside} inside"
        );
        assert!(playing.iter().all(|s| s.epoch == playing[0].epoch));
        let entered = playing
            .iter()
            .position(|s| s.loop_range.is_some())
            .expect("the line wraps once inside the loop");
        assert!(playing[entered..].iter().all(|s| s.loop_range.is_some()));
        // Published at the refresh rate (about 2.3 s: ~23 refreshes plus
        // the loop entry), not on every callback.
        assert!(
            playing.len() <= 30,
            "{} samples for {buffers} buffers",
            playing.len()
        );
        let bound = if exact { 60e-6 } else { 1e-3 };
        eprintln!(
            "device {device} Hz, loop from before: {} samples, worst |predicted − presented| {:.1} µs",
            playing.len(),
            worst_s * 1e6
        );
        assert!(worst_s < bound, "worst {:.1} µs", worst_s * 1e6);
        drop(playback);
    }
}

/// A pause during the run before the loop stops where the listener is, not
/// inside the loop.
#[test]
fn a_pause_before_the_loop_stops_where_heard() {
    let dir = temp_dir("pre-loop-pause");
    let file = ramp_file(&dir, 6);
    let (playback, mut dev, _rig) = setup(&file, 6.0, SR, 0.0);
    playback
        .command(TransportCommand::SetLoop {
            range: Some(TickRange {
                start: ticks(4.0),
                end: ticks(4.5),
            }),
        })
        .unwrap();
    playback
        .command(TransportCommand::Seek {
            time: ticks(1.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(60);
    let (last_host, last_buf) = dev
        .log
        .iter()
        .rev()
        .find(|(_, b)| b[1] == 0.5)
        .unwrap()
        .clone();
    playback.command(TransportCommand::Pause).unwrap();
    let stopped = playback.clock().unwrap();
    assert_eq!(stopped.state, TransportState::Stopped);
    assert_eq!(stopped.loop_range, None);
    let heard_now = position_of(last_buf[0]) as f64 / SR as f64
        + (host_now_us() as f64 - (last_host + LATENCY_US as u64) as f64) / 1e6;
    let paused_at = stopped.media_time.get() as f64 / TICKS_PER_SECOND as f64;
    assert!(
        paused_at < 4.0 && (paused_at - heard_now).abs() < 0.03,
        "paused at {paused_at}, heard {heard_now}"
    );
}

/// The media positions every frame heard plays (the ramp's left channel),
/// with the host µs at which each frame reaches the listener.
fn heard_positions(dev: &SimDevice, from: usize) -> Vec<(f64, i64)> {
    let mut out = vec![];
    for (host, buf) in &dev.log[from..] {
        for (i, f) in buf.chunks(2).enumerate() {
            if f[1] == 0.5 {
                let at = *host as f64 + LATENCY_US as f64 + i as f64 * 1e6 / dev.rate as f64;
                out.push((at, position_of(f[0])));
            }
        }
    }
    out
}

/// J/K/L at −1×: the mix plays backwards, sample by sample, on a clock line
/// running backwards; the run stops after sample 0 and the clock rests
/// there.
#[test]
fn reverse_playback_plays_backwards_on_its_clock() {
    let dir = temp_dir("jkl-reverse");
    let file = ramp_file(&dir, 4);
    let (playback, mut dev, _rig) = setup(&file, 4.0, SR, 0.0);
    playback
        .command(TransportCommand::Seek {
            time: ticks(0.8),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate { num: -1, den: 1 },
        })
        .unwrap();
    dev.run(30);
    let clock = playback.clock().unwrap();
    assert_eq!(clock.rate, PlaybackRate { num: -1, den: 1 });
    assert_eq!(clock.state, TransportState::Shuttling);
    let heard = heard_positions(&dev, 0);
    assert_eq!(
        heard[0].1,
        (0.8 * SR as f64) as i64,
        "starts on the playhead"
    );
    for w in heard.windows(2) {
        assert_eq!(w[1].1, w[0].1 - 1, "one sample back per frame");
    }
    let mut worst = 0f64;
    for (at, pos) in &heard[heard.len() / 2..] {
        let predicted = clock
            .media_time_at(HostMicros::new(*at as u64).unwrap())
            .get() as f64
            / TICKS_PER_SECOND as f64;
        worst = worst.max((predicted - *pos as f64 / SR as f64).abs());
    }
    eprintln!("−1×: worst |line − heard| {:.1} µs", worst * 1e6);
    assert!(worst < 1e-3, "{worst}");
    // On to sample 0, then stopped there.
    dev.run(90);
    let end = playback.clock().unwrap();
    assert_eq!(end.state, TransportState::Stopped);
    assert_eq!(end.media_time.get(), 0);
    let heard = heard_positions(&dev, 0);
    assert_eq!(heard.last().unwrap().1, 0, "the run plays sample 0 last");
}

/// J/K/L at 2×, −2×, and −1× (also through the device-rate converter):
/// the line runs at the rate in either
/// direction; what plays is the mix at twice the speed at its pitch (a
/// 1 kHz tone stays 1 kHz), and sounds land where the line says (bursts
/// heard within 2 ms of the line's instant for their media).
#[test]
fn jkl_rates_keep_pitch_and_stay_on_their_line() {
    let dir = temp_dir("jkl-double");
    let file = dir.join("bursts.wav");
    let n = SR as usize * 8;
    let mut left = vec![0f32; n];
    let bursts: Vec<i64> = (1..8).map(|s| s * SR as i64).collect();
    for (i, v) in left.iter_mut().enumerate() {
        let t = i as f64 / SR as f64;
        *v = (0.2 * (std::f64::consts::TAU * 1000.0 * t).sin()) as f32;
    }
    for &b in &bursts {
        for d in -64i64..64 {
            let w = 0.5 + 0.5 * (std::f64::consts::PI * d as f64 / 64.0).cos();
            left[(b + d) as usize] +=
                (0.7 * w * (std::f64::consts::TAU * 6000.0 * d as f64 / SR as f64).sin()) as f32;
        }
    }
    write_wav(&file, SR, &[left, vec![0.5; n]]);
    for (rate, from, device) in [
        (2i32, 0.5f64, SR),
        (-2, 7.5, SR),
        (2, 0.5, 44_100),
        (-1, 7.5, 44_100),
    ] {
        let (playback, mut dev, _rig) = setup(&file, 8.0, device, 0.0);
        playback
            .command(TransportCommand::Seek {
                time: ticks(from),
                mode: SeekMode::Exact,
            })
            .unwrap();
        playback
            .command(TransportCommand::Play {
                rate: PlaybackRate { num: rate, den: 1 },
            })
            .unwrap();
        dev.run(220); // 2.3 s: 4.7 s of media at 2×
        let clock = playback.clock().unwrap();
        assert_eq!(clock.rate.num, rate);
        // The heard signal (left channel) with its listener instants.
        let mut heard: Vec<(f64, f32)> = vec![];
        for (host, buf) in &dev.log {
            for (i, f) in buf.chunks(2).enumerate() {
                let at = *host as f64 + LATENCY_US as f64 + i as f64 * 1e6 / device as f64;
                heard.push((at, f[0]));
            }
        }
        // Pitch of the bed between bursts: zero crossings over 0.25 s.
        let line = |at: f64| {
            clock
                .media_time_at(HostMicros::new(at as u64).unwrap())
                .get() as f64
                / TICKS_PER_SECOND as f64
        };
        let quiet: Vec<f32> = heard
            .iter()
            .filter(|(at, _)| {
                let m = line(*at);
                (m.fract() - 0.5).abs() < 0.3 && (m - from).abs() > 0.4
            })
            .map(|(_, v)| *v)
            .collect();
        let seconds = quiet.len() as f64 / device as f64;
        let crossings = quiet
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        let hz = crossings as f64 / seconds;
        // Burst positions: energy of the first difference (above the bed).
        let mut errors = vec![];
        for &b in &bursts {
            let media = b as f64 / SR as f64;
            let near: Vec<(f64, f64)> = heard
                .windows(2)
                .filter(|w| (line(w[1].0) - media).abs() < 0.04)
                .map(|w| (w[1].0, ((w[1].1 - w[0].1) as f64).powi(2)))
                .collect();
            let energy: f64 = near.iter().map(|x| x.1).sum();
            if near.len() < 800 || energy < 1.0 {
                continue;
            }
            let centroid = near.iter().map(|x| x.0 * x.1).sum::<f64>() / energy;
            // Where the line puts the burst's media, in host µs.
            let expected = clock.host_time.get() as f64
                + (media - clock.media_time.get() as f64 / TICKS_PER_SECOND as f64) * 1e6
                    / rate as f64;
            errors.push((centroid - expected) / 1000.0);
        }
        eprintln!(
            "{rate}× on a {device} Hz device: bed {hz:.1} Hz over {seconds:.2} s, burst offsets {errors:.2?} ms"
        );
        assert!((hz - 1000.0).abs() < 15.0, "{rate}×: {hz} Hz");
        assert!(errors.len() >= 3, "{rate}×: {errors:?}");
        for e in &errors {
            assert!(e.abs() < 2.0, "{rate}×: {errors:?}");
        }
    }
}

/// A new J/K/L rate while playing continues from what is heard, on a new
/// epoch.
#[test]
fn a_rate_change_continues_from_what_is_heard() {
    let dir = temp_dir("jkl-change");
    let file = ramp_file(&dir, 8);
    let (playback, mut dev, _rig) = setup(&file, 8.0, SR, 0.0);
    playback
        .command(TransportCommand::Seek {
            time: ticks(2.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(60);
    let before = playback.clock().unwrap();
    let (last_host, last_buf) = dev
        .log
        .iter()
        .rev()
        .find(|(_, b)| b[1] == 0.5)
        .unwrap()
        .clone();
    let heard_now = position_of(last_buf[0]) as f64 / SR as f64
        + (host_now_us() as f64 - (last_host + LATENCY_US as u64) as f64) / 1e6;
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate { num: -1, den: 1 },
        })
        .unwrap();
    let mark = dev.log.len();
    dev.run(40);
    let after = playback.clock().unwrap();
    assert!(after.epoch > before.epoch);
    assert_eq!(after.rate.num, -1);
    let first = heard_positions(&dev, mark)
        .into_iter()
        .map(|(_, p)| p)
        .find(|_| true)
        .unwrap();
    let first_s = first as f64 / SR as f64;
    eprintln!("1× → −1×: heard {heard_now:.4} s at the command, reverse starts at {first_s:.4} s");
    assert!(
        (first_s - heard_now).abs() < 0.03,
        "{first_s} vs {heard_now}"
    );
}

/// A source miss must not enter any stateful DSP stage. Compare a cold,
/// delayed run with a fully decoded run, including the master 2x stretch,
/// reverse's free-position reader, and the device-rate converter.
#[test]
fn starved_playback_recovers_without_poisoning_dsp() {
    let _load = CpuContention::start();
    let dir = temp_dir("playback-starve");
    let file = dir.join("tone.wav");
    write_wav(
        &file,
        44_100,
        &[tone(44_100, 8.0, 997.0, 0.2), tone(44_100, 8.0, 631.0, 0.1)],
    );
    for (rate, device_rate) in [(1, SR), (2, 44_100), (-2, 44_100)] {
        let rig = Rig::new(&[("s", &file)]);
        // Cold source per case, with background workers as in production.
        let cache = SourceCache::new(64 << 20, 2);
        cache.set_decode_delay(Duration::from_millis(20));
        let clip = audio("A", "s", 0.0, 4.0, r#", "speed":1.5"#, "");
        let mut captures = Vec::new();
        for cold in [true, false] {
            let (playback, side) = Playback::start_headless(
                MixFormat::stereo(SR),
                cache.clone(),
                PlaybackConfig {
                    preroll_ms: 0,
                    preroll_wait_ms: 0,
                    ..PlaybackConfig::default()
                },
                device_rate,
                2,
                LATENCY_US,
            )
            .unwrap();
            let resolver = |id: &str| rig.media.get(id).cloned();
            let problems = playback
                .set_project(
                    Arc::new(project(30.0, 4.0, std::slice::from_ref(&clip), &[], "")),
                    "C",
                    &resolver,
                )
                .unwrap();
            assert!(problems.is_empty());
            playback
                .command(TransportCommand::Seek {
                    time: ticks(2.0),
                    mode: SeekMode::Exact,
                })
                .unwrap();
            playback
                .command(TransportCommand::Play {
                    rate: PlaybackRate { num: rate, den: 1 },
                })
                .unwrap();
            let mut dev = SimDevice::new(side, device_rate, 2, 0.0);
            dev.run(90);
            let stats = playback.stats();
            drop(playback);
            let samples: Vec<f32> = dev
                .log
                .iter()
                .flat_map(|(_, b)| b.iter().copied())
                .collect();
            let first = samples
                .chunks_exact(2)
                .position(|s| s.iter().any(|&v| v != 0.0))
                .unwrap()
                * 2;
            captures.push(samples[first..first + 48_000].to_vec());
            if cold {
                assert!(
                    stats.starved_renders > 0,
                    "{rate}x/{device_rate}: forced miss must be counted"
                );
            } else {
                assert_eq!(stats.starved_renders, 0, "{rate}x/{device_rate}: warm run");
            }
        }
        assert_eq!(
            captures[0], captures[1],
            "{rate}x/{device_rate}: delayed decode changes decoded audio"
        );
        cache.shutdown();
    }
}
