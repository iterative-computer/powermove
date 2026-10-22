//! The mixer against the section 2 and 25 audio bars and the legacy level
//! rules, on generated media: clicks land on their frames, exported sample
//! counts are exact, renders are identical whatever block or start, gains
//! and fades follow `gainAt`, precompositions nest, solo and mute silence,
//! speed and remaps are followed, layouts fold.

mod common;

use common::*;
use std::sync::Arc;

use pm_audio::MixFormat;
use pm_audio::layout::FOLD;
use pm_audio::offline::sample_range;
use pm_time::{Rate, TICKS_PER_SECOND};

const SR: u32 = 48_000;

fn frame_sample(n: i64, rate: u32, fps: Rate) -> i64 {
    // floor(N × sr × den / num)
    (n as i128 * rate as i128 * fps.den() as i128).div_euclid(fps.num() as i128) as i64
}

#[test]
fn clicks_land_on_their_frames() {
    let dir = temp_dir("clicks");
    for (fps_f, fps, rate) in [
        (30.0, Rate::FPS_30, 48_000u32),
        (29.97, Rate::FPS_29_97, 48_000),
        (23.976, Rate::FPS_23_976, 48_000),
        (29.97, Rate::FPS_29_97, 44_100),
        (25.0, Rate::FPS_25, 96_000),
    ] {
        let frames = [1i64, 2, 30, 59, 61];
        let at: Vec<usize> = frames
            .iter()
            .map(|&n| frame_sample(n, rate, fps) as usize)
            .collect();
        let file = dir.join(format!("clicks-{rate}-{}.wav", fps.num()));
        let len = rate as usize * 3;
        write_wav(
            &file,
            rate,
            &[clicks(len, &at, 0.9), clicks(len, &at, -0.9)],
        );
        for project_rate in [48_000u32, 44_100] {
            let doc = project(
                fps_f,
                3.0,
                &[audio("A", "click", 0.0, 3.0, "", "")],
                &[],
                &format!(r#","audioSampleRate":{project_rate}"#),
            );
            let rig = Rig::new(&[("click", &file)]);
            let mut m = rig.mixer(doc, "C", MixFormat::stereo(project_rate));
            let out = rig.render(&mut m, 0, project_rate as i64 * 3, 4096);
            let found = peaks(&out[0], 0.3);
            let expected: Vec<i64> = frames
                .iter()
                .map(|&n| frame_sample(n, project_rate, fps))
                .collect();
            assert_eq!(
                found.len(),
                expected.len(),
                "{rate}->{project_rate} at {fps_f}: {found:?}"
            );
            for (f, e) in found.iter().zip(&expected) {
                let d = *f as i64 - e;
                assert!(
                    d.abs() <= 1,
                    "{rate} Hz source at {fps_f} fps into {project_rate} Hz: click at {f}, frame start {e}"
                );
                if rate == project_rate {
                    assert_eq!(d, 0, "same-rate clicks are exact");
                }
            }
            // The right channel is the negated click.
            assert!(out[1][found[0]] < -0.3);
        }
    }
}

#[test]
fn exported_sample_counts_are_exact() {
    // A range of whole frames at 29.97 covers round(duration × sr) samples.
    for (fps, frames, rate) in [
        (Rate::FPS_29_97, 1001i64, 48_000u32),
        (Rate::FPS_23_976, 2_400, 44_100),
        (Rate::FPS_59_94, 7, 48_000),
        (Rate::FPS_25, 250, 96_000),
    ] {
        let end = fps.frame_start(frames).unwrap();
        let (a, b) = sample_range(0, end, rate);
        let duration = end as f64 / TICKS_PER_SECOND as f64;
        assert_eq!(a, 0);
        assert_eq!(
            b,
            (duration * rate as f64).round() as i64,
            "{frames} frames at {fps:?} into {rate}"
        );
    }
    // An actual render produces exactly that many frames.
    let dir = temp_dir("count");
    let file = dir.join("tone.wav");
    write_wav(&file, SR, &[tone(SR, 2.0, 440.0, 0.5)]);
    let doc = project(29.97, 2.0, &[audio("A", "t", 0.0, 2.0, "", "")], &[], "");
    let rig = Rig::new(&[("t", &file)]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let end = Rate::FPS_29_97.frame_start(59).unwrap();
    let (a, b) = sample_range(0, end, SR);
    let out = rig.render(&mut m, a, b, 1000);
    assert_eq!(
        out[0].len() as i64,
        (end as f64 / TICKS_PER_SECOND as f64 * SR as f64).round() as i64
    );
}

/// Every render is a pure function of the sample range: blocks of any size,
/// starting anywhere, give the same samples (deterministic pre-roll).
#[test]
fn renders_are_identical_from_any_start() {
    let dir = temp_dir("determinism");
    let f48 = dir.join("a48.wav");
    let f44 = dir.join("a44.wav");
    write_wav(
        &f48,
        48_000,
        &[tone(48_000, 4.0, 700.0, 0.4), tone(48_000, 4.0, 900.0, 0.3)],
    );
    write_wav(&f44, 44_100, &[tone(44_100, 4.0, 1234.5, 0.4)]);
    let layers = [
        audio(
            "A",
            "a48",
            0.1,
            3.0,
            &format!(
                r#","gain":{},"fadeIn":0.2,"fadeOut":0.4"#,
                keys(&[(0.0, 0.2), (1.3, 1.4), (2.5, 0.6)])
            ),
            "",
        ),
        audio("B", "a44", 0.0, 3.5, r#","trim":0.05"#, ""),
        layer(
            "V",
            "video",
            0.3,
            3.0,
            &format!(
                r#"{{"asset":"a48","trim":0.1,"speed":{},"embeddedAudio":true,"audioGain":0.8,"pitch":"follow"}}"#,
                keys(&[(0.0, 0.5), (3.0, 1.7)])
            ),
            "",
        ),
    ];
    let doc = project(30.0, 4.0, &layers, &[], "");
    let rig = Rig::new(&[("a48", &f48), ("a44", &f44)]);
    let mut m = rig.mixer(doc.clone(), "C", MixFormat::stereo(SR));
    let whole = rig.render(&mut m, 0, SR as i64 * 4, 1 << 20);
    for (start, block) in [
        (0i64, 64usize),
        (12_345, 1000),
        (96_001, 4096),
        (150_000, 333),
    ] {
        let mut m = rig.mixer(doc.clone(), "C", MixFormat::stereo(SR));
        let part = rig.render(&mut m, start, start + 20_000, block);
        for c in 0..2 {
            let a = &whole[c][start as usize..start as usize + 20_000];
            assert!(
                a == &part[c][..],
                "channel {c} differs when rendering from {start} in blocks of {block}"
            );
        }
    }
    assert!(whole[0].iter().any(|v| v.abs() > 0.1));
}

/// Gain, fades, composition levels, and the master on a constant source:
/// every output sample equals the legacy `gainAt` product.
#[test]
fn levels_follow_gain_at() {
    let dir = temp_dir("levels");
    let dc = dir.join("dc.wav");
    write_wav(&dc, SR, &[vec![0.5; SR as usize * 3]]);
    let gain = keys(&[(0.0, 0.2), (1.0, 1.5), (2.0, 0.5)]);
    let doc = project(
        30.0,
        3.0,
        &[audio(
            "A",
            "dc",
            0.25,
            2.5,
            &format!(r#","gain":{gain},"fadeIn":0.3,"fadeOut":0.5,"trim":0.1"#),
            "",
        )],
        &[],
        r#","audioGain":0.8"#,
    );
    let rig = Rig::new(&[("dc", &dc)]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut m, 0, SR as i64 * 3, 4096);
    let audible = 2.5f64.min(3.0 - 0.1);
    let mut worst = 0f64;
    for (n, &v) in out[0].iter().enumerate() {
        let t = n as f64 / SR as f64;
        let local = t - 0.25;
        let expected = if !(0.0..2.5).contains(&local) {
            0.0
        } else {
            let g = if local <= 1.0 {
                0.2 + 1.3 * local
            } else if local <= 2.0 {
                1.5 - (local - 1.0)
            } else {
                0.5
            };
            let l = local.clamp(0.0, audible);
            let fade = (l / 0.3)
                .clamp(0.0, 1.0)
                .min(((audible - l) / 0.5).clamp(0.0, 1.0));
            0.5 * 0.8 * g.clamp(0.0, 4.0) * fade
        };
        let e = (v as f64 - expected).abs();
        if e > worst {
            worst = e;
            if e > 1e-3 {
                eprintln!("sample {n} (t {t:.6}): got {v}, expected {expected}");
            }
        }
    }
    // Control points every 16 samples, linear between: exact for linear keys
    // except within one control interval of a key.
    assert!(worst < 2e-4, "worst level error {worst}");
    assert_eq!(out[0][(0.2 * SR as f64) as usize], 0.0);
}

/// One composition used twice, offset and trimmed, through precomposition and
/// composition levels.
#[test]
fn precompositions_nest() {
    let dir = temp_dir("nest");
    let file = dir.join("click.wav");
    let at = [4_800usize, 48_000];
    write_wav(&file, SR, &[clicks(SR as usize * 3, &at, 0.8)]);
    let inner = comp(
        30.0,
        3.0,
        &[audio("I", "click", 0.25, 2.5, r#","trim":0.05"#, "")],
        r#","audioGain":0.5"#,
    );
    let layers = [
        layer(
            "PA",
            "precomp",
            0.0,
            3.0,
            r#"{"comp":"X","audioGain":0.5}"#,
            "",
        ),
        layer(
            "PB",
            "precomp",
            0.5,
            2.5,
            r#"{"comp":"X","trim":0.2,"audioGain":1}"#,
            "",
        ),
    ];
    let doc = project(30.0, 3.0, &layers, &[("X".into(), inner)], "");
    let rig = Rig::new(&[("click", &file)]);
    let graph = rig.graph(&doc, "C");
    let ids: Vec<&str> = graph.voices.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, vec!["PA/I", "PB/I"]);
    assert_eq!(graph.strips, vec!["PA", "PB"]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut m, 0, SR as i64 * 3, 4096);
    let found = peaks(&out[0], 0.05);
    // PA: inner time = t; click at source s plays at t = 0.25 + (s − 0.05).
    // PB: inner time = t − 0.5 + 0.2.
    let sec = |s: f64| (s * SR as f64).round() as usize;
    let mut expected = vec![
        sec(0.25 - 0.05 + 0.1),
        sec(0.25 - 0.05 + 1.0),
        sec(0.5 - 0.2 + 0.25 - 0.05 + 0.1),
        sec(0.5 - 0.2 + 0.25 - 0.05 + 1.0),
    ];
    expected.sort();
    assert_eq!(found, expected);
    // Levels: 0.8 × inner comp 0.5 × precomp 0.5 (A) or 1 (B).
    assert!((out[0][sec(0.3)] - 0.8 * 0.25).abs() < 1e-6);
    assert!((out[0][sec(0.6)] - 0.8 * 0.5).abs() < 1e-6);
}

/// The shape of today's app projects: every content field a channel object
/// (`PM.P(v)`), as the export-encoding `automation` fixture builds them. A
/// precomposition's static `trim` channel once reached the linear source-time
/// map as an object (`Number({…})` is NaN), so the nested voice and the whole
/// export mix became NaN, written as silence.
#[test]
fn channel_valued_precomposition_fields_mix_like_numbers() {
    let dir = temp_dir("nest-channels");
    let dc = dir.join("dc.wav");
    write_wav(&dc, SR, &[vec![0.5; SR as usize * 3]]);
    let render = |channels: bool| {
        let v = |x: f64| {
            if channels {
                ch(&x.to_string())
            } else {
                x.to_string()
            }
        };
        let inner = comp(
            30.0,
            1.0,
            &[audio(
                "I",
                "dc",
                0.0,
                1.0,
                &format!(
                    r#","gain":{},"trim":{},"speed":{},"fadeIn":{},"fadeOut":{}"#,
                    v(0.35),
                    v(0.1),
                    v(1.0),
                    v(0.08),
                    v(0.1)
                ),
                "",
            )],
            // Today reads a composition's own level with `Number(…)`: a
            // channel object there is unity, in both forms.
            &format!(
                r#","audioGain":{}"#,
                if channels { ch("0.7") } else { "1".into() }
            ),
        );
        let gain = keys(&[(0.0, 0.25), (0.4, 0.9), (0.85, 0.4)]);
        let layers = [
            audio(
                "A",
                "dc",
                0.1,
                0.85,
                &format!(
                    r#","gain":{gain},"trim":{},"fadeIn":{},"fadeOut":{}"#,
                    v(0.05),
                    v(0.14),
                    v(0.2)
                ),
                "",
            ),
            layer(
                "P",
                "precomp",
                0.0,
                1.0,
                &format!(
                    r#"{{"comp":"X","audioGain":{},"trim":{}}}"#,
                    v(0.75),
                    v(0.0)
                ),
                "",
            ),
        ];
        let doc = project(30.0, 1.0, &layers, &[("X".into(), inner)], "");
        let rig = Rig::new(&[("dc", &dc)]);
        let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
        rig.render(&mut m, 0, SR as i64, 4096)
    };
    let numbers = render(false);
    let channels = render(true);
    for out in [&numbers, &channels] {
        assert!(out[0].iter().all(|v| v.is_finite()), "the mix has NaN");
        // The nested clip alone at 0.05 s: 0.5 × 0.35 × 0.75 × fade 0.625.
        let at = (0.05 * SR as f64) as usize;
        assert!(
            (out[0][at] - 0.5 * 0.35 * 0.75 * 0.625).abs() < 1e-4,
            "{}",
            out[0][at]
        );
    }
    assert_eq!(numbers, channels);
}

#[test]
fn solo_mute_and_groups_silence() {
    let dir = temp_dir("solo");
    let dc = dir.join("dc.wav");
    write_wav(&dc, SR, &[vec![0.25; SR as usize * 3]]);
    // Solo: only A sounds.
    let doc = project(
        30.0,
        2.0,
        &[
            audio("A", "dc", 0.0, 2.0, "", "").replace(r#""solo":false"#, r#""solo":true"#),
            audio("B", "dc", 0.0, 2.0, "", ""),
        ],
        &[],
        "",
    );
    let rig = Rig::new(&[("dc", &dc)]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut m, 0, SR as i64, 4096);
    assert!((out[0][1000] - 0.25).abs() < 1e-7);
    // Mute (static off), a group switched off at 1 s, a layer off 0.5–1.5 s.
    let group = layer("G", "group", 0.0, 2.0, "{}", "").replace(
        r#""on":true"#,
        &format!(r#""on":{}"#, bool_keys(&[(0.0, true), (1.0, false)])),
    );
    let in_group = audio("A", "dc", 0.0, 2.0, "", "").replace(r#""group":null"#, r#""group":"G""#);
    let muted = audio("B", "dc", 0.0, 2.0, "", "").replace(r#""on":true"#, r#""on":false"#);
    let flicker = audio("C", "dc", 0.0, 2.0, r#","gain":2"#, "").replace(
        r#""on":true"#,
        &format!(
            r#""on":{}"#,
            bool_keys(&[(0.0, true), (0.5, false), (1.5, true)])
        ),
    );
    let doc = project(30.0, 2.0, &[group, in_group, muted, flicker], &[], "");
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut m, 0, SR as i64 * 2, 4096);
    let at = |s: f64| out[0][(s * SR as f64) as usize];
    assert!((at(0.25) - 0.75).abs() < 1e-6, "{}", at(0.25)); // A + C×2
    assert!((at(0.75) - 0.25).abs() < 1e-6, "{}", at(0.75)); // A only
    assert!(at(1.25).abs() < 1e-6, "{}", at(1.25)); // nothing
    assert!((at(1.75) - 0.5).abs() < 1e-6, "{}", at(1.75)); // C only
}

/// Speed and remaps are followed (appendix A item 10), pitch following speed.
#[test]
fn speed_and_remaps_are_followed() {
    let dir = temp_dir("speed");
    let file = dir.join("click.wav");
    write_wav(
        &file,
        SR,
        &[clicks(SR as usize * 4, &[24_000, 72_000, 120_000], 0.9)],
    );
    // Constant speed 2 on a video soundtrack: source clicks at 0.5, 1.5, 2.5 s
    // sound at 0.25, 0.75, 1.25 s.
    let v = layer(
        "V",
        "video",
        0.0,
        2.0,
        r#"{"asset":"c","speed":2,"embeddedAudio":true}"#,
        "",
    );
    let doc = project(30.0, 2.0, &[v], &[], "");
    let rig = Rig::new(&[("c", &file)]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut m, 0, SR as i64 * 2, 4096);
    let found = peaks(&out[0], 0.3);
    assert_eq!(found.len(), 3, "{found:?}");
    for (f, e) in found.iter().zip([12_000usize, 36_000, 60_000]) {
        assert!(
            (*f as i64 - e as i64).abs() <= 1,
            "click at {f}, expected {e}"
        );
    }
    // With `pitch: "follow"` the pitch follows the speed: a 1 kHz tone at
    // speed 1.5 sounds at 1.5 kHz.
    let tone_file = dir.join("tone.wav");
    write_wav(&tone_file, SR, &[tone(SR, 4.0, 1000.0, 0.5)]);
    let v = layer(
        "V",
        "video",
        0.0,
        2.0,
        r#"{"asset":"t","speed":1.5,"embeddedAudio":true,"pitch":"follow"}"#,
        "",
    );
    let doc = project(30.0, 2.0, &[v], &[], "");
    let rig = Rig::new(&[("t", &tone_file)]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut m, 4_800, 4_800 + 9_600, 4096);
    let crossings = out[0]
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    assert!(
        (crossings as i64 - 300).abs() <= 1,
        "{crossings} cycles in 0.2 s"
    );
    // A time remap running backwards: source time 3 → 0 over 1.5 s plays the
    // clicks reversed at 1.5 − s / 2.
    let remap = keys(&[(0.0, 3.0), (1.5, 0.0)]);
    let v = layer(
        "V",
        "video",
        0.0,
        1.5,
        &format!(r#"{{"asset":"c","timeRemap":true,"sourceTime":{remap},"embeddedAudio":true}}"#),
        "",
    );
    let doc = project(30.0, 1.5, &[v], &[], "");
    let rig = Rig::new(&[("c", &file)]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut m, 0, SR as i64 * 3 / 2, 4096);
    let found = peaks(&out[0], 0.2);
    let expected = [0.25, 0.75, 1.25].map(|t| (t * SR as f64) as i64);
    assert_eq!(found.len(), 3, "{found:?}");
    for (f, e) in found.iter().zip(expected) {
        assert!(
            (*f as i64 - e).abs() <= 2,
            "reversed click at {f}, expected {e}"
        );
    }
}

#[test]
fn layouts_fold_into_stereo() {
    let dir = temp_dir("layouts");
    let mono = dir.join("mono.wav");
    write_wav(&mono, 22_050, &[vec![0.5; 22_050]]);
    // A 5.1 source: the writer marks 6 channels as 5.1 (side).
    let six = dir.join("six.wav");
    let ch: Vec<Vec<f32>> = [0.1f32, 0.2, 0.3, 0.4, 0.05, 0.06]
        .iter()
        .map(|&v| vec![v; SR as usize])
        .collect();
    write_wav(&six, SR, &ch);
    let doc = project(30.0, 1.0, &[audio("M", "m", 0.0, 1.0, "", "")], &[], "");
    let rig = Rig::new(&[("m", &mono), ("s", &six)]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut m, 0, SR as i64, 4096);
    assert!((out[0][24_000] - 0.5).abs() < 1e-5 && (out[1][24_000] - 0.5).abs() < 1e-5);
    let doc = project(30.0, 1.0, &[audio("S", "s", 0.0, 1.0, "", "")], &[], "");
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut m, 0, SR as i64, 4096);
    let l = 0.1 + FOLD * (0.3 + 0.05);
    let r = 0.2 + FOLD * (0.3 + 0.06);
    assert!((out[0][1000] - l).abs() < 1e-6, "{} vs {l}", out[0][1000]);
    assert!((out[1][1000] - r).abs() < 1e-6, "{} vs {r}", out[1][1000]);
}

#[test]
fn missing_media_is_reported() {
    let doc = project(
        30.0,
        1.0,
        &[audio("A", "nowhere", 0.0, 1.0, "", "")],
        &[],
        "",
    );
    let rig = Rig::new(&[]);
    let graph = rig.graph(&doc, "C");
    assert!(graph.voices.is_empty());
    assert_eq!(graph.problems.len(), 1);
    assert_eq!(graph.problems[0].1, "nowhere");
}

/// The monitor solo is a listening aid: strips outside it are silent in the
/// mix, but their meters (read before the solo, as the legacy taps) move.
#[test]
fn monitor_solo_and_meters() {
    use pm_audio::RenderOptions;
    let dir = temp_dir("monitor");
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    write_wav(&a, SR, &[vec![0.25; SR as usize]]);
    write_wav(&b, SR, &[vec![0.5; SR as usize]]);
    let doc = project(
        30.0,
        1.0,
        &[
            audio("A", "a", 0.0, 1.0, "", ""),
            audio("B", "b", 0.0, 1.0, "", ""),
        ],
        &[],
        r#","audioGain":0.5"#,
    );
    let rig = Rig::new(&[("a", &a), ("b", &b)]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let mut out = vec![Vec::new(), Vec::new()];
    let options = RenderOptions {
        wait: true,
        monitor_solo: Some(["B".to_string()].into_iter().collect()),
        meters: true,
        ..Default::default()
    };
    m.render(1000, 4096, &mut out, &options).unwrap();
    assert!((out[0][100] - 0.25).abs() < 1e-7, "{}", out[0][100]); // B × master 0.5
    let meters = m.meters(5096);
    let strip = |id: &str| {
        meters
            .strips
            .iter()
            .find(|(s, _)| s == id)
            .unwrap()
            .1
            .clone()
    };
    assert!((strip("A")[0].peak - 0.25).abs() < 1e-7);
    assert!((strip("B")[1].rms - 0.5).abs() < 1e-6);
    assert!((meters.master[0].peak - 0.25).abs() < 1e-7);
}

/// A video without a soundtrack plays nothing and reports nothing.
#[test]
fn a_silent_video_is_not_a_problem() {
    let fixtures = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures");
    let v = layer(
        "V",
        "video",
        0.0,
        1.0,
        r#"{"asset":"v","embeddedAudio":true}"#,
        "",
    );
    let doc = project(30.0, 1.0, &[v], &[], "");
    let rig = Rig::new(&[("v", &fixtures.join("prores4444-alpha.mov"))]);
    let graph = rig.graph(&doc, "C");
    assert!(graph.voices.is_empty());
    assert!(graph.problems.is_empty(), "{:?}", graph.problems);
}

/// An animated clip that starts between control points starts on its sample.
#[test]
fn animated_clips_start_on_their_sample() {
    let dir = temp_dir("start");
    let dc = dir.join("dc.wav");
    write_wav(&dc, SR, &[vec![0.5; SR as usize]]);
    let from = 4805.0 / SR as f64;
    let doc = project(
        30.0,
        1.0,
        &[audio(
            "A",
            "dc",
            from,
            0.5,
            &format!(r#","gain":{}"#, keys(&[(0.0, 1.0), (0.5, 1.0)])),
            "",
        )],
        &[],
        "",
    );
    let rig = Rig::new(&[("dc", &dc)]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut m, 0, 9_600, 1024);
    assert_eq!(out[0][4804], 0.0);
    assert!((out[0][4805] - 0.5).abs() < 1e-7, "{}", out[0][4805]);
}

/// A clip whose in-point is a frame start that falls on or between samples
/// plays its first source sample at the rounded project sample, never one
/// later: `sample_at` treats a time within amendment 1's tolerance of a sample
/// as that sample (frame 7 of 25 fps is 0.28000000000000003 s in f64), and a
/// copy reader's window is measured on its whole-sample shift (frame 7 of
/// 29.97 fps is 11,211.2 samples). Both used to drop the first sample.
#[test]
fn clips_on_frame_starts_keep_their_first_sample() {
    use pm_audio::mixer::sample_at;
    assert_eq!(sample_at(7.0 / 25.0, 48_000.0), 13_440);
    assert_eq!(
        sample_at(0.280_000_1, 48_000.0),
        13_441,
        "off the tolerance: ceil"
    );
    assert_eq!(sample_at(13_439.5 / 48_000.0, 48_000.0), 13_440);
    assert_eq!(sample_at(0.0, 48_000.0), 0);
    assert_eq!(sample_at(-7.0 / 25.0, 48_000.0), -13_440);
    // The tolerance scales with n: an hour in, f64 error is still absorbed.
    assert_eq!(sample_at(3_600.0 + 7.0 / 25.0, 48_000.0), 172_813_440);

    let dir = temp_dir("first-sample");
    let file = dir.join("click.wav");
    let mut click = vec![0f32; SR as usize];
    click[0] = 0.9;
    click[1] = 0.45;
    write_wav(&file, SR, &[click]);
    for (fps, frame, from) in [
        (25.0, 7, 7.0 / 25.0),
        (29.97, 7, 7.0 * 1001.0 / 30_000.0),
        (23.976, 5, 5.0 * 1001.0 / 24_000.0),
        (30.0, 11, 11.0 / 30.0),
    ] {
        let doc = project(fps, 2.0, &[audio("A", "click", from, 1.0, "", "")], &[], "");
        let rig = Rig::new(&[("click", &file)]);
        let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
        let out = rig.render(&mut m, 0, SR as i64 * 2, 1024);
        let at = (from * SR as f64).round() as usize;
        assert_eq!(
            out[0][at - 1],
            0.0,
            "{fps} fps frame {frame}: nothing before the clip"
        );
        assert!(
            (out[0][at] - 0.9).abs() < 1e-6 && (out[0][at + 1] - 0.45).abs() < 1e-6,
            "{fps} fps frame {frame}: first source samples at {at}: {:?}",
            &out[0][at - 1..at + 3]
        );
    }
}

/// A live edit during playback glides into the new mix over 12 ms
/// (`RETUNE_GLIDE` today): a level change ramps linearly, overlapping edits
/// (a fader dragged at 60 Hz) stay continuous, a monitor solo crossfades, and
/// after the glide the mix equals the new document's render exactly.
#[test]
fn live_edits_glide_without_clicks() {
    use pm_audio::{RETUNE_GLIDE_MS, RenderOptions};
    let dir = temp_dir("retune");
    let dc = dir.join("dc.wav");
    let t = dir.join("tone.wav");
    write_wav(&dc, SR, &[vec![0.5; SR as usize * 2]]);
    write_wav(&t, SR, &[tone(SR, 2.0, 440.0, 0.5)]);
    let doc_at = |gain: f64| {
        project(
            30.0,
            2.0,
            &[
                audio("A", "dc", 0.0, 2.0, &format!(r#","gain":{gain}"#), ""),
                audio("B", "t", 0.0, 2.0, r#","gain":0"#, ""),
            ],
            &[],
            "",
        )
    };
    let rig = Rig::new(&[("dc", &dc), ("t", &t)]);
    let glide = (SR * RETUNE_GLIDE_MS / 1000) as usize;
    let options = RenderOptions {
        wait: true,
        ..Default::default()
    };
    let block = 256usize;
    let mut out = vec![Vec::new(), Vec::new()];

    // Level 1 → 0.5 at sample 1000: the DC ramps 0.5 → 0.25 over the glide.
    let mut m = rig.mixer(doc_at(1.0), "C", MixFormat::stereo(SR));
    let mut heard = Vec::new();
    let mut pos = 0i64;
    while pos < 4000 {
        if pos == 1024 {
            let d = doc_at(0.5);
            let g = Arc::new(rig.graph(&d, "C"));
            m.glide_to(Arc::new(d), g, None);
            assert!(m.is_gliding());
        }
        m.render(pos, block, &mut out, &options).unwrap();
        heard.extend_from_slice(&out[0]);
        pos += block as i64;
    }
    assert!(!m.is_gliding(), "the glide ends after {glide} samples");
    for (i, v) in heard.iter().enumerate() {
        let w = ((i as f64 - 1024.0) / glide as f64).clamp(0.0, 1.0);
        let want = 0.5 * (1.0 - 0.5 * w);
        assert!((*v as f64 - want).abs() < 1e-6, "sample {i}: {v} vs {want}");
    }

    // A fader dragged fast: a new edit about every 400 samples, so glides
    // overlap. Each glide fades the blend so far into the next mix, so with
    // two in flight the DC moves by at most three glide increments a sample
    // (w0'·(1 − w1) + w1'·(2 − w0)): continuous, never a step.
    let mut m = rig.mixer(doc_at(1.0), "C", MixFormat::stereo(SR));
    let mut heard = Vec::new();
    let mut pos = 0i64;
    let mut level = 1.0;
    let mut next_edit = 400i64;
    while pos < 12_000 {
        if pos >= next_edit && next_edit < 8_000 {
            level -= 0.04;
            let d = doc_at(level);
            let g = Arc::new(rig.graph(&d, "C"));
            m.glide_to(Arc::new(d), g, None);
            next_edit += 400;
        }
        m.render(pos, 160, &mut out, &options).unwrap();
        heard.extend_from_slice(&out[0]);
        pos += 160;
    }
    let max_step = heard
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0f32, f32::max);
    let increment = 0.5 * 0.04 / glide as f32;
    assert!(
        max_step <= increment * 3.0 + 1e-6,
        "largest step {max_step}, one glide increment {increment}"
    );
    let final_level = (0.5 * level) as f32;
    assert!((heard[heard.len() - 1] - final_level).abs() < 1e-6);

    // A monitor solo crossfades too; after it, the mix is the solo's.
    let mut m = rig.mixer(doc_at(1.0), "C", MixFormat::stereo(SR));
    m.render(0, block, &mut out, &options).unwrap();
    let (d, g) = (m.document().clone(), m.graph().clone());
    m.glide_to(d, g, None);
    let solo = RenderOptions {
        wait: true,
        monitor_solo: Some(["B".to_string()].into_iter().collect()),
        meters: false,
        ..Default::default()
    };
    m.render(256, 1024, &mut out, &solo).unwrap();
    assert!((out[0][0] - 0.5).abs() < 1e-6, "starts on the old mix");
    let mid = out[0][glide / 2];
    assert!((mid - 0.25).abs() < 1e-6, "halfway {mid}");
    assert!(
        out[0][glide + 10].abs() < 1e-6,
        "the soloed strip is silent: {}",
        out[0][glide + 10]
    );
    assert!(!m.is_gliding());

    // After any glide the mix equals the offline render of the new document.
    let mut m = rig.mixer(doc_at(1.0), "C", MixFormat::stereo(SR));
    let d = doc_at(0.3);
    let g = Arc::new(rig.graph(&d, "C"));
    m.render(0, block, &mut out, &options).unwrap();
    m.glide_to(Arc::new(d), g, None);
    for p in 1..8 {
        m.render(p * block as i64, block, &mut out, &options)
            .unwrap();
    }
    m.render(4096, 1024, &mut out, &options).unwrap();
    let mut fresh = rig.mixer(doc_at(0.3), "C", MixFormat::stereo(SR));
    let reference = rig.render(&mut fresh, 4096, 5120, 1024);
    assert_eq!(out[0], reference[0]);
    assert_eq!(out[1], reference[1]);

    // An immediate swap (paused, an export) never glides.
    m.glide_to(m.document().clone(), m.graph().clone(), None);
    let d = doc_at(1.0);
    let g = Arc::new(rig.graph(&d, "C"));
    m.set_graph(Arc::new(d), g);
    assert!(!m.is_gliding());
}
