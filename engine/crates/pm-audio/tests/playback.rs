//! Playback against contract 6 and section 15 with a simulated device: the
//! clock line predicts what the device presents (also when the device's
//! sample clock drifts against the host clock), what plays equals the
//! offline render from the same sample (deterministic pre-roll), and pause,
//! seek, loop, and scrub behave.

mod common;

use std::collections::HashSet;
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
    /// How long a callback waits for the render thread (see [`Self::run`]).
    patience: Duration,
    /// Waits for a whole buffer of the current epoch (see [`Self::patient`]).
    patient: bool,
    /// Callbacks that gave up waiting.
    expired: usize,
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
            patience: Duration::from_millis(40),
            patient: false,
            expired: 0,
            start_host: host_now_us(),
            frames: 0,
            out: vec![0.0; BUFFER * channels],
            log: vec![],
        }
    }

    /// A device that waits for every buffer, for tests of what is rendered
    /// rather than of real-time delivery: a render thread deliberately
    /// stalled on a missing source block would otherwise underrun the ring
    /// (a counted, separate drop) and displace what the test compares.
    /// It waits for a whole buffer of frames of the current epoch (chunks
    /// can be short, and a seek's dropped chunks still count as queued), and
    /// still gives up after `patience`, so a render that never resumes fails
    /// as an underrun instead of hanging the test.
    fn patient(mut self, patience: Duration) -> SimDevice {
        self.patience = patience;
        self.patient = true;
        self
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
            let ready = |side: &DeviceSide| {
                if self.patient {
                    side.queued_frames() >= BUFFER
                } else {
                    side.queued_chunks() >= need
                }
            };
            while !ready(&self.side) && patience.elapsed() < self.patience {
                std::thread::sleep(Duration::from_micros(500));
            }
            if !ready(&self.side) {
                self.expired += 1;
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

/// Both input blocks for a fresh mapped start must be queued while decode
/// is blocked; discovering the second only after the first decodes serializes
/// the artificial/real decode latency and can miss the first callback.
#[test]
fn a_cold_mapped_start_queues_its_preroll_blocks_together() {
    let dir = temp_dir("mapped-preroll-prefetch");
    let file = dir.join("tone.m4a");
    generate(
        &file,
        "aevalsrc=0.125*sin(2*PI*441*t)|0.125*sin(2*PI*660*t):s=44100:d=20",
        &["-c:a", "aac", "-b:a", "160k"],
    );
    let cache = SourceCache::new(64 << 20, 2);
    let source = cache.open(&file, None).unwrap();
    cache.set_decode_blocked(&source, true);
    let (playback, mut side) = Playback::start_headless(
        MixFormat::stereo(SR),
        cache.clone(),
        PlaybackConfig {
            preroll_wait_ms: 4000,
            ..Default::default()
        },
        SR,
        2,
        LATENCY_US,
    )
    .unwrap();
    // Set the target before Project can launch an obsolete start at zero.
    playback
        .command(TransportCommand::Seek {
            time: ticks(2.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
    let remap = keys(&[(0.0, 2.0), (6.0, 10.0)]);
    let clip = layer(
        "V",
        "video",
        0.0,
        6.0,
        &format!(r#"{{"asset":"s","timeRemap":true,"sourceTime":{remap},"embeddedAudio":true}}"#),
        "",
    );
    playback
        .set_project(
            Arc::new(project(30.0, 12.0, &[clip], &[], "")),
            "C",
            &|_: &str| Some(file.clone()),
        )
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_millis(100);
    while cache.blocked_decoders() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let blocked = cache.blocked_decoders();
    cache.set_decode_blocked(&source, false);
    until(|| side.queued_frames() >= BUFFER);
    let mut out = vec![0.0; BUFFER * 2];
    side.callback(&mut out, 2, host_now_us());
    assert!(out.iter().any(|v| *v != 0.0));
    drop(playback);
    cache.shutdown();
    assert_eq!(
        blocked, 2,
        "both decode workers must receive needed pre-roll blocks before either finishes"
    );
}

fn time_mapped_playback(loaded: bool) {
    // Real-time comparisons (40 ms device, zero underruns) never overlap the
    // sixteen-voice start tests' bursts of user-interactive pool jobs.
    let _isolated = POOL_BURSTS.read().unwrap_or_else(|e| e.into_inner());
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
        let playback_timing = playback.start_timing();
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
        eprintln!(
            "{name}: first program frame {start}, offline lead {lead}, expired callbacks {}, first queued after control {:?} ms",
            dev.expired,
            playback_timing.map(|t| (t.queued_us - t.applied_us) as f64 / 1000.0)
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
            // This test is about what the render thread queues once the
            // source decodes, not about real-time delivery: its forced
            // 20 ms decode delay with no pre-roll wait stalls the render
            // thread on purpose, which on a loaded machine outlasts a
            // real-time device's patience and underruns the ring (counted
            // separately; `a_partial_callback_underrun_is_reported_and_recovers_in_order`).
            let mut dev = SimDevice::new(side, device_rate, 2, 0.0).patient(Duration::from_secs(5));
            // Within the run (2× from 2 s of a 4 s timeline plays one
            // second): a patient device would wait out its patience after
            // the end.
            dev.run(60);
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
            assert_eq!(
                stats.underruns, 0,
                "{rate}x/{device_rate}: a patient device never reads an empty ring"
            );
            assert_eq!(
                stats.dropped_renders, 0,
                "{rate}x/{device_rate}: cold DSP priming must retain its no-loss hold: {stats:?}"
            );
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

/// A seek after a starve is a new request: the stretch starts at the seek
/// target by the pre-roll rules and plays what an offline render from
/// there plays, instead of continuing the starved run and walking it over
/// the gap (each block of which would miss on a cold cache).
#[test]
fn a_seek_after_a_starve_plays_as_rendered_offline() {
    let _load = CpuContention::start();
    let dir = temp_dir("playback-seek-after-starve");
    let file = dir.join("tone.wav");
    write_wav(
        &file,
        44_100,
        &[
            tone(44_100, 30.0, 997.0, 0.2),
            tone(44_100, 30.0, 631.0, 0.1),
        ],
    );
    let rig = Rig::new(&[("s", &file)]);
    let cache = SourceCache::new(64 << 20, 2);
    // Slow enough that the first segment is still starved when the seek
    // comes.
    cache.set_decode_delay(Duration::from_millis(300));
    let clip = audio("A", "s", 0.0, 30.0, r#", "speed":0.75"#, "");
    let doc = || project(30.0, 30.0, std::slice::from_ref(&clip), &[], "");
    let (playback, side) = Playback::start_headless(
        MixFormat::stereo(SR),
        cache.clone(),
        PlaybackConfig {
            preroll_ms: 0,
            preroll_wait_ms: 0,
            ..PlaybackConfig::default()
        },
        SR,
        2,
        LATENCY_US,
    )
    .unwrap();
    let resolver = |id: &str| rig.media.get(id).cloned();
    assert!(
        playback
            .set_project(Arc::new(doc()), "C", &resolver)
            .unwrap()
            .is_empty()
    );
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    let waited = std::time::Instant::now();
    while playback.stats().starved_renders == 0 && waited.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(1));
    }
    let before = playback.stats();
    assert!(before.starved_renders > 0, "the cold start starves");
    // Seek far forward, past anything decoded, and play on.
    let target = 20.0;
    playback
        .command(TransportCommand::Seek {
            time: ticks(target),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    let mut dev = SimDevice::new(side, SR, 2, 0.0).patient(Duration::from_secs(5));
    dev.run(120);
    let stats = playback.stats();
    drop(playback);
    cache.shutdown();
    // The seek may drop queued chunks between the device's check and its
    // read (an underrun at the seek, as on a real device, before the
    // target's audio); after that the device always found a whole buffer.
    assert_eq!(dev.expired, 0, "the render thread kept up after the seek");
    let played: Vec<f32> = dev
        .log
        .iter()
        .flat_map(|(_, b)| b.iter().step_by(2).copied())
        .collect();
    let from = (target * SR as f64).round() as i64;
    let n = 48_000;
    let mut m = rig.mixer(doc(), "C", MixFormat::stereo(SR));
    let offline = rig.render(&mut m, from, from + n as i64, 4096);
    let head = &offline[0][..256];
    let at = played
        .windows(head.len())
        .position(|w| w == head)
        .unwrap_or_else(|| panic!("the seek target's audio never played ({stats:?})"));
    assert!(
        played.len() >= at + n,
        "played {} after the seek",
        played.len() - at
    );
    let differ = played[at..at + n]
        .iter()
        .zip(&offline[0])
        .filter(|(a, b)| a != b)
        .count();
    eprintln!(
        "seek after starve: {} starved renders before, {} after; {differ} of {n} samples differ",
        before.starved_renders, stats.starved_renders
    );
    assert_eq!(
        differ, 0,
        "playback after the seek differs from the offline render"
    );
}

/// A callback-driven, deterministic stuck decoder probe mid-run: both voices
/// start ready and the ring is primed, then the late voice's next source
/// block is held at its worker. A wait the ring's reserve covers stays bit
/// exact; a stuck voice drops before the reserve is spent, so the ready
/// voice never underruns. No wall-time decode delay or simulated-device
/// patience hides it. (Cold starts: `a_cold_start_holds_by_device_time`.)
#[test]
fn a_long_decode_stall_is_reported_and_recovers() {
    bounded_decode_wait(true, 48_000, 2);
    bounded_decode_wait(true, 44_100, 0);
}

#[test]
fn a_decode_wait_below_the_bound_is_bit_exact() {
    bounded_decode_wait(false, 48_000, 2);
    bounded_decode_wait(false, 44_100, 0);
}

#[track_caller]
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "worker/render did not progress"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn bounded_decode_wait(exceed: bool, device_rate: u32, workers: usize) {
    let _load = stress_enabled().then(CpuContention::start);
    let dir = temp_dir(&format!("reserve-{exceed}-{device_rate}"));
    let good_file = dir.join("good.wav");
    let late_file = dir.join("late.wav");
    let n = 3 * pm_audio::source::BLOCK;
    let good: Vec<f32> = (0..n).map(|i| 0.1 + (i % 997) as f32 / 10_000.0).collect();
    let late: Vec<f32> = (0..n).map(|i| 0.2 + (i % 991) as f32 / 10_000.0).collect();
    write_wav(&good_file, device_rate, &[good.clone(), vec![0.0; n]]);
    write_wav(&late_file, device_rate, &[vec![0.0; n], late.clone()]);
    let cache = SourceCache::new(64 << 20, workers);
    let good_source = cache.open(&good_file, None).unwrap();
    for block in 0..3 {
        cache.block(&good_source, block, true).unwrap();
    }
    let late_source = cache.open(&late_file, None).unwrap();
    cache.block(&late_source, 0, true).unwrap();
    cache.set_decode_blocked(&late_source, true);
    let (playback, mut side) = Playback::start_headless(
        MixFormat::stereo(device_rate),
        cache.clone(),
        PlaybackConfig {
            preroll_ms: 0,
            preroll_wait_ms: 0,
            lookahead_ms: 0,
            ..PlaybackConfig::default()
        },
        device_rate,
        2,
        LATENCY_US,
    )
    .unwrap();
    let doc = project(
        30.0,
        n as f64 / device_rate as f64,
        &[
            audio("good", "g", 0.0, n as f64 / device_rate as f64, "", ""),
            audio("late", "s", 0.0, n as f64 / device_rate as f64, "", ""),
        ],
        &[],
        "",
    );
    let resolver = |id: &str| {
        Some(if id == "g" {
            good_file.clone()
        } else {
            late_file.clone()
        })
    };
    assert!(
        playback
            .set_project(Arc::new(doc), "C", &resolver)
            .unwrap()
            .is_empty()
    );
    let from = pm_audio::source::BLOCK - 8192;
    playback
        .command(TransportCommand::Seek {
            time: ticks(from as f64 / device_rate as f64),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    until(|| side.queued_frames() >= 3072);
    let host_start = host_now_us();
    let mut out = vec![0.0; 256 * 2];
    let mut played = Vec::new();
    let mut released = false;
    let mut crossed = false;
    let mut held_at = 0;
    let mut first_drop = None;
    for turn in 0..80 {
        // No patient-device wait: the source policy must keep the ready
        // voice in the ring even with the other worker stuck indefinitely.
        assert!(
            side.queued_frames() >= 256,
            "reserve exhausted: {:?}",
            playback.stats()
        );
        side.callback(
            &mut out,
            2,
            host_start + turn * 256 * 1_000_000 / device_rate as u64,
        );
        played.extend_from_slice(&out);
        let clock = playback.clock().unwrap();
        let host = host_start + turn * 256 * 1_000_000 / device_rate as u64;
        let actual = clock
            .media_time_at(HostMicros::new(host + LATENCY_US as u64).unwrap())
            .get();
        let expected = (from as i64 + turn as i64 * 256) * TICKS_PER_SECOND / device_rate as i64;
        assert!(
            (actual - expected).abs() <= TICKS_PER_SECOND / 500_000,
            "device clock: {actual} vs {expected}"
        );
        std::thread::sleep(Duration::from_secs_f64(256.0 / device_rate as f64));
        if !crossed && playback.stats().starved_renders > 0 {
            crossed = true;
            held_at = turn;
            // The render can count the miss before the decode worker it
            // queued reaches the gate; wait (wall time only, no device
            // callbacks) until the worker is actually held there.
            until(|| cache.blocked_decoders() > 0);
            if !exceed {
                // Wall time alone cannot spend the buffered device frames.
                std::thread::sleep(Duration::from_millis(130));
                assert_eq!(playback.stats().dropped_renders, 0);
                cache.set_decode_blocked(&late_source, false);
                until(|| cache.is_ready(&late_source, pm_audio::source::BLOCK as i64, 256));
                released = true;
            }
        }
        if playback.stats().dropped_renders > 0 {
            first_drop.get_or_insert(turn);
        }
        if exceed && !released && playback.stats().dropped_renders > 0 && turn > held_at + 12 {
            // The default 80 ms ring forces a drop sooner than 120 ms.
            assert!((first_drop.unwrap() - held_at) * 256 < device_rate as u64 * 120 / 1000);
            let errors = playback.take_errors();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert_eq!(
                errors[0],
                "Audio decode fell behind the output buffer; missing voice plays as silence so the others keep playing: late"
            );
            cache.set_decode_blocked(&late_source, false);
            until(|| cache.is_ready(&late_source, pm_audio::source::BLOCK as i64, 256));
            released = true;
        }
    }
    assert!(crossed && released);
    let mut missing = 0;
    let mut resumed = false;
    for (i, pair) in played.chunks_exact(2).enumerate() {
        assert_eq!(pair[0], good[from + i], "ready voice at {i}");
        if pair[1] == 0.0 {
            assert!(!resumed);
            missing += 1;
        } else {
            if missing > 0 {
                resumed = true;
            }
            assert_eq!(pair[1], late[from + i], "late voice at {i}");
        }
    }
    let stats = playback.stats();
    assert_eq!(
        stats.underruns, 0,
        "a stuck voice must never silence ready voices"
    );
    if exceed {
        assert!(missing > 0 && resumed);
        assert_eq!(stats.dropped_voice_frames, missing);
    } else {
        assert_eq!(stats.dropped_voice_frames, 0);
        assert_eq!(missing, 0, "briefly late voice remains bit-exact");
    }
    assert!(playback.take_errors().is_empty());
    drop(playback);
    cache.shutdown();
}

/// A live edit while playing (a fader drag, a monitor solo toggle) keeps up
/// to four retune glides in flight, and a waiting render holds them, so the
/// mixer renders the missing voice once per mix. The reserve deadline must
/// still apply: one voice missing beside a ready one, under a drag, drops
/// on the ring's reserve and the ready voice never underruns.
#[test]
fn a_decode_miss_during_a_live_edit_keeps_ready_voices_heard() {
    decode_miss_while_editing(LiveEdit::Gain);
    decode_miss_while_editing(LiveEdit::Solo);
}

#[derive(Clone, Copy, Debug)]
enum LiveEdit {
    /// The late voice's gain, dragged.
    Gain,
    /// The monitor solo toggled between every strip and both strips, which
    /// glides without changing what is heard.
    Solo,
}

fn decode_miss_while_editing(edit: LiveEdit) {
    let _load = stress_enabled().then(CpuContention::start);
    let rate = 48_000u32;
    let dir = temp_dir(&format!("edit-miss-{edit:?}"));
    let good_file = dir.join("good.wav");
    let late_file = dir.join("late.wav");
    let n = 3 * pm_audio::source::BLOCK;
    let good: Vec<f32> = (0..n).map(|i| 0.1 + (i % 997) as f32 / 10_000.0).collect();
    let late: Vec<f32> = (0..n).map(|i| 0.2 + (i % 991) as f32 / 10_000.0).collect();
    write_wav(&good_file, rate, &[good.clone(), vec![0.0; n]]);
    write_wav(&late_file, rate, &[vec![0.0; n], late.clone()]);
    let cache = SourceCache::new(64 << 20, 2);
    let good_source = cache.open(&good_file, None).unwrap();
    for block in 0..3 {
        cache.block(&good_source, block, true).unwrap();
    }
    let late_source = cache.open(&late_file, None).unwrap();
    cache.block(&late_source, 0, true).unwrap();
    cache.set_decode_blocked(&late_source, true);
    let (playback, mut side) = Playback::start_headless(
        MixFormat::stereo(rate),
        cache.clone(),
        PlaybackConfig {
            preroll_ms: 0,
            preroll_wait_ms: 0,
            lookahead_ms: 0,
            ..PlaybackConfig::default()
        },
        rate,
        2,
        LATENCY_US,
    )
    .unwrap();
    let dur = n as f64 / rate as f64;
    let doc = |late_gain: f64| {
        project(
            30.0,
            dur,
            &[
                audio("good", "g", 0.0, dur, "", ""),
                audio(
                    "late",
                    "s",
                    0.0,
                    dur,
                    &format!(r#","gain":{late_gain}"#),
                    "",
                ),
            ],
            &[],
            "",
        )
    };
    let resolver = |id: &str| {
        Some(if id == "g" {
            good_file.clone()
        } else {
            late_file.clone()
        })
    };
    assert!(
        playback
            .set_project(Arc::new(doc(1.0)), "C", &resolver)
            .unwrap()
            .is_empty()
    );
    let from = pm_audio::source::BLOCK - 8192;
    playback
        .command(TransportCommand::Seek {
            time: ticks(from as f64 / rate as f64),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    until(|| side.queued_frames() >= 3072);
    let host_start = host_now_us();
    let mut out = vec![0.0; 256 * 2];
    let mut played = Vec::new();
    let mut held_at = None;
    let mut first_drop = None;
    let mut released_at = None;
    let both: HashSet<String> = ["good".to_string(), "late".to_string()].into();
    for turn in 0..90u64 {
        // The drag: one edit per callback (5.3 ms) against a 12 ms glide,
        // from before the miss until well after the voice re-enters.
        if released_at.is_none_or(|at| turn < at + 10) {
            match edit {
                LiveEdit::Gain => {
                    let gain = if turn % 2 == 0 { 0.75 } else { 0.5 };
                    playback
                        .set_project(Arc::new(doc(gain)), "C", &resolver)
                        .unwrap();
                }
                LiveEdit::Solo => {
                    playback.set_monitor_solo((turn % 2 == 0).then(|| both.clone()));
                }
            }
        }
        assert!(
            side.queued_frames() >= 256,
            "{edit:?}: the ready voice ran out at turn {turn}: {:?}",
            playback.stats()
        );
        side.callback(
            &mut out,
            2,
            host_start + turn * 256 * 1_000_000 / rate as u64,
        );
        played.extend_from_slice(&out);
        std::thread::sleep(Duration::from_secs_f64(256.0 / rate as f64));
        if held_at.is_none() && playback.stats().starved_renders > 0 {
            held_at = Some(turn);
        }
        if playback.stats().dropped_renders > 0 && first_drop.is_none() {
            first_drop = Some(turn);
        }
        if let (Some(drop), None) = (first_drop, released_at)
            && turn > drop + 8
        {
            cache.set_decode_blocked(&late_source, false);
            until(|| cache.is_ready(&late_source, pm_audio::source::BLOCK as i64, 256));
            released_at = Some(turn);
        }
    }
    let (held_at, first_drop) = (held_at.unwrap(), first_drop.unwrap());
    assert!(released_at.is_some());
    // The reserve, not the 120 ms hold, ended the wait.
    assert!(
        (first_drop - held_at) * 256 < rate as u64 * 120 / 1000,
        "{edit:?}: held from turn {held_at} to {first_drop}"
    );
    let errors = playback.take_errors();
    assert_eq!(
        errors,
        [
            "Audio decode fell behind the output buffer; missing voice plays as silence so the others keep playing: late"
        ],
        "{edit:?}"
    );
    let mut missing = 0u64;
    let mut resumed = false;
    for (i, pair) in played.chunks_exact(2).enumerate() {
        assert_eq!(pair[0], good[from + i], "{edit:?}: ready voice at {i}");
        if pair[1] == 0.0 {
            assert!(!resumed, "{edit:?}: one gap at {i}");
            missing += 1;
        } else {
            if missing > 0 {
                resumed = true;
            }
            if let LiveEdit::Solo = edit {
                assert_eq!(pair[1], late[from + i], "{edit:?}: late voice at {i}");
            }
        }
    }
    assert!(missing > 0 && resumed, "{edit:?}: {missing} {resumed}");
    let stats = playback.stats();
    assert_eq!(
        stats.underruns, 0,
        "{edit:?}: a stuck voice must never silence ready voices"
    );
    assert_eq!(stats.dropped_voice_frames, missing, "{edit:?}");
    drop(playback);
    cache.shutdown();
}

/// A play or seek whose voice is cold holds for up to 120 ms of device time,
/// as the device's callbacks keep running against an empty ring (a real
/// stream never waits for the render thread). Nothing of the run is queued,
/// so the hold is start latency, not a gap: below the bound every voice,
/// ready or late, plays bit-exact from the target; at the bound only the
/// missing voice drops, once, and re-enters sample-accurately. Covers a
/// cold start alone and beside a ready voice, from a stop and mid-run seek.
#[test]
fn a_cold_start_holds_by_device_time() {
    for (with_ready, seek) in [(false, false), (true, false), (false, true), (true, true)] {
        cold_start_decode_wait(false, with_ready, seek, 48_000, 2);
    }
    cold_start_decode_wait(false, true, true, 44_100, 0);
}

#[test]
fn a_cold_start_past_the_bound_drops_only_the_missing_voice() {
    for (with_ready, seek) in [(false, false), (true, false), (true, true)] {
        cold_start_decode_wait(true, with_ready, seek, 48_000, 2);
    }
    cold_start_decode_wait(true, true, false, 44_100, 0);
}

fn cold_start_decode_wait(
    exceed: bool,
    with_ready: bool,
    seek: bool,
    device_rate: u32,
    workers: usize,
) {
    let _load = stress_enabled().then(CpuContention::start);
    let case = format!("cold exceed={exceed} ready={with_ready} seek={seek} {device_rate}");
    let dir = temp_dir(&format!("cold-{exceed}-{with_ready}-{seek}-{device_rate}"));
    let good_file = dir.join("good.wav");
    let late_file = dir.join("late.wav");
    let n = 2 * pm_audio::source::BLOCK;
    let good: Vec<f32> = (0..n).map(|i| 0.1 + (i % 997) as f32 / 10_000.0).collect();
    let late: Vec<f32> = (0..n).map(|i| 0.2 + (i % 991) as f32 / 10_000.0).collect();
    write_wav(&good_file, device_rate, &[good.clone(), vec![0.0; n]]);
    write_wav(&late_file, device_rate, &[vec![0.0; n], late.clone()]);
    let cache = SourceCache::new(64 << 20, workers);
    let good_source = cache.open(&good_file, None).unwrap();
    for block in 0..2 {
        cache.block(&good_source, block, true).unwrap();
    }
    let late_source = cache.open(&late_file, None).unwrap();
    // A seek lands in block 1, cold; a play from the start in block 0.
    let target = if seek {
        cache.block(&late_source, 0, true).unwrap();
        pm_audio::source::BLOCK + 1_000
    } else {
        0
    };
    cache.set_decode_blocked(&late_source, true);
    let (playback, mut side) = Playback::start_headless(
        MixFormat::stereo(device_rate),
        cache.clone(),
        PlaybackConfig {
            preroll_ms: 0,
            preroll_wait_ms: 0,
            lookahead_ms: 0,
            ..PlaybackConfig::default()
        },
        device_rate,
        2,
        LATENCY_US,
    )
    .unwrap();
    let dur = n as f64 / device_rate as f64;
    let mut clips = vec![audio("late", "s", 0.0, dur, "", "")];
    if with_ready {
        clips.push(audio("good", "g", 0.0, dur, "", ""));
    }
    let doc = project(30.0, dur, &clips, &[], "");
    let resolver = |id: &str| {
        Some(if id == "g" {
            good_file.clone()
        } else {
            late_file.clone()
        })
    };
    assert!(
        playback
            .set_project(Arc::new(doc), "C", &resolver)
            .unwrap()
            .is_empty()
    );
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    let host_start = host_now_us();
    let host = |frame: u64| host_start + frame * 1_000_000 / device_rate as u64;
    let mut device = 0u64;
    let mut out = vec![0.0; 256 * 2];
    if seek {
        // An established run first: everything it needs is decoded.
        for _ in 0..16 {
            until(|| side.queued_frames() >= 256);
            side.callback(&mut out, 2, host(device));
            device += 256;
        }
        assert_eq!(playback.stats().starved_renders, 0, "{case}");
        playback
            .command(TransportCommand::Seek {
                time: ticks(target as f64 / device_rate as f64),
                mode: SeekMode::Exact,
            })
            .unwrap();
    }
    let bound = device_rate as u64 * pm_audio::playback::SOURCE_WAIT_MS / 1000;
    // Device frames since the play or seek command.
    let mut held = 0u64;
    out.resize(64 * 2, 0.0);
    // A seek's old chunks fill the ring until a callback retires them, as a
    // real device's next callback does, so the seek case keeps calling back
    // (that time counts toward the hold) until the new run's first render
    // is waiting. The hold began at or after the command and before `pre`.
    let t = std::time::Instant::now();
    while !(playback.stats().starved_renders > 0 && cache.blocked_decoders() > 0) {
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "{case}: the run never waited: {:?}, {} blocked decoders, {} queued",
            playback.stats(),
            cache.blocked_decoders(),
            side.queued_frames()
        );
        if seek {
            side.callback(&mut out, 2, host(device));
            held += 64;
            device += 64;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let pre = held;
    assert!(
        pre + 64 < bound,
        "{case}: {pre} frames before the run waited"
    );
    // Stop one callback before the bound: elapsed wall time cannot trigger
    // a drop. The decoder remains blocked throughout.
    while held + 64 < bound {
        side.callback(&mut out, 2, host(device));
        held += 64;
        device += 64;
        assert!(
            out.iter().all(|&v| v == 0.0),
            "{case}: held start plays nothing"
        );
    }
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(playback.stats().dropped_renders, 0, "{case}");
    assert_eq!(side.queued_frames(), 0, "{case}");
    assert!(playback.take_errors().is_empty(), "{case}");
    if exceed {
        // The hold ends between `bound` and `pre + bound`.
        while held < pre + bound {
            side.callback(&mut out, 2, host(device));
            held += 64;
            device += 64;
        }
        until(|| side.queued_frames() >= 512);
        assert!(
            cache.blocked_decoders() > 0,
            "{case}: playback continues with a stuck worker"
        );
        assert_eq!(
            playback.take_errors(),
            ["Audio decode exceeded 120 ms; missing voice plays as silence: late"],
            "{case}"
        );
    } else {
        cache.set_decode_blocked(&late_source, false);
        until(|| cache.is_ready(&late_source, target as i64, 4096));
        until(|| side.queued_frames() >= 512);
    }
    let mut played = Vec::new();
    let mut frame = 0usize;
    out.resize(512 * 2, 0.0);
    for turn in 0..20 {
        until(|| side.queued_frames() >= 512);
        side.callback(&mut out, 2, host(device));
        played.extend_from_slice(&out);
        if exceed && turn == 3 {
            // Consuming several buffers while stuck proves the clock and the
            // other voice do not wait for the decoder again.
            cache.set_decode_blocked(&late_source, false);
            until(|| cache.is_ready(&late_source, target as i64, 4096 * 4));
        }
        let clock = playback.clock().unwrap();
        let actual = clock
            .media_time_at(HostMicros::new(host(device) + LATENCY_US as u64).unwrap())
            .get();
        let expected = (target + frame) as i64 * TICKS_PER_SECOND / device_rate as i64;
        assert!(
            (actual - expected).abs() <= TICKS_PER_SECOND / 500_000,
            "{case}: device clock: {actual} vs {expected}"
        );
        frame += 512;
        device += 512;
    }
    let mut resumed = !exceed;
    let mut missing = 0u64;
    for (i, pair) in played.chunks_exact(2).enumerate() {
        let ready = if with_ready { good[target + i] } else { 0.0 };
        assert_eq!(pair[0], ready, "{case}: other voice at {i}");
        if pair[1] == 0.0 {
            assert!(
                !resumed,
                "{case}: no second missing span after re-entry at {i}"
            );
            missing += 1;
        } else {
            resumed = true;
            assert_eq!(
                pair[1],
                late[target + i],
                "{case}: sample-accurate re-entry at {i}"
            );
        }
    }
    assert!(resumed, "{case}");
    let stats = playback.stats();
    if exceed {
        assert!(missing >= 2_048, "{case}: {missing}");
        assert_eq!(stats.dropped_voice_frames, missing, "{case}");
        assert!(stats.dropped_renders > 0, "{case}");
    } else {
        assert_eq!(stats.dropped_renders, 0, "{case}");
        assert_eq!(missing, 0, "{case}: a briefly cold start is bit exact");
    }
    assert!(
        playback.take_errors().is_empty(),
        "{case}: reported only once after draining"
    );
    drop(playback);
    cache.shutdown();
}

/// A memoized decode error is met on every segment and the A3 drop message
/// on every episode; polling `take_errors` (the viewer's status does, every
/// 33 ms) hands each distinct message out once, not once per drain.
#[test]
fn recurring_errors_are_taken_once() {
    let rate = 48_000u32;
    let dir = temp_dir("recurring-errors");
    let (good_file, bad_file, late_file) = (
        dir.join("good.wav"),
        dir.join("bad.wav"),
        dir.join("late.wav"),
    );
    let n = rate as usize;
    let good: Vec<f32> = (0..n).map(|i| 0.1 + (i % 997) as f32 / 10_000.0).collect();
    write_wav(&good_file, rate, &[good.clone(), vec![0.0; n]]);
    write_wav(&bad_file, rate, &[vec![0.3; n], vec![0.3; n]]);
    write_wav(&late_file, rate, &[vec![0.0; n], vec![0.2; n]]);
    let cache = SourceCache::new(64 << 20, 2);
    let good_source = cache.open(&good_file, None).unwrap();
    cache.block(&good_source, 0, true).unwrap();
    let bad_source = cache.open(&bad_file, None).unwrap();
    cache.set_decode_failing(&bad_source, true);
    assert!(cache.block(&bad_source, 0, true).is_err());
    let late_source = cache.open(&late_file, None).unwrap();
    cache.set_decode_blocked(&late_source, true);
    let (playback, mut side) = Playback::start_headless(
        MixFormat::stereo(rate),
        cache.clone(),
        PlaybackConfig {
            preroll_ms: 0,
            preroll_wait_ms: 0,
            ..PlaybackConfig::default()
        },
        rate,
        2,
        LATENCY_US,
    )
    .unwrap();
    let doc = project(
        30.0,
        1.0,
        &[
            audio("good", "g", 0.0, 1.0, "", ""),
            audio("bad", "b", 0.0, 1.0, "", ""),
            audio("late", "s", 0.0, 1.0, "", ""),
        ],
        &[],
        "",
    );
    let resolver = |id: &str| {
        Some(match id {
            "g" => good_file.clone(),
            "b" => bad_file.clone(),
            _ => late_file.clone(),
        })
    };
    assert!(
        playback
            .set_project(Arc::new(doc), "C", &resolver)
            .unwrap()
            .is_empty()
    );
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    let mut taken = Vec::new();
    let mut out = vec![0.0; 256 * 2];
    let mut played = Vec::new();
    let host_start = host_now_us();
    let mut frame = 0u64;
    // Past the bound and well into the drop episode, draining every buffer.
    while frame < rate as u64 / 2 {
        side.callback(&mut out, 2, host_start + frame * 1_000_000 / rate as u64);
        if frame >= rate as u64 * pm_audio::playback::SOURCE_WAIT_MS / 1000 {
            played.extend_from_slice(&out);
        }
        frame += 256;
        taken.extend(playback.take_errors());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        playback.stats().dropped_renders > 1,
        "{:?}",
        playback.stats()
    );
    assert!(
        played.chunks_exact(2).any(|p| p[0] != 0.0),
        "the good voice plays on: playback is not frozen"
    );
    let decode: Vec<_> = taken.iter().filter(|e| e.starts_with("bad:")).collect();
    assert_eq!(decode.len(), 1, "{taken:?}");
    assert_eq!(
        decode[0], "bad: media: decode: block 0: injected decode failure",
        "a memoized failure reads as the first did"
    );
    let drops: Vec<_> = taken.iter().filter(|e| e.contains("120 ms")).collect();
    assert_eq!(drops.len(), 1, "{taken:?}");
    assert!(
        drops[0].ends_with(": late"),
        "each starved voice named once: {taken:?}"
    );
    assert_eq!(taken.len(), 2, "{taken:?}");
    cache.set_decode_blocked(&late_source, false);
    drop(playback);
    cache.shutdown();
}

/// A keeps dropping while B arrives late. B owns the reserve still in the
/// ring, so it can recover bit-exactly; A then recovers and starts a second
/// episode at its next source block, which reports again.
/// A is cold from the start, so the run first holds for the whole A3 bound
/// (start latency: nothing of the run is queued) and then drops only A.
#[test]
fn voices_own_their_waits_and_notifications() {
    let _load = stress_enabled().then(CpuContention::start);
    let rate = 48_000;
    let dir = temp_dir("voice-episodes");
    let files = [dir.join("good.wav"), dir.join("a.wav"), dir.join("b.wav")];
    let n = 3 * pm_audio::source::BLOCK;
    write_wav(&files[0], rate, &[vec![0.1; n], vec![0.0; n]]);
    write_wav(&files[1], rate, &[vec![0.0; n], vec![0.2; n]]);
    write_wav(&files[2], rate, &[vec![0.0; n], vec![0.3; n]]);
    let cache = SourceCache::new(64 << 20, 2);
    let good = cache.open(&files[0], None).unwrap();
    for block in 0..3 {
        cache.block(&good, block, true).unwrap();
    }
    let a = cache.open(&files[1], None).unwrap();
    let b = cache.open(&files[2], None).unwrap();
    cache.set_decode_blocked(&a, true);
    cache.set_decode_blocked(&b, true);
    let (playback, mut side) = Playback::start_headless(
        MixFormat::stereo(rate),
        cache.clone(),
        PlaybackConfig {
            preroll_ms: 0,
            preroll_wait_ms: 0,
            lookahead_ms: 0,
            ..PlaybackConfig::default()
        },
        rate,
        2,
        LATENCY_US,
    )
    .unwrap();
    let doc = project(
        30.0,
        4.0,
        &[
            audio("good", "g", 0.0, 4.0, "", ""),
            audio("A", "a", 0.0, 4.0, "", ""),
            audio("B", "b", 0.25, 0.5, "", ""),
        ],
        &[],
        "",
    );
    let resolver = |id: &str| {
        Some(
            files[match id {
                "g" => 0,
                "a" => 1,
                _ => 2,
            }]
            .clone(),
        )
    };
    playback.set_project(Arc::new(doc), "C", &resolver).unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    until(|| cache.blocked_decoders() > 0);
    // The start hold: callback time, not wall time, ends it at 120 ms.
    let bound = rate as u64 * pm_audio::playback::SOURCE_WAIT_MS / 1000;
    let mut out = vec![0.0; 512 * 2];
    let host = host_now_us();
    let mut held = 0u64;
    while held < bound {
        assert!(
            playback.take_errors().is_empty(),
            "dropped before the bound"
        );
        assert_eq!(playback.stats().dropped_renders, 0);
        side.callback(&mut out, 2, host + held * 1_000_000 / rate as u64);
        assert!(out.iter().all(|&v| v == 0.0), "nothing plays while held");
        held += 512;
        std::thread::sleep(Duration::from_secs_f64(512.0 / rate as f64));
    }
    let start_underruns = playback.stats().underruns;
    assert_eq!(start_underruns, held / 512);
    until(|| side.queued_frames() >= 3072);
    let mut messages = playback.take_errors();
    assert_eq!(
        messages,
        ["Audio decode exceeded 120 ms; missing voice plays as silence: A"]
    );
    let mut b_released = false;
    let mut a_released = false;
    let mut second_released = false;
    let mut second_at = None;
    let mut b_samples = 0;
    for turn in 0..170u64 {
        assert!(side.queued_frames() >= 512, "{:?}", playback.stats());
        side.callback(
            &mut out,
            2,
            host + (held + turn * 512) * 1_000_000 / rate as u64,
        );
        for (i, pair) in out.chunks_exact(2).enumerate() {
            assert_eq!(pair[0], 0.1, "ready voice never goes silent");
            let at = turn as usize * 512 + i;
            if (12_000..36_000).contains(&at) {
                // A has not yet been released while B sounds. Any lost B
                // sample would be 0, exposing a stolen per-voice budget.
                assert_eq!(pair[1], 0.3, "briefly late B at {at}");
                b_samples += 1;
            }
        }
        std::thread::sleep(Duration::from_secs_f64(512.0 / rate as f64));
        if !b_released && cache.blocked_decoders() == 2 {
            assert!(playback.take_errors().is_empty(), "B must get its own wait");
            cache.set_decode_blocked(&b, false);
            until(|| cache.is_ready(&b, 0, 256));
            b_released = true;
        }
        if b_released && !a_released && turn * 512 > 38_000 {
            cache.set_decode_blocked(&a, false);
            until(|| cache.is_ready(&a, 0, 256));
            // Block the next source block, after A's successful recovery.
            cache.set_decode_blocked(&a, true);
            a_released = true;
        }
        let new = playback.take_errors();
        if !new.is_empty() {
            assert!(
                a_released,
                "no extra message during the first episode: {new:?}"
            );
            second_at = Some(turn);
            messages.extend(new);
        }
        if second_at.is_some_and(|at| turn > at + 3) && !second_released {
            cache.set_decode_blocked(&a, false);
            until(|| cache.is_ready(&a, pm_audio::source::BLOCK as i64, 256));
            second_released = true;
        }
    }
    assert!(b_released && a_released && second_released);
    assert_eq!(b_samples, 24_000);
    assert_eq!(
        messages.len(),
        2,
        "one notification per A episode: {messages:?}"
    );
    // The second episode is mid-run with the good voice sounding, so the
    // ring's reserve ends it, and its own message says so.
    assert_eq!(
        messages[1],
        "Audio decode fell behind the output buffer; missing voice plays as silence so the others keep playing: A"
    );
    assert_eq!(
        playback.stats().underruns,
        start_underruns,
        "no underrun after the start hold"
    );
    drop(playback);
    cache.shutdown();
}

/// A6.1/2: once a run has started, even an all-missing mix commits tagged
/// silence before its ring empties. Large callbacks must first leave a real
/// per-voice wait: the initial fixed 160 ms ring could not do this at 8192.
#[test]
fn every_missing_voice_keeps_the_clock_running() {
    callback_decode_wait(256, false, false);
}

#[test]
fn large_callbacks_keep_per_voice_waits() {
    for frames in [4096, 8192] {
        callback_decode_wait(frames, true, true);
        callback_decode_wait(frames, true, false);
        callback_decode_wait(frames, false, false);
    }
}

fn callback_decode_wait(buffer: usize, with_ready: bool, recover_in_budget: bool) {
    let _load = stress_enabled().then(CpuContention::start);
    let dir = temp_dir(&format!("a6-{buffer}-{with_ready}-{recover_in_budget}"));
    let late_file = dir.join("late.wav");
    let good_file = dir.join("good.wav");
    let n = 4 * pm_audio::source::BLOCK;
    let late: Vec<f32> = (0..n).map(|i| 0.2 + (i % 991) as f32 / 10_000.0).collect();
    let good: Vec<f32> = (0..n).map(|i| 0.1 + (i % 997) as f32 / 10_000.0).collect();
    write_wav(&late_file, SR, &[vec![0.0; n], late.clone()]);
    write_wav(&good_file, SR, &[good.clone(), vec![0.0; n]]);
    let cache = SourceCache::new(64 << 20, 2);
    let late_source = cache.open(&late_file, None).unwrap();
    cache.block(&late_source, 0, true).unwrap();
    cache.set_decode_blocked(&late_source, true);
    let good_source = cache.open(&good_file, None).unwrap();
    for b in 0..4 {
        cache.block(&good_source, b, true).unwrap();
    }
    let (playback, mut side) = Playback::start_headless(
        MixFormat::stereo(SR),
        cache.clone(),
        PlaybackConfig {
            preroll_ms: 0,
            preroll_wait_ms: 0,
            lookahead_ms: 0,
            ..PlaybackConfig::default()
        },
        SR,
        2,
        LATENCY_US,
    )
    .unwrap();
    // The first callback tells the producer the device size while paused.
    let mut out = vec![0.0; buffer * 2];
    side.callback(&mut out, 2, host_now_us());
    let base_underruns = playback.stats().underruns;
    let dur = n as f64 / SR as f64;
    let mut clips = vec![audio("late", "s", 0.0, dur, "", "")];
    if with_ready {
        clips.push(audio("good", "g", 0.0, dur, "", ""));
    }
    let resolver = |id: &str| {
        Some(if id == "s" {
            late_file.clone()
        } else {
            good_file.clone()
        })
    };
    playback
        .set_project(
            Arc::new(project(30.0, dur, &clips, &[], "")),
            "C",
            &resolver,
        )
        .unwrap();
    let from = pm_audio::source::BLOCK - 3 * buffer - 8192;
    playback
        .command(TransportCommand::Seek {
            time: ticks(from as f64 / SR as f64),
            mode: SeekMode::Exact,
        })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    let target = (3 * buffer + 1920).max(3840) / 256 * 256;
    until(|| side.queued_frames() >= target);
    let host = host_now_us();
    let mut samples = Vec::new();
    let mut missed_at = None;
    let mut dropped_at = None;
    let mut released = false;
    let turns = (3 * buffer + 8192 + 24000).div_ceil(buffer);
    for turn in 0..turns {
        // This device does not wait for the source or for a full buffer.
        assert!(
            side.queued_frames() >= buffer,
            "{buffer}: ring empty: {:?}",
            playback.stats()
        );
        let at = host + turn as u64 * buffer as u64 * 1_000_000 / SR as u64;
        side.callback(&mut out, 2, at);
        samples.extend_from_slice(&out);
        let clock = playback.clock().unwrap();
        let heard = clock
            .media_time_at(HostMicros::new(at + LATENCY_US as u64).unwrap())
            .get();
        let want = (from + turn * buffer) as i64 * TICKS_PER_SECOND / SR as i64;
        assert!(
            (heard - want).abs() <= TICKS_PER_SECOND / 500_000,
            "{buffer}: clock {heard} vs {want}"
        );
        std::thread::sleep(Duration::from_secs_f64(buffer as f64 / SR as f64));
        if missed_at.is_none() && playback.stats().starved_renders > 0 {
            missed_at = Some(turn);
            assert_eq!(
                playback.stats().dropped_renders,
                0,
                "{buffer}: no per-voice wait budget"
            );
            if recover_in_budget {
                // Spending wall time without a callback cannot spend the
                // voice's device-time budget, even on an 8192-frame device.
                std::thread::sleep(Duration::from_millis(130));
                assert_eq!(playback.stats().dropped_renders, 0);
                cache.set_decode_blocked(&late_source, false);
                until(|| cache.is_ready(&late_source, pm_audio::source::BLOCK as i64, buffer));
                released = true;
            }
        }
        if playback.stats().dropped_renders > 0 {
            dropped_at.get_or_insert(turn);
            if !released && turn > missed_at.unwrap() + 1 {
                cache.set_decode_blocked(&late_source, false);
                until(|| cache.is_ready(&late_source, pm_audio::source::BLOCK as i64, buffer));
                released = true;
            }
        }
    }
    assert!(
        missed_at.is_some() && released,
        "{buffer}: miss/recovery not exercised"
    );
    let mut missing = 0;
    let mut resumed = false;
    for (i, pair) in samples.chunks_exact(2).enumerate() {
        assert_eq!(pair[0], if with_ready { good[from + i] } else { 0.0 });
        if pair[1] == 0.0 {
            assert!(!resumed);
            missing += 1;
        } else {
            resumed |= missing > 0;
            assert_eq!(
                pair[1],
                late[from + i],
                "{buffer}: sample-accurate re-entry at {i}"
            );
        }
    }
    assert_eq!(playback.stats().underruns - base_underruns, 0);
    if recover_in_budget {
        assert_eq!(missing, 0);
        assert!(playback.take_errors().is_empty());
    } else {
        assert!(missing > 0 && resumed);
        assert!(
            (dropped_at.unwrap() - missed_at.unwrap()) * buffer
                <= SR as usize * 120 / 1000 + buffer
        );
        assert_eq!(playback.take_errors().len(), 1);
    }
    drop(playback);
    cache.shutdown();
}

/// A6.3: both decoded (warm) and initially cold sources prepare every voice
/// while paused, including a distinct work-area start. Play and a paused
/// seek use those exact vocoder histories with no source wait or lost sample.
#[test]
fn paused_prefetch_starts_within_one_callback() {
    let _load = stress_enabled().then(CpuContention::start);
    let dir = temp_dir("a6-paused-start");
    let file = dir.join("tone.wav");
    write_wav(
        &file,
        SR,
        &[tone(SR, 12.0, 997.0, 0.2), tone(SR, 12.0, 631.0, 0.1)],
    );
    for buffer in [256, BUFFER] {
        for warm in [false, true] {
            let cache = SourceCache::new(64 << 20, 2);
            let source = cache.open(&file, None).unwrap();
            if warm {
                for b in 0..9 {
                    cache.block(&source, b, true).unwrap();
                }
            } else {
                cache.set_decode_delay(Duration::from_millis(8));
            }
            let (playback, mut side) = Playback::start_headless(
                MixFormat::stereo(SR),
                cache.clone(),
                PlaybackConfig::default(),
                SR,
                2,
                LATENCY_US,
            )
            .unwrap();
            let doc = Arc::new(project(
                30.0,
                8.0,
                &[
                    audio("stretched", "s", 0.0, 8.0, r#", "speed":1.5"#, ""),
                    audio("plain", "s", 0.0, 8.0, "", ""),
                ],
                &[],
                "",
            ));
            playback
                .set_project(doc.clone(), "C", &|_: &str| Some(file.clone()))
                .unwrap();
            playback
                .command(TransportCommand::SetLoop {
                    range: Some(TickRange {
                        start: ticks(4.0),
                        end: ticks(6.0),
                    }),
                })
                .unwrap();
            let mut out = vec![0.0; buffer * 2];
            // Preparation of the work start happens without seeking there.
            until(|| playback.prepared_starts().contains(&(4 * SR as i64)));
            for target in [0, 4 * SR as i64, 2 * SR as i64] {
                playback.command(TransportCommand::Pause).unwrap();
                // Retire previous epoch's queued chunks just as a live device
                // does continuously while paused.
                side.callback(&mut out, 2, host_now_us());
                playback
                    .command(TransportCommand::Seek {
                        time: ticks(target as f64 / SR as f64),
                        mode: SeekMode::Exact,
                    })
                    .unwrap();
                until(|| playback.prepared_starts().contains(&target));
                let before = playback.stats();
                let start = std::time::Instant::now();
                playback
                    .command(TransportCommand::Play {
                        rate: PlaybackRate::NORMAL,
                    })
                    .unwrap();
                let deadline = start + Duration::from_secs_f64(buffer as f64 / SR as f64);
                while side.queued_frames() < buffer && std::time::Instant::now() < deadline {
                    std::thread::yield_now();
                }
                assert!(
                    side.queued_frames() >= buffer,
                    "buffer={buffer} warm={warm} target={target}: first callback missed, {:?}",
                    start.elapsed()
                );
                let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
                side.callback(&mut out, 2, host_now_us());
                let rig = Rig::new(&[("s", file.as_path())]);
                let mut mixer = rig.mixer((*doc).clone(), "C", MixFormat::stereo(SR));
                let mut expected = vec![Vec::new(); 2];
                mixer
                    .render(
                        target,
                        buffer,
                        &mut expected,
                        &pm_audio::RenderOptions {
                            wait: true,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                for i in 0..buffer {
                    for c in 0..2 {
                        assert_eq!(out[2 * i + c], expected[c][i]);
                    }
                }
                assert_eq!(playback.stats().starved_renders, before.starved_renders);
                assert_eq!(playback.stats().dropped_renders, before.dropped_renders);
                eprintln!(
                    "A6 start buffer={buffer} warm={warm} target={target}: {latency_ms:.3} ms (one callback {:.3} ms)",
                    buffer as f64 * 1000.0 / SR as f64
                );
            }
            drop(playback);
            cache.shutdown();
        }
    }
}

/// A6 review: paused preparation renders every voice for up to three starts.
/// Right after an edit (which clears prepared starts) with many stretched
/// voices, a Play arriving mid-pass must not wait for the remaining starts.
#[test]
fn play_preempts_paused_preparation_after_an_edit() {
    let _load = stress_enabled().then(CpuContention::start);
    let dir = temp_dir("a6-preempt");
    let file = dir.join("tone.wav");
    write_wav(
        &file,
        SR,
        &[tone(SR, 12.0, 997.0, 0.2), tone(SR, 12.0, 631.0, 0.1)],
    );
    let cache = SourceCache::new(128 << 20, 2);
    let source = cache.open(&file, None).unwrap();
    for b in 0..12 {
        cache.block(&source, b, true).unwrap();
    }
    let (playback, mut side) = Playback::start_headless(
        MixFormat::stereo(SR),
        cache.clone(),
        PlaybackConfig::default(),
        SR,
        2,
        LATENCY_US,
    )
    .unwrap();
    let voices = 16;
    let doc = |gain: f64| {
        let layers: Vec<String> = (0..voices)
            .map(|i| {
                audio(
                    &format!("v{i}"),
                    "s",
                    0.0,
                    8.0,
                    &format!(r#", "speed":{}, "gain":{gain}"#, 1.25 + i as f64 * 0.05),
                    "",
                )
            })
            .collect();
        Arc::new(project(30.0, 8.0, &layers, &[], ""))
    };
    let path = |_: &str| Some(file.clone());
    playback.set_project(doc(1.0), "C", &path).unwrap();
    playback
        .command(TransportCommand::SetLoop {
            range: Some(TickRange {
                start: ticks(4.0),
                end: ticks(6.0),
            }),
        })
        .unwrap();
    // A whole pass: the playhead and both loop edges, every voice fresh.
    let all = [
        0,
        4 * SR as i64,
        6 * SR as i64 - pm_audio::ring::CHUNK as i64,
    ];
    playback.set_project(doc(0.9), "C", &path).unwrap();
    let pass = std::time::Instant::now();
    until(|| playback.prepared_starts().contains(&0));
    let playhead_ms = pass.elapsed().as_secs_f64() * 1000.0;
    until(|| all.iter().all(|s| playback.prepared_starts().contains(s)));
    let pass_ms = pass.elapsed().as_secs_f64() * 1000.0;
    let mut out = vec![0.0; 512 * 2];
    let (mut acks, mut firsts) = (vec![], vec![]);
    // Land Play at every phase of the restarted pass (2 ms steps).
    for (trial, delay_ms) in (0..48u64).map(|i| i * 2).enumerate() {
        playback.command(TransportCommand::Pause).unwrap();
        side.callback(&mut out, 2, host_now_us());
        // An edit invalidates every prepared start; preparation restarts.
        let gain = if trial % 2 == 0 { 1.0 } else { 0.9 };
        playback.set_project(doc(gain), "C", &path).unwrap();
        std::thread::sleep(Duration::from_millis(delay_ms));
        let at = std::time::Instant::now();
        playback
            .command(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            })
            .unwrap();
        acks.push(at.elapsed().as_secs_f64() * 1000.0);
        // First device buffer queued (prepared or fresh pre-roll).
        until(|| side.queued_frames() >= 512);
        firsts.push(at.elapsed().as_secs_f64() * 1000.0);
    }
    firsts.sort_by(f64::total_cmp);
    let worst = acks.iter().copied().fold(0.0, f64::max);
    let mut sorted = acks.clone();
    sorted.sort_by(f64::total_cmp);
    let pct = |q: f64| sorted[((sorted.len() as f64 * q).ceil() as usize).max(1) - 1];
    eprintln!(
        "A6 preempt: {voices} stretched voices, playhead prepared {playhead_ms:.1} ms, all starts {pass_ms:.1} ms after an edit; Play acknowledged p50/p95/max {:.2}/{:.2}/{worst:.2} ms; first 512-frame buffer queued p50/p95/max {:.2}/{:.2}/{:.2} ms over {} trials",
        pct(0.5),
        pct(0.95),
        firsts[firsts.len() / 2],
        firsts[(firsts.len() * 95).div_ceil(100) - 1],
        firsts[firsts.len() - 1],
        acks.len()
    );
    // The pass is spread over idle turns; Play must never wait for a
    // pass's remaining starts, only (at most) the one being rendered.
    assert!(
        worst < pass_ms / 3.0,
        "Play waited {worst:.1} ms of a {pass_ms:.1} ms preparation pass: {acks:.2?}"
    );
    drop(playback);
    cache.shutdown();
}

/// A8: a paused level edit followed by Play in the same control batch
/// preserves every warmed voice, and the result is the edited offline PCM.
#[test]
fn edit_then_play_reuses_all_sixteen_stretched_voices() {
    assert_sixteen_voice_edit_starts(false, false);
}

#[test]
fn cold_seek_and_speed_edit_prepare_sixteen_voices_in_parallel() {
    assert_sixteen_voice_edit_starts(true, false);
}

/// The same starts with sixteen different signals (each voice trimmed
/// 61.3 ms further): no voice can reuse another's stretched head, so every
/// voice is prepared by its own vocoder job and must still start complete.
#[test]
fn edit_then_play_reuses_sixteen_distinct_stretched_voices() {
    assert_sixteen_voice_edit_starts(false, true);
}

#[test]
fn cold_starts_of_sixteen_distinct_voices_are_complete_and_simultaneous() {
    assert_sixteen_voice_edit_starts(true, true);
}

/// The sixteen-voice start tests fill the process-wide preparation pool with
/// user-interactive jobs. Run concurrently, each would time the other's
/// sixteen jobs, and a burst would compete with another test's real-time
/// producer in the same process (an app never prepares paused starts during
/// its own playback). They take this lock exclusively; the real-time
/// time-mapped comparisons share it; every other test runs beside both.
static POOL_BURSTS: std::sync::RwLock<()> = std::sync::RwLock::new(());

fn assert_sixteen_voice_edit_starts(cold: bool, distinct: bool) {
    let _serial = POOL_BURSTS.write().unwrap_or_else(|e| e.into_inner());
    let _load = stress_enabled().then(CpuContention::start);
    let dir = temp_dir(match (cold, distinct) {
        (true, false) => "a8-cold-start",
        (false, false) => "a8-level-start",
        (true, true) => "a8-cold-distinct",
        (false, true) => "a8-level-distinct",
    });
    let trim = |i: usize| if distinct { i as f64 * 0.0613 } else { 0.0 };
    let file = dir.join("tone.wav");
    write_wav(
        &file,
        SR,
        &[tone(SR, 12.0, 997.0, 0.01), tone(SR, 12.0, 631.0, 0.005)],
    );
    let cache = SourceCache::new(128 << 20, 2);
    let source = cache.open(&file, None).unwrap();
    for b in 0..9 {
        cache.block(&source, b, true).unwrap();
    }
    let (playback, mut side) = Playback::start_headless(
        MixFormat::stereo(SR),
        cache.clone(),
        PlaybackConfig::default(),
        SR,
        2,
        LATENCY_US,
    )
    .unwrap();
    let doc = |gain, speed| {
        let layers: Vec<String> = (0..16)
            .map(|i| {
                audio(
                    &format!("v{i}"),
                    "s",
                    0.0,
                    8.0,
                    &format!(r#", "speed":{speed}, "gain":{gain}, "trim":{}"#, trim(i)),
                    "",
                )
            })
            .collect();
        Arc::new(project(30.0, 8.0, &layers, &[], ""))
    };
    let path = |_: &str| Some(file.clone());
    playback.set_project(doc(1.0, 1.5), "C", &path).unwrap();
    let mut out = vec![0.0; 512 * 2];
    for (trial, gain) in [0.3, 0.7, 0.2, 1.0].into_iter().enumerate() {
        let target = if cold {
            if trial % 2 == 0 { 2 } else { 3 }
        } else {
            4
        };
        playback.command(TransportCommand::Pause).unwrap();
        side.callback(&mut out, 2, host_now_us());
        playback
            .command(TransportCommand::Seek {
                time: ticks(target as f64),
                mode: SeekMode::Exact,
            })
            .unwrap();
        if !cold {
            until(|| playback.prepared_starts().contains(&(target * SR as i64)));
        }
        let edited = doc(gain, if cold && trial % 2 == 0 { 1.6 } else { 1.5 });
        playback.set_project(edited.clone(), "C", &path).unwrap();
        let before = playback.stats();
        let at = std::time::Instant::now();
        playback
            .commands([
                TransportCommand::SetLoop { range: None },
                TransportCommand::Seek {
                    time: ticks(target as f64),
                    mode: SeekMode::Exact,
                },
                TransportCommand::Play {
                    rate: PlaybackRate::NORMAL,
                },
            ])
            .unwrap();
        let bound = Duration::from_secs_f64(512.0 / SR as f64);
        // The deadline is checked against the producer's own publication
        // time, not against when this thread happened to observe the ring:
        // an observer descheduled under load must not hide a late queue,
        // nor fail an on-time one.
        while side.queued_frames() < 512 && at.elapsed() < Duration::from_secs(5) {
            std::thread::yield_now();
        }
        let observed = at.elapsed();
        assert!(
            side.queued_frames() >= 512,
            "edited first buffer: {observed:?}"
        );
        side.callback(&mut out, 2, host_now_us());
        let timing = playback.start_timing().unwrap();
        let &(frames, published_us) = timing
            .publications
            .iter()
            .find(|(frames, _)| *frames >= 512)
            .expect("first 512 frames published");
        let published = Duration::from_micros(published_us.saturating_sub(timing.requested_us));
        let ms = |from: u64, to: u64| to.saturating_sub(from) as f64 / 1000.0;
        eprintln!(
            "A8 sixteen-voice edit cold={cold} distinct={distinct} first buffer: published {:.3} ms ({frames} frames), observed {:.3} ms; request to applied {:.3}, applied to ready {:.3}, ready to published {:.3}",
            published.as_secs_f64() * 1000.0,
            observed.as_secs_f64() * 1000.0,
            ms(timing.requested_us, timing.applied_us),
            ms(timing.applied_us, timing.ready_us),
            ms(timing.ready_us, published_us)
        );
        // A prepared start needs no vocoder work after Play: it must publish
        // within one 512-frame callback by the producer's own clock. A cold
        // start's publication time is its CPU preparation (the canonical
        // pre-roll on the shared pool), which a niceness-15 test process
        // cannot guarantee beside other foreground load. It is reported
        // here; its latency gate is the paired app capture against today.
        assert!(
            cold || published <= bound,
            "first 512 frames published {published:?} after Play (bound {bound:?}, cold={cold}, distinct={distinct})"
        );
        // Distinct signals cannot share an immutable head; identical cold
        // starts do (the representative job's prefix serves all sixteen).
        if distinct {
            assert_eq!(
                timing.prefill_ready_us, 0,
                "distinct voices used a shared head"
            );
        }
        let rig = Rig::new(&[("s", file.as_path())]);
        let mut reference = rig.mixer((*edited).clone(), "C", MixFormat::stereo(SR));
        let mut expected = vec![vec![]; 2];
        reference
            .render(
                target * SR as i64,
                512,
                &mut expected,
                &pm_audio::RenderOptions {
                    wait: true,
                    ..Default::default()
                },
            )
            .unwrap();
        if cold {
            let mut captured = out.clone();
            for _ in 0..15 {
                until(|| side.queued_frames() >= 512);
                side.callback(&mut out, 2, host_now_us());
                captured.extend_from_slice(&out);
            }
            let timing = playback.start_timing().unwrap();
            if !distinct {
                assert_ne!(timing.prefill_ready_us, 0, "identical voices share a head");
            }
            assert_eq!(
                timing.omitted_prefix_frames, 0,
                "ordinary cold start must be complete"
            );
            assert_eq!(timing.start_spread_frames, 0, "every voice starts together");
            assert_eq!(timing.safety_net_fires, 0, "A8 must not use A3");
            assert_eq!(
                timing.program_underruns, 0,
                "complete PCM must reach the device without gaps"
            );
            assert_eq!(timing.voice_start_samples.len(), 16);
            let gaps = timing.late_voice_frames;
            assert!(gaps.is_empty(), "ordinary cold start omitted PCM: {gaps:?}");
            let max_gap = gaps.iter().map(|(_, n)| *n).max().unwrap_or(0) as usize;
            eprintln!(
                "A8 CPU preparation late spans: {gaps:?}, longest {:.3} ms",
                max_gap as f64 / 48.0
            );
            assert!(
                max_gap < 8192,
                "every voice must join within the captured span"
            );
            // Exact full mix from the first sample, with no omitted voice:
            // each voice rendered alone, offline, then summed in voice order.
            let pcm: Vec<Vec<Vec<f32>>> = (0..16)
                .map(|voice| {
                    if !distinct && voice > 0 {
                        return vec![];
                    }
                    let one = project(
                        30.0,
                        8.0,
                        &[audio(
                            &format!("v{voice}"),
                            "s",
                            0.0,
                            8.0,
                            &format!(
                                r#", "speed":{}, "gain":{gain}, "trim":{}"#,
                                if trial % 2 == 0 { 1.6 } else { 1.5 },
                                trim(voice)
                            ),
                            "",
                        )],
                        &[],
                        "",
                    );
                    let mut single = rig.mixer(one, "C", MixFormat::stereo(SR));
                    let mut pcm = vec![vec![]; 2];
                    single
                        .render(
                            target * SR as i64,
                            8192,
                            &mut pcm,
                            &pm_audio::RenderOptions {
                                wait: true,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    pcm
                })
                .collect();
            if distinct {
                assert!(
                    (0..8192).any(|i| pcm[0][0][i] != pcm[1][0][i]),
                    "the trims must give different signals"
                );
            }
            for i in 0..8192 {
                for c in 0..2 {
                    let mut sum = 0.0f32;
                    for voice in 0..16 {
                        let id = format!("v{voice}");
                        let gap = gaps
                            .iter()
                            .find(|(v, _)| *v == id)
                            .map_or(0, |(_, n)| *n as usize);
                        if i >= gap {
                            sum += pcm[if distinct { voice } else { 0 }][c][i];
                        }
                    }
                    assert_eq!(
                        captured[2 * i + c],
                        sum,
                        "cold trial {trial}, sample {i}, channel {c}, gaps {gaps:?}"
                    );
                }
            }
        } else {
            for i in 0..512 {
                for c in 0..2 {
                    assert_eq!(out[2 * i + c], expected[c][i]);
                }
            }
            assert_eq!(playback.stats().starved_renders, before.starved_renders);
            assert_eq!(playback.stats().dropped_renders, before.dropped_renders);
        }
    }
    drop(playback);
    cache.shutdown();
}

/// A first callback is submitted before its presentation time. Pausing in
/// that interval must keep the seek target, including a loop's first frame.
#[test]
fn pausing_before_the_first_buffer_is_presented_keeps_the_prepared_target() {
    let dir = temp_dir("a8-first-presentation");
    let file = ramp_file(&dir, 8);
    for looping in [false, true] {
        let (playback, mut dev, _rig) = setup(&file, 8.0, SR, 0.0);
        if looping {
            playback
                .command(TransportCommand::SetLoop {
                    range: Some(TickRange {
                        start: ticks(4.0),
                        end: ticks(6.0),
                    }),
                })
                .unwrap();
        }
        playback
            .command(TransportCommand::Seek {
                time: ticks(4.0),
                mode: SeekMode::Exact,
            })
            .unwrap();
        until(|| playback.prepared_starts().contains(&(4 * SR as i64)));
        playback
            .command(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            })
            .unwrap();
        until(|| dev.side.queued_frames() >= BUFFER);
        dev.side.callback(&mut dev.out, 2, host_now_us() + 100_000);
        assert_eq!(playback.clock().unwrap().state, TransportState::Playing);
        playback.command(TransportCommand::Pause).unwrap();
        assert_eq!(playback.clock().unwrap().media_time, ticks(4.0));
        until(|| playback.prepared_starts().contains(&(4 * SR as i64)));
        playback
            .command(TransportCommand::Seek {
                time: ticks(4.005),
                mode: SeekMode::Exact,
            })
            .unwrap();
        // A raw seek still prepares the displayed frame as well as its
        // sample-exact target; the app's next normal Play uses that frame.
        until(|| playback.prepared_starts().contains(&(4 * SR as i64)));
        until(|| playback.prepared_starts().contains(&(4 * SR as i64 + 240)));
    }
}

/// Prepared loop starts apply at a wrap. Straight entry from before the
/// work area must keep the running vocoder's phase history.
#[test]
fn entering_the_loop_keeps_the_uninterrupted_stretch_history() {
    let dir = temp_dir("a7-loop-entry");
    let file = dir.join("tone.wav");
    write_wav(
        &file,
        SR,
        &[tone(SR, 4.0, 997.0, 0.2), tone(SR, 4.0, 631.0, 0.1)],
    );
    let doc = || {
        project(
            30.0,
            3.0,
            &[audio("A", "s", 0.0, 3.0, r#", "speed":1.5"#, "")],
            &[],
            "",
        )
    };
    let (playback, mut dev, rig) = setup_doc(doc(), &[("s", &file)], SR, 2, 0.0);
    playback
        .command(TransportCommand::SetLoop {
            range: Some(TickRange {
                start: ticks(1.0),
                end: ticks(1.5),
            }),
        })
        .unwrap();
    playback
        .command(TransportCommand::Seek {
            time: ticks(0.5),
            mode: SeekMode::Exact,
        })
        .unwrap();
    until(|| playback.prepared_starts().contains(&(SR as i64)));
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    dev.run(70);
    let stats = playback.stats();
    drop(playback);
    let mut reference = rig.mixer(doc(), "C", MixFormat::stereo(SR));
    let expected = rig.render(&mut reference, SR as i64 / 2, SR as i64 / 2 + 32_000, 4096);
    let played: Vec<Vec<f32>> = (0..2)
        .map(|c| {
            dev.log
                .iter()
                .flat_map(|(_, b)| b.iter().skip(c).step_by(2).copied())
                .collect()
        })
        .collect();
    let start = played[0].iter().position(|v| *v != 0.0).unwrap();
    let lead = expected[0].iter().position(|v| *v != 0.0).unwrap();
    for c in 0..2 {
        assert_eq!(
            &played[c][start - lead..start - lead + 32_000],
            &expected[c]
        );
    }
    assert_eq!(stats.underruns, 0);
    assert_eq!(stats.starved_renders, 0);
}

/// CPU priming must never use the missing-decoder safety net. A genuinely
/// stuck stretched input keeps all voices complete until 120 ms of device
/// time, regardless of wall time; only then may its own prefix be dropped.
#[test]
fn stretched_starts_are_complete_until_the_decoder_device_bound() {
    let _load = stress_enabled().then(CpuContention::start);
    for exceed in [false, true] {
        let dir = temp_dir(&format!("a8-stretched-bound-{exceed}"));
        let good_file = dir.join("good.wav");
        let late_file = dir.join("late.wav");
        let signal = tone(SR, 8.0, 719.0, 0.2);
        write_wav(&good_file, SR, &[signal.clone(), vec![0.0; signal.len()]]);
        write_wav(&late_file, SR, &[vec![0.0; signal.len()], signal]);
        let cache = SourceCache::new(128 << 20, 2);
        let good = cache.open(&good_file, None).unwrap();
        for b in 0..6 {
            cache.block(&good, b, true).unwrap();
        }
        let late = cache.open(&late_file, None).unwrap();
        cache.set_decode_blocked(&late, true);
        let (playback, mut side) = Playback::start_headless(
            MixFormat::stereo(SR),
            cache.clone(),
            PlaybackConfig::default(),
            SR,
            2,
            LATENCY_US,
        )
        .unwrap();
        playback
            .command(TransportCommand::Seek {
                time: ticks(2.0),
                mode: SeekMode::Exact,
            })
            .unwrap();
        let doc = project(
            30.0,
            8.0,
            &[
                audio("good", "g", 0.0, 8.0, r#", "speed":1.5"#, ""),
                audio("late0", "s", 0.0, 8.0, r#", "speed":1.5, "gain":0.5"#, ""),
                audio("late1", "s", 0.0, 8.0, r#", "speed":1.5, "gain":0.5"#, ""),
            ],
            &[],
            "",
        );
        let path = |id: &str| {
            Some(if id == "g" {
                good_file.clone()
            } else {
                late_file.clone()
            })
        };
        playback
            .set_project(Arc::new(doc.clone()), "C", &path)
            .unwrap();
        playback
            .command(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            })
            .unwrap();
        until(|| cache.blocked_decoders() > 0);
        let mut out = vec![0.0; 256 * 2];
        // Three quarters of the bound, then a long wall pause: neither may
        // release even the already primed voice.
        for _ in 0..16 {
            side.callback(&mut out, 2, host_now_us());
        }
        std::thread::sleep(Duration::from_millis(130));
        assert_eq!(side.queued_frames(), 0);
        assert_eq!(playback.stats().dropped_renders, 0);
        if exceed {
            for _ in 0..32 {
                side.callback(&mut out, 2, host_now_us());
            }
            until(|| side.queued_frames() >= 512);
        } else {
            cache.set_decode_blocked(&late, false);
            until(|| side.queued_frames() >= 512);
        }
        out.resize(512 * 2, 0.0);
        side.callback(&mut out, 2, host_now_us());
        let timing = playback.start_timing().unwrap();
        assert_eq!(timing.safety_net_fires, u64::from(exceed));
        assert_eq!(timing.omitted_prefix_frames > 0, exceed);
        let rig = Rig::new(&[("g", good_file.as_path()), ("s", late_file.as_path())]);
        let mut reference = rig.mixer(doc, "C", MixFormat::stereo(SR));
        let mut expected = vec![vec![]; 2];
        reference
            .render(
                2 * SR as i64,
                512,
                &mut expected,
                &pm_audio::RenderOptions {
                    wait: true,
                    ..Default::default()
                },
            )
            .unwrap();
        for i in 0..512 {
            assert_eq!(out[2 * i], expected[0][i], "ready stretched voice {i}");
            assert_eq!(
                out[2 * i + 1],
                if exceed { 0.0 } else { expected[1][i] },
                "late stretched voice {i}"
            );
        }
        if !exceed {
            assert_eq!(timing.start_spread_frames, 0);
            assert_eq!(timing.voice_start_samples.len(), 3);
        }
        cache.set_decode_blocked(&late, false);
        if exceed {
            let mut captured = out.clone();
            for _ in 0..31 {
                until(|| side.queued_frames() >= 512);
                side.callback(&mut out, 2, host_now_us());
                captured.extend_from_slice(&out);
            }
            let timing = playback.start_timing().unwrap();
            assert_eq!(
                timing.voice_start_samples.len(),
                3,
                "both gated voices must recover"
            );
            let mut reference = rig.mixer(
                project(
                    30.0,
                    8.0,
                    &[
                        audio("good", "g", 0.0, 8.0, r#", "speed":1.5"#, ""),
                        audio("late", "s", 0.0, 8.0, r#", "speed":1.5"#, ""),
                    ],
                    &[],
                    "",
                ),
                "C",
                MixFormat::stereo(SR),
            );
            reference
                .render(
                    2 * SR as i64,
                    16384,
                    &mut expected,
                    &pm_audio::RenderOptions {
                        wait: true,
                        ..Default::default()
                    },
                )
                .unwrap();
            for i in 0..16384 {
                let mut right = 0.0f32;
                for id in ["late0", "late1"] {
                    let omitted = timing
                        .late_voice_frames
                        .iter()
                        .find(|(v, _)| v == id)
                        .unwrap()
                        .1 as usize;
                    if i >= omitted {
                        right += expected[1][i] * 0.5;
                    }
                }
                assert_eq!(captured[2 * i], expected[0][i], "ready voice recovery {i}");
                assert_eq!(
                    captured[2 * i + 1],
                    right,
                    "gated voice recovery {i}, {:?}",
                    timing.late_voice_frames
                );
            }
        }
        drop(playback);
        cache.shutdown();
    }
}
