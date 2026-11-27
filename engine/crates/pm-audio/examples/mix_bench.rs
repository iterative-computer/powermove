//! Mixing cost: renders a composition of many voices offline and reports
//! how many times faster than real time it mixes, per voice kind.
//!
//!   cargo run --release -p pm-audio --example mix_bench [seconds]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use pm_audio::offline::{self, PcmFormat, WavWriter};
use pm_audio::{MixFormat, MixGraph, Mixer, SourceCache};
use pm_eval::Evaluator;
use pm_model::Document;
use pm_model::json::parse;

fn wav(path: &Path, rate: u32, seconds: f64, hz: f64) {
    let n = (rate as f64 * seconds) as usize;
    let mut inter = Vec::with_capacity(n * 2);
    for i in 0..n {
        let v = (0.2 * (std::f64::consts::TAU * hz * i as f64 / rate as f64).sin()) as f32;
        inter.push(v);
        inter.push(-v);
    }
    let mut w = WavWriter::new(
        std::io::BufWriter::new(std::fs::File::create(path).unwrap()),
        PcmFormat::F32,
        2,
        rate,
    )
    .unwrap();
    w.write(0, &inter).unwrap();
    w.finish().unwrap();
}

fn keys(points: &[(f64, f64)]) -> String {
    let kf: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(i, (t, v))| {
            format!(r#"{{"t":{t},"v":{v},"i":"k{i}","inInterp":"linear","outInterp":"linear"}}"#)
        })
        .collect();
    format!(
        r#"{{"v":{},"kf":[{}],"expr":null}}"#,
        points[0].1,
        kf.join(",")
    )
}

fn run(label: &str, layers: Vec<String>, seconds: f64, media: &[(String, PathBuf)]) {
    let text = format!(
        r#"{{"id":"P","compId":"C","w":16,"h":16,"fps":30,"dur":{seconds},"layers":[{}],"comps":{{}},"assets":{{}},"params":{{}}}}"#,
        layers.join(",")
    );
    let doc = Document::new(Some(parse(&text).unwrap()), 1);
    let cache = SourceCache::new(256 << 20, 0);
    let map: std::collections::HashMap<String, PathBuf> = media.iter().cloned().collect();
    let resolver = move |id: &str| map.get(id).cloned();
    let graph = MixGraph::build(&doc, "C", &mut Evaluator::new(), &cache, &resolver).unwrap();
    let voices = graph.voices.len();
    let mut mixer = Mixer::new(
        MixFormat::stereo(48_000),
        Arc::new(doc),
        Arc::new(graph),
        Evaluator::new(),
        cache,
    );
    // The playback look-ahead's read bounds (control points of time-mapped
    // voices, evaluated once per point): their cost per second of audio.
    let t = Instant::now();
    let spans = mixer.source_spans(0, (seconds * 48_000.0) as usize).len();
    let bounds_ms = t.elapsed().as_secs_f64() * 1000.0 / seconds;
    // Decode first so the timing is the mix alone.
    offline::render(
        &mut mixer,
        0,
        (seconds * 48_000.0) as i64,
        48_000,
        |_, _| Ok(()),
    )
    .unwrap();
    let t = Instant::now();
    offline::render(&mut mixer, 0, (seconds * 48_000.0) as i64, 1024, |_, _| {
        Ok(())
    })
    .unwrap();
    let elapsed = t.elapsed().as_secs_f64();
    println!(
        "{label:<40} {voices:>3} voices: {:>7.1}× real time ({:.2} ms per second of audio, {:.3} ms per voice-second); \
         look-ahead read bounds {bounds_ms:.3} ms per second of audio ({spans} spans)",
        seconds / elapsed,
        elapsed * 1000.0 / seconds,
        elapsed * 1000.0 / seconds / voices as f64
    );
}

fn main() {
    let seconds: f64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20.0);
    let dir = tempfile::Builder::new()
        .prefix("pm-audio-bench-")
        .tempdir()
        .unwrap();
    let f48 = dir.path().join("a48.wav");
    let f44 = dir.path().join("a44.wav");
    wav(&f48, 48_000, seconds + 1.0, 440.0);
    wav(&f44, 44_100, seconds + 1.0, 440.0);
    let media = vec![("a48".to_string(), f48), ("a44".to_string(), f44)];
    let audio = |i: usize, asset: &str, extra: &str| {
        format!(
            r#"{{"id":"A{i}","type":"audio","from":0,"dur":{seconds},"on":true,"p":{{}},"d":{{"asset":"{asset}","gain":0.5,"trim":0,"fadeIn":0.1,"fadeOut":0.1{extra}}}}}"#
        )
    };
    let video = |i: usize, speed: &str, pitch: &str| {
        format!(
            r#"{{"id":"V{i}","type":"video","from":0,"dur":{seconds},"on":true,"p":{{}},"d":{{"asset":"a48","speed":{speed},"embeddedAudio":true,"audioGain":0.5,"pitch":"{pitch}"}}}}"#
        )
    };
    run(
        "48 kHz sources (copy)",
        (0..16).map(|i| audio(i, "a48", "")).collect(),
        seconds,
        &media,
    );
    run(
        "44.1 kHz sources (exact-phase sinc)",
        (0..16).map(|i| audio(i, "a44", "")).collect(),
        seconds,
        &media,
    );
    let gain = keys(&[(0.0, 0.1), (seconds / 2.0, 1.0), (seconds, 0.3)]);
    run(
        "48 kHz, keyframed gain (control points)",
        (0..16)
            .map(|i| audio(i, "a48", &format!(r#","gain":{gain}"#)))
            .collect(),
        seconds,
        &media,
    );
    let speed = keys(&[(0.0, 0.5), (seconds, 1.0)]);
    run(
        "variable speed, pitch follows (free-position sinc)",
        (0..4).map(|i| video(i, &speed, "follow")).collect(),
        seconds,
        &media,
    );
    run(
        "constant speed 1.5, pitch follows (free-position sinc)",
        (0..4).map(|i| video(i, "1.5", "follow")).collect(),
        seconds,
        &media,
    );
    run(
        "variable speed, pitch preserved (stretch)",
        (0..4).map(|i| video(i, &speed, "preserve")).collect(),
        seconds,
        &media,
    );
    run(
        "constant speed 1.5, pitch preserved (stretch)",
        (0..4).map(|i| video(i, "1.5", "preserve")).collect(),
        seconds,
        &media,
    );
}
