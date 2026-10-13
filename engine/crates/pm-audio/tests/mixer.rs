//! The mixer against the section 2 and 25 audio bars and the legacy level
//! rules, on generated media: clicks land on their frames, exported sample
//! counts are exact, renders are identical whatever block or start, gains
//! and fades follow `gainAt`, precompositions nest, solo and mute silence,
//! speed and remaps are followed, layouts fold.

mod common;

use common::*;
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
                r#"{{"asset":"a48","trim":0.1,"speed":{},"embeddedAudio":true,"audioGain":0.8}}"#,
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
    // Pitch follows speed: a 1 kHz tone at speed 1.5 sounds at 1.5 kHz.
    let tone_file = dir.join("tone.wav");
    write_wav(&tone_file, SR, &[tone(SR, 4.0, 1000.0, 0.5)]);
    let v = layer(
        "V",
        "video",
        0.0,
        2.0,
        r#"{"asset":"t","speed":1.5,"embeddedAudio":true}"#,
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
