//! The native audio path's own CPU cost on the real output device, silently
//! (the source is a -120 dBFS tone): paused with the device open and every
//! start prepared, then playing. Reports process CPU (getrusage) and context
//! switches per second, the callback rate, CoreAudio overloads and ring
//! underruns while playing.
//!
//!   cargo run --release -p pm-audio --example paused_cost [voices] [seconds] [speed] [loop]
//!
//! `speed` other than 1 time-stretches every voice (the vocoder path).
//! `loop` 0 plays straight through instead of looping [4 s, 6 s).

use std::sync::Arc;
use std::time::{Duration, Instant};

use pm_audio::offline::{PcmFormat, WavWriter};
use pm_audio::playback::{Playback, PlaybackConfig};
use pm_audio::{MixFormat, SourceCache};
use pm_model::Document;
use pm_model::json::parse;
use pm_protocol::clock::{SeekMode, TransportCommand};
use pm_protocol::common::{PlaybackRate, TICKS_PER_SECOND, TickRange, Ticks};

const RATE: u32 = 48_000;

struct DecodeWorkers(Arc<SourceCache>);

impl Drop for DecodeWorkers {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

struct Usage {
    cpu_s: f64,
    switches: i64,
}

fn usage() -> Usage {
    let mut r: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `r` is a valid, writable rusage for this call.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut r) };
    let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
    Usage {
        cpu_s: t(r.ru_utime) + t(r.ru_stime),
        switches: r.ru_nvcsw + r.ru_nivcsw,
    }
}

fn ticks(s: f64) -> Ticks {
    Ticks::new((s * TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let voices: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let speed: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let looping = args.next().is_none_or(|s| s != "0");

    let dir = tempfile::Builder::new()
        .prefix("pm-audio-paused-cost-")
        .tempdir()
        .unwrap();
    let file = dir.path().join("tone.wav");
    let frames = RATE as usize * 12;
    let mut pcm = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let x = (1e-6 * (std::f64::consts::TAU * 997.0 * i as f64 / RATE as f64).sin()) as f32;
        pcm.extend([x, x]);
    }
    let mut w = WavWriter::new(
        std::io::BufWriter::new(std::fs::File::create(&file).unwrap()),
        PcmFormat::F32,
        2,
        RATE,
    )
    .unwrap();
    w.write(0, &pcm).unwrap();
    w.finish().unwrap();

    let speed_field = if speed == 1.0 {
        String::new()
    } else {
        format!(r#","speed":{speed}"#)
    };
    let layers: Vec<String> = (0..voices)
        .map(|i| {
            format!(
                r#"{{"id":"v{i}","type":"audio","name":"v{i}","from":0,"dur":10,"on":true,"solo":false,"group":null,"parent":null,"p":{{}},"fx":[],"masks":[],"d":{{"asset":"s","gain":1,"trim":0,"fadeIn":0,"fadeOut":0{speed_field}}}}}"#
            )
        })
        .collect();
    let doc = Document::new(
        Some(
            parse(&format!(
                r#"{{"id":"P","compId":"C","w":16,"h":16,"fps":30,"dur":10,"layers":[{}],"comps":{{}},"assets":{{}},"params":{{}}}}"#,
                layers.join(",")
            ))
            .unwrap(),
        ),
        1,
    );
    // The presenter's budget and decode workers.
    let cache = SourceCache::new(256 << 20, 2);
    let _decode_workers = DecodeWorkers(cache.clone());
    let mut playback = Playback::start(MixFormat::stereo(RATE), cache, PlaybackConfig::default())
        .expect("open the output device");
    let path = |_: &str| Some(file.clone());
    playback.set_project(Arc::new(doc), "C", &path).unwrap();
    playback
        .command(TransportCommand::SetLoop {
            range: looping.then(|| TickRange {
                start: ticks(4.0),
                end: ticks(6.0),
            }),
        })
        .unwrap();
    // A first play opens the path as the app's first Play does.
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    playback.command(TransportCommand::Pause).unwrap();
    playback
        .command(TransportCommand::Seek {
            time: Ticks::ZERO,
            mode: SeekMode::Exact,
        })
        .unwrap();
    let settle = Instant::now();
    let starts = if looping { 3 } else { 1 };
    while playback.prepared_starts().len() < starts && settle.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let prepared_after = settle.elapsed();
    std::thread::sleep(Duration::from_secs(1));
    let callbacks = |p: &Playback| p.stream().map_or(0, |s| s.callbacks());

    let window = |label: &str, p: &Playback| {
        let (u0, s0, c0) = (usage(), p.stats(), callbacks(p));
        let t0 = Instant::now();
        std::thread::sleep(Duration::from_secs_f64(seconds));
        let elapsed = t0.elapsed().as_secs_f64();
        let (u1, s1, c1) = (usage(), p.stats(), callbacks(p));
        println!(
            "{label}: process CPU {:.2}%, context switches {:.0}/s, callbacks {:.1}/s, overloads {}, ring underruns {}, starved renders {}",
            100.0 * (u1.cpu_s - u0.cpu_s) / elapsed,
            (u1.switches - u0.switches) as f64 / elapsed,
            (c1 - c0) as f64 / elapsed,
            s1.device_overloads - s0.device_overloads,
            s1.underruns - s0.underruns,
            s1.starved_renders - s0.starved_renders,
        );
    };
    println!(
        "device {:?}; {voices} voice(s) at speed {speed}, loop {looping}; {starts} start(s) prepared after {:.0} ms",
        playback
            .device()
            .map(|d| (&d.name, d.sample_rate, d.buffer_frames)),
        prepared_after.as_secs_f64() * 1000.0
    );
    window("paused", &playback);
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    window("playing", &playback);
    playback.command(TransportCommand::Pause).unwrap();
    playback.shutdown();
}
