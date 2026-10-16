//! Pitch-preserving voices through the mixer (plan section 16): pitch and
//! level kept, sounds on their frames, deterministic pre-roll, conformed
//! frame rates, and the quality of offline mixes against references (the
//! library's own one-shot stretch, and FFmpeg's `atempo`).
//!
//! `cargo test --release -p pm-audio --test stretch -- --nocapture` prints
//! the measurements docs/engine/pm-audio.md reports.

mod common;

use std::path::Path;

use common::*;
use pm_audio::MixFormat;
use pm_model::Document;
use pm_model::json::parse;

const SR: u32 = 48_000;

/// Least-squares fit of a sinusoid at `freq` over `x`: (amplitude,
/// residual rms relative to the sinusoid's rms, dB).
fn fit(x: &[f32], freq: f64, rate: u32) -> (f64, f64) {
    let w = std::f64::consts::TAU * freq / rate as f64;
    let (mut ss, mut cc, mut sc, mut xs, mut xc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (w * i as f64).sin_cos();
        ss += s * s;
        cc += c * c;
        sc += s * c;
        xs += v as f64 * s;
        xc += v as f64 * c;
    }
    let det = ss * cc - sc * sc;
    let a = (xs * cc - xc * sc) / det;
    let b = (xc * ss - xs * sc) / det;
    let mut res = 0.0;
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (w * i as f64).sin_cos();
        let e = v as f64 - a * s - b * c;
        res += e * e;
    }
    let amp = (a * a + b * b).sqrt();
    let rms = (res / x.len() as f64).sqrt();
    (amp, 20.0 * (rms / (amp / 2f64.sqrt())).log10())
}

/// The frequency whose fit leaves the least residual, near `freq` (0.01 Hz).
fn best_freq(x: &[f32], freq: f64, rate: u32) -> f64 {
    let mut lo = freq * 0.95;
    let mut hi = freq * 1.05;
    let mut best = freq;
    for step in [freq * 0.001, freq * 0.00005, 0.002] {
        let mut f = lo;
        let mut r = f64::INFINITY;
        while f <= hi {
            let (_, res) = fit(x, f, rate);
            if res < r {
                r = res;
                best = f;
            }
            f += step;
        }
        lo = best - step;
        hi = best + step;
    }
    best
}

fn video(id: &str, from: f64, dur: f64, d: &str) -> String {
    layer(
        id,
        "video",
        from,
        dur,
        &format!(r#"{{"asset":"a","embeddedAudio":true{d}}}"#),
        "",
    )
}

#[test]
fn stretched_voices_keep_their_pitch_and_level() {
    let dir = temp_dir("stretch-pitch");
    let file = dir.join("tone.wav");
    write_wav(&file, SR, &[tone(SR, 12.0, 1000.0, 0.5)]);
    let rig = Rig::new(&[("a", &file)]);
    println!("speed | pitch    | frequency  | level     | residual (THD+N)");
    for speed in [0.5, 0.75, 1.25, 1.5, 2.0] {
        for pitch in ["preserve", "follow"] {
            let doc = project(
                30.0,
                4.0,
                &[video(
                    "V",
                    0.0,
                    4.0,
                    &format!(r#","speed":{speed},"pitch":"{pitch}""#),
                )],
                &[],
                "",
            );
            let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
            let out = rig.render(&mut m, SR as i64, SR as i64 * 3 / 2, 4096);
            let expect = if pitch == "preserve" {
                1000.0
            } else {
                1000.0 * speed
            };
            // The search's step limits the fit; the expected frequency
            // itself is tried too.
            let found = best_freq(&out[0], expect, SR);
            let f = if fit(&out[0], expect, SR).1 <= fit(&out[0], found, SR).1 {
                expect
            } else {
                found
            };
            let (amp, residual) = fit(&out[0], f, SR);
            println!(
                "{speed:5} | {pitch:8} | {f:9.3} Hz | {:+.3} dB | {residual:.1} dB",
                20.0 * (amp / 0.5).log10()
            );
            assert!((f / expect - 1.0).abs() < 2e-4, "{speed} {pitch}: {f} Hz");
            // A phase vocoder's level on a steady tone wanders by a few
            // tenths of a dB at 0.5×.
            let level = 20.0 * (amp / 0.5).log10();
            assert!(level.abs() < 0.6, "{speed} {pitch}: level {level} dB");
            let bar = if pitch == "preserve" { -40.0 } else { -100.0 };
            assert!(residual < bar, "{speed} {pitch}: residual {residual} dB");
        }
    }
}

/// Clicks stretched at 0.5×, 0.8×, and 1.6× sound where the time map puts
/// them (within 1 ms; the vocoder smears a click over its analysis block,
/// so its peak is measured as the energy centroid).
#[test]
fn stretched_sounds_stay_on_their_frames() {
    let dir = temp_dir("stretch-place");
    let file = dir.join("bursts.wav");
    // Short 3 kHz bursts at 0.5, 1.5, 2.5, 3.5 s.
    let mut x = vec![0f32; SR as usize * 5];
    for at in [0.5, 1.5, 2.5, 3.5] {
        let c = (at * SR as f64) as i64;
        for d in -96i64..96 {
            let w = 0.5 + 0.5 * (std::f64::consts::PI * d as f64 / 96.0).cos();
            x[(c + d) as usize] =
                (0.8 * w * (std::f64::consts::TAU * 3000.0 * d as f64 / SR as f64).sin()) as f32;
        }
    }
    write_wav(&file, SR, &[x]);
    let rig = Rig::new(&[("a", &file)]);
    for speed in [0.5f64, 0.8, 1.6] {
        let dur = 3.6 / speed;
        let doc = project(
            30.0,
            dur,
            &[video("V", 0.1, dur, &format!(r#","speed":{speed}"#))],
            &[],
            "",
        );
        let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
        let out = rig.render(&mut m, 0, (dur * SR as f64) as i64, 4096);
        let mut errors = vec![];
        for at in [0.5, 1.5, 2.5, 3.5] {
            let expect = 0.1 + at / speed;
            if (expect + 0.06) * SR as f64 > out[0].len() as f64 {
                continue;
            }
            let c = (expect * SR as f64) as usize;
            let (lo, hi) = (c - SR as usize / 20, c + SR as usize / 20);
            let (mut num, mut den) = (0.0, 0.0);
            for (i, v) in out[0][lo..hi].iter().enumerate() {
                let e = (*v as f64).powi(2);
                num += e * (lo + i) as f64;
                den += e;
            }
            let err_ms = (num / den - c as f64) / SR as f64 * 1000.0;
            errors.push(err_ms);
            assert!(
                err_ms.abs() < 1.0,
                "speed {speed}: burst at {at} off by {err_ms} ms"
            );
        }
        println!("speed {speed}: burst placement errors {errors:.3?} ms");
    }
}

/// Deterministic pre-roll: a render from a sample is the same in any block
/// size, twice, and in playback order; a render that played on from an
/// earlier start keeps the same spectrum (only the vocoder's phases differ).
#[test]
fn stretched_renders_are_deterministic() {
    let dir = temp_dir("stretch-determinism");
    let file = dir.join("music.wav");
    write_wav(&file, SR, &music(SR, 8.0));
    let rig = Rig::new(&[("a", &file)]);
    // Variable speed 0.5 → 1.7 over the clip, preserving pitch.
    let doc = project(
        30.0,
        6.0,
        &[video(
            "V",
            0.25,
            5.5,
            &format!(r#","trim":0.1,"speed":{}"#, keys(&[(0.0, 0.5), (5.5, 1.7)])),
        )],
        &[],
        "",
    );
    let whole = rig.render(
        &mut rig.mixer(doc.clone(), "C", MixFormat::stereo(SR)),
        0,
        SR as i64 * 6,
        1 << 20,
    );
    for start in [12_345i64, 96_001, 150_000] {
        let reference = rig.render(
            &mut rig.mixer(doc.clone(), "C", MixFormat::stereo(SR)),
            start,
            start + 30_000,
            30_000,
        );
        for block in [64usize, 1000, 4096] {
            let part = rig.render(
                &mut rig.mixer(doc.clone(), "C", MixFormat::stereo(SR)),
                start,
                start + 30_000,
                block,
            );
            assert!(
                part == reference,
                "from {start} in blocks of {block}: differs from the same render in one block"
            );
        }
        let from_zero: Vec<Vec<f32>> = whole
            .iter()
            .map(|c| c[start as usize..start as usize + 30_000].to_vec())
            .collect();
        let lsd = log_spectral_distance(&reference[0], &from_zero[0], SR);
        println!(
            "from {start}: fresh start against the run from 0, log-spectral distance {lsd:.2} dB"
        );
        assert!(lsd < 3.0, "from {start}: {lsd} dB");
    }
}

/// A seek deep into a long stretched clip costs one grid step and the
/// pre-roll, not the clip so far.
#[test]
fn a_seek_into_a_long_stretched_clip_costs_a_bounded_pre_roll() {
    let dir = temp_dir("stretch-seek");
    let file = dir.join("long.wav");
    write_wav(&file, SR, &[tone(SR, 300.0, 440.0, 0.3)]);
    let rig = Rig::new(&[("a", &file)]);
    let doc = project(
        30.0,
        200.0,
        &[video("V", 0.0, 200.0, r#","speed":1.25"#)],
        &[],
        "",
    );
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    // Warm the source blocks so the measurement is the stretch's.
    rig.render(
        &mut m,
        SR as i64 * 150 - SR as i64,
        SR as i64 * 150 + 4096,
        1 << 20,
    );
    let mut m2 = rig.mixer(
        project(
            30.0,
            200.0,
            &[video("V", 0.0, 200.0, r#","speed":1.25"#)],
            &[],
            "",
        ),
        "C",
        MixFormat::stereo(SR),
    );
    let t = std::time::Instant::now();
    rig.render(&mut m2, SR as i64 * 150, SR as i64 * 150 + 256, 256);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("first 256 samples at 150 s of a stretched clip: {ms:.1} ms");
    assert!(ms < 2_000.0);
}

fn project_with_assets(layers: &[String], assets: &str, dur: f64) -> Document {
    let text = format!(
        r#"{{"id":"P","compId":"C","name":"T","w":320,"h":180,"fps":24,"dur":{dur},"layers":[{}],"comps":{{}},"assets":{assets},"params":{{}}}}"#,
        layers.join(",")
    );
    Document::new(Some(parse(&text).unwrap()), 1)
}

/// A clip of 23.976 fps footage conformed to 24 or 25 (Interpret Footage)
/// plays its soundtrack with its frames: a sound at native time t is heard
/// at clip time t × 23.976 / conformed.
#[test]
fn conformed_footage_keeps_its_soundtrack_on_its_frames() {
    let dir = temp_dir("stretch-conform");
    let file = dir.join("footage-23976.mov");
    // Bursts at 1, 2, 3, 4 s of native time and a 1 kHz bed.
    generate(
        &file,
        "testsrc2=s=160x90:r=24000/1001:d=6",
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.2*sin(2*PI*1000*t)+0.8*sin(2*PI*3000*t)*exp(-((t-floor(t+0.5))*200)^2)*gt(t\\,0.5):s=48000:d=6",
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "mjpeg",
            "-c:a",
            "pcm_f32le",
        ],
    );
    assert_eq!(
        pm_audio::conform::native_rate(&file),
        Some(pm_time::Rate::FPS_23_976)
    );
    let rig = Rig::new(&[("a", &file)]);
    for (conform, rational, pitch) in [
        (24.0, "[24,1]", "preserve"),
        (25.0, "[25,1]", "preserve"),
        (25.0, "[25,1]", "follow"),
    ] {
        let assets = format!(
            r#"{{"a":{{"id":"a","type":"video","interpret":{{"frameRate":{conform},"frameRateRational":{rational}}}}}}}"#
        );
        let factor = conform / (24_000.0 / 1001.0);
        let dur = 5.0 / factor;
        let doc = project_with_assets(
            &[video("V", 0.0, dur, &format!(r#","pitch":"{pitch}""#))],
            &assets,
            dur,
        );
        let graph = rig.graph(&doc, "C");
        assert!((graph.voices[0].conform - factor).abs() < 1e-12);
        let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
        let out = rig.render(&mut m, 0, (dur * SR as f64) as i64, 4096);
        let mut errors = vec![];
        for native in [1.0, 2.0, 3.0, 4.0] {
            let c = (native / factor * SR as f64) as usize;
            let (lo, hi) = (c - SR as usize / 20, c + SR as usize / 20);
            // Energy centroid of the burst above the 1 kHz bed (high-passed
            // by a first difference).
            let (mut num, mut den) = (0.0, 0.0);
            for i in lo..hi {
                let e = ((out[0][i] - out[0][i - 1]) as f64).powi(2);
                num += e * i as f64;
                den += e;
            }
            errors.push((num / den - c as f64) / SR as f64 * 1000.0);
        }
        // The bed's pitch: preserved, or moved by the factor.
        let bed = &out[0][(0.55 / factor * SR as f64) as usize..][..SR as usize / 3];
        let expect = if pitch == "follow" {
            1000.0 * factor
        } else {
            1000.0
        };
        let f = best_freq(bed, expect, SR);
        println!(
            "23.976 conformed to {conform} ({pitch}): factor {factor:.6}, burst errors {errors:.3?} ms, bed {f:.2} Hz"
        );
        for e in &errors {
            assert!(e.abs() < 1.0, "{conform} {pitch}: {e} ms");
        }
        // Within the unity band (24/23.976) the soundtrack resamples: its
        // pitch moves by 1.7 cents, below hearing.
        let tolerance = if (factor - 1.0).abs() < 0.0015 {
            0.0012
        } else {
            3e-4
        };
        assert!(
            (f / expect - 1.0).abs() < tolerance,
            "{conform} {pitch}: bed {f} Hz"
        );
    }
}

// ── quality against references ──────────────────────────────────────────

/// A deterministic music-like signal: a chord of harmonic tones with
/// vibrato, a bass line, and noise-burst percussion, at -12 dBFS or so.
fn music(rate: u32, seconds: f64) -> Vec<Vec<f32>> {
    let n = (seconds * rate as f64) as usize;
    let mut l = vec![0f32; n];
    let mut r = vec![0f32; n];
    let mut seed = 0x1234_5678u64;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let chords = [
        [220.0, 277.18, 329.63],
        [196.0, 246.94, 293.66],
        [174.61, 220.0, 261.63],
    ];
    for i in 0..n {
        let t = i as f64 / rate as f64;
        let chord = chords[((t / 1.5) as usize) % chords.len()];
        let mut v = 0.0;
        for (k, &f) in chord.iter().enumerate() {
            let vib = 1.0 + 0.004 * (std::f64::consts::TAU * 5.0 * t + k as f64).sin();
            for h in 1..6 {
                v += 0.05 / h as f64
                    * (std::f64::consts::TAU * f * vib * h as f64 * t + h as f64).sin();
            }
        }
        let bass = 0.12 * (std::f64::consts::TAU * chord[0] / 2.0 * t).sin();
        let beat = t.fract() * 2.0 % 1.0;
        let hit = 0.25 * (-beat * 40.0).exp() * noise();
        l[i] = (v + bass + hit) as f32;
        r[i] = (0.9 * v + bass + 0.7 * hit) as f32;
    }
    vec![l, r]
}

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (xr, xi) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
            }
        }
        len <<= 1;
    }
}

/// Magnitude spectra (dB) of 2048-sample Hann frames every 512 samples,
/// bins up to 16 kHz.
fn spectra(x: &[f32], rate: u32) -> Vec<Vec<f64>> {
    const N: usize = 2048;
    let bins = (16_000.0 / rate as f64 * N as f64) as usize;
    let mut out = vec![];
    let mut at = 0;
    while at + N <= x.len() {
        let mut re: Vec<f64> = (0..N)
            .map(|i| {
                let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / N as f64).cos();
                x[at + i] as f64 * w
            })
            .collect();
        let mut im = vec![0.0; N];
        fft(&mut re, &mut im);
        out.push(
            (1..bins)
                .map(|k| 10.0 * (re[k] * re[k] + im[k] * im[k] + 1e-12).log10())
                .collect(),
        );
        at += 512;
    }
    out
}

/// Mean log-spectral distance (dB) between two signals of the same length,
/// over frames of `a` within 50 dB of its loudest, with bins floored 80 dB
/// below the frame's peak (so silence and noise floors do not count).
fn log_spectral_distance(a: &[f32], b: &[f32], rate: u32) -> f64 {
    let (sa, sb) = (spectra(a, rate), spectra(b, rate));
    let loudest = sa
        .iter()
        .map(|f| f.iter().cloned().fold(f64::MIN, f64::max))
        .fold(f64::MIN, f64::max);
    let mut total = 0.0;
    let mut frames = 0;
    for (fa, fb) in sa.iter().zip(&sb) {
        let peak = fa.iter().cloned().fold(f64::MIN, f64::max);
        if peak < loudest - 50.0 {
            continue;
        }
        let floor = peak - 80.0;
        let d: f64 = fa
            .iter()
            .zip(fb)
            .map(|(x, y)| (x.max(floor) - y.max(floor)).powi(2))
            .sum::<f64>()
            / fa.len() as f64;
        total += d.sqrt();
        frames += 1;
    }
    total / frames.max(1) as f64
}

/// The lag (samples, within ±max) that best aligns `b` with `a` by their
/// 256-sample energy envelopes.
fn align(a: &[f32], b: &[f32], max: i64) -> i64 {
    let env = |x: &[f32]| -> Vec<f64> {
        x.chunks(256)
            .map(|c| c.iter().map(|v| (*v as f64).powi(2)).sum::<f64>())
            .collect()
    };
    let (ea, eb) = (env(a), env(b));
    let mut best = (f64::MIN, 0);
    for lag in -max / 256..=max / 256 {
        let mut s = 0.0;
        for (i, a) in ea.iter().enumerate() {
            let j = i as i64 + lag;
            if j >= 0 && (j as usize) < eb.len() {
                s += a * eb[j as usize];
            }
        }
        if s > best.0 {
            best = (s, lag * 256);
        }
    }
    best.1
}

fn read_wav(path: &Path) -> Vec<Vec<f32>> {
    let mut d = pm_media::audio::AudioDecoder::open(path, None).unwrap();
    let channels = d.info().channels();
    let mut out = vec![Vec::new(); channels];
    let mut buf = vec![Vec::new(); channels];
    loop {
        for c in &mut buf {
            c.clear();
        }
        let n = d.read(&mut buf, 1 << 15).unwrap();
        if n == 0 {
            break;
        }
        for (o, b) in out.iter_mut().zip(&buf) {
            o.extend_from_slice(&b[..n]);
        }
    }
    out
}

/// Offline mixes with stretch against references. The clip is the
/// music-like signal at constant speeds; the references are (1) the
/// library's one-shot stretch of the whole clip (the best its algorithm
/// does: no streaming, no pre-roll, no restarts) and (2) FFmpeg's `atempo`
/// (WSOLA, an independent method). Phases differ between methods, so the
/// comparison is the log-spectral distance of aligned 2048-sample frames;
/// the same distance between the source at speed 1 and the reference
/// one-shot calibrates the scale (the algorithm's own change).
#[test]
fn offline_stretch_mixes_against_references() {
    let dir = temp_dir("stretch-quality");
    let file = dir.join("music.wav");
    let source = music(SR, 12.0);
    write_wav(&file, SR, &source);
    let rig = Rig::new(&[("a", &file)]);
    println!(
        "speed | vs one-shot (L/R)  | vs atempo (L/R)    | one-shot vs atempo | fresh start at 3 s vs run"
    );
    for speed in [0.5, 0.8, 1.25, 2.0] {
        let dur = 10.0 / speed;
        let n = (dur * SR as f64) as usize;
        let doc = project(
            30.0,
            dur,
            &[video("V", 0.0, dur, &format!(r#","speed":{speed}"#))],
            &[],
            "",
        );
        let mut m = rig.mixer(doc.clone(), "C", MixFormat::stereo(SR));
        let ours = rig.render(&mut m, 0, n as i64, 4096);
        // (1) The library's one-shot stretch of the same 10 s.
        let input_len = (10.0 * SR as f64) as usize;
        let mut st = pm_stretch::Stretcher::new(2, SR).unwrap();
        let mut one = vec![vec![0f32; n]; 2];
        {
            let input: Vec<&[f32]> = source.iter().map(|c| &c[..input_len]).collect();
            let mut output: Vec<&mut [f32]> = one.iter_mut().map(|c| &mut c[..]).collect();
            assert!(st.exact(&input, &mut output).unwrap());
        }
        // (2) FFmpeg's atempo (chained for 0.5).
        let tempo = dir.join(format!("atempo-{speed}.wav"));
        let _ = std::fs::remove_file(&tempo);
        let status = std::process::Command::new(ffmpeg())
            .args(["-hide_banner", "-loglevel", "error", "-y", "-t", "10", "-i"])
            .arg(&file)
            .args(["-af", &format!("atempo={speed}"), "-c:a", "pcm_f32le"])
            .arg(&tempo)
            .status()
            .unwrap();
        assert!(status.success());
        let atempo = read_wav(&tempo);
        // Measure 1 s in from each end of the overlap.
        let body = |x: &[f32], lag: i64| -> Vec<f32> {
            let s = (SR as i64 + lag).max(0) as usize;
            let e = (n - SR as usize).min(x.len());
            x[s.min(e)..e].to_vec()
        };
        let measure = |a: &[f32], b: &[f32]| -> f64 {
            let lag = align(a, b, 4096);
            let (x, y) = (body(a, 0), body(b, lag));
            let len = x.len().min(y.len());
            log_spectral_distance(&x[..len], &y[..len], SR)
        };
        let vs_one = [measure(&ours[0], &one[0]), measure(&ours[1], &one[1])];
        let vs_atempo = [measure(&ours[0], &atempo[0]), measure(&ours[1], &atempo[1])];
        let one_atempo = measure(&one[0], &atempo[0]);
        // A fresh start (a seek to 3 s) against the run from 0 over the next 2 s.
        let at = 3 * SR as usize;
        let mut fresh = rig.mixer(doc, "C", MixFormat::stereo(SR));
        let seeked = rig.render(&mut fresh, at as i64, (at + 2 * SR as usize) as i64, 4096);
        let fresh_lsd = log_spectral_distance(&seeked[0], &ours[0][at..at + 2 * SR as usize], SR);
        println!(
            "{speed:5} | {:5.2} / {:5.2} dB    | {:5.2} / {:5.2} dB    | {one_atempo:5.2} dB           | {fresh_lsd:.2} dB",
            vs_one[0], vs_one[1], vs_atempo[0], vs_atempo[1]
        );
        assert!(vs_one[0] < 2.0 && vs_one[1] < 2.0, "{speed}: {vs_one:?}");
        assert!(fresh_lsd < 3.0, "{speed}: fresh start {fresh_lsd}");
    }
}
