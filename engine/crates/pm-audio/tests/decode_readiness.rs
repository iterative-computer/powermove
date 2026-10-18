//! Nonblocking source misses must be reported, and must not change later
//! decoded audio. Exercise the readers and retune's outgoing mixes directly
//! so worker scheduling cannot hide the missing-read regression.
mod common;

use common::*;
use pm_audio::MixFormat;
use pm_audio::mixer::RenderOptions;
use std::sync::Arc;

#[test]
fn readers_recover_exactly_after_an_undecoded_block() {
    let dir = temp_dir("readiness");
    let file = dir.join("stereo.wav");
    write_wav(
        &file,
        44_100,
        &[tone(44_100, 8.0, 997.0, 0.2), tone(44_100, 8.0, 631.0, 0.1)],
    );
    let remap = keys(&[(0.0, 5.0), (4.0, 0.5)]);
    let speed = keys(&[(0.0, 0.75), (4.0, 1.5)]);
    for pitch in ["preserve", "follow"] {
        for mapping in [
            r#""speed":1.5"#.to_string(),
            format!(r#""speed":{speed}"#),
            format!(r#""timeRemap":true,"sourceTime":{remap}"#),
            r#""trim":0.25"#.to_string(),
        ] {
            let rig = Rig::new(&[("s", &file)]);
            let doc = || {
                project(
                    30.0,
                    4.0,
                    &[audio(
                        "A",
                        "s",
                        0.0,
                        4.0,
                        &format!(r#",{mapping},"pitch":"{pitch}""#),
                        "",
                    )],
                    &[],
                    "",
                )
            };
            let mut played = rig.mixer(doc(), "C", MixFormat::stereo(48_000));
            let mut out = vec![vec![]; 2];
            let options = RenderOptions::default();
            let report = played.render(48_000, 256, &mut out, &options).unwrap();
            assert_eq!(report.starved, 1, "{pitch} {mapping}");
            assert!(report.errors.is_empty());
            assert!(out.iter().flatten().all(|&v| v == 0.0));
            // Filling the cache through an independent offline mix lets the
            // suspended stretch resume the same history, including the
            // unheard, missing chunk, rather than starting with new phases.
            let mut reference = rig.mixer(doc(), "C", MixFormat::stereo(48_000));
            let expected = rig.render(&mut reference, 48_000, 56_192, 4096);
            for start in (48_256..56_192).step_by(256) {
                let report = played.render(start, 256, &mut out, &options).unwrap();
                assert_eq!(report.starved, 0, "{pitch} {mapping} at {start}");
                assert!(report.errors.is_empty());
                let offset = (start - 48_000) as usize;
                for c in 0..2 {
                    assert_eq!(
                        out[c],
                        expected[c][offset..offset + 256],
                        "{pitch} {mapping} at {start}, channel {c}"
                    );
                }
            }
        }
    }
}

#[test]
fn outgoing_retune_misses_are_reported() {
    let dir = temp_dir("readiness-glide");
    let file = dir.join("tone.wav");
    write_wav(&file, 48_000, &[tone(48_000, 2.0, 997.0, 0.2)]);
    let rig = Rig::new(&[("s", &file)]);
    let mut mixer = rig.mixer(
        project(
            30.0,
            2.0,
            &[audio("A", "s", 0.0, 2.0, r#", "speed":1.5"#, "")],
            &[],
            "",
        ),
        "C",
        MixFormat::stereo(48_000),
    );
    let silent = project(30.0, 2.0, &[], &[], "");
    let graph = Arc::new(rig.graph(&silent, "C"));
    mixer.glide_to(Arc::new(silent), graph, None);
    let report = mixer
        .render(48_000, 256, &mut vec![vec![]; 2], &RenderOptions::default())
        .unwrap();
    assert_eq!(
        report.starved, 1,
        "a missing outgoing voice is still heard in the glide"
    );
}

#[test]
fn stretch_preroll_prefetch_includes_zero_gain_history() {
    let dir = temp_dir("readiness-muted-history");
    let file = dir.join("tone.wav");
    write_wav(&file, 48_000, &[tone(48_000, 8.0, 997.0, 0.2)]);
    let rig = Rig::new(&[("s", &file)]);
    let gain = keys(&[(0.0, 0.0), (2.0, 0.0), (2.01, 1.0)]);
    let clip = layer(
        "A",
        "audio",
        0.0,
        4.0,
        &format!(r#"{{"asset":"s","gain":{gain},"trim":0,"speed":2}}"#),
        "",
    );
    let mut mixer = rig.mixer(
        project(30.0, 4.0, &[clip], &[], ""),
        "C",
        MixFormat::stereo(48_000),
    );
    let start = 97_440;
    for (source, lo, hi) in mixer.source_spans(start, 256) {
        let len = (hi - lo) as usize;
        let mut scratch = vec![0.0; len];
        rig.cache
            .read(&source, lo, len, &mut [&mut scratch], true)
            .unwrap();
    }
    assert!(mixer.is_ready(start, 256));
    let report = mixer
        .render(start, 256, &mut vec![vec![]; 2], &RenderOptions::default())
        .unwrap();
    assert_eq!(
        report.starved, 0,
        "readiness includes the muted part of the vocoder's history"
    );
    assert!(report.errors.is_empty());
}

/// Playback retries a starved render; the meters take each block once,
/// when it renders completely, so a retried block reads as if it had never
/// missed (no silent block in the window).
#[test]
fn a_retried_render_meters_once() {
    let dir = temp_dir("readiness-meters");
    let file = dir.join("tone.wav");
    let rate = 48_000;
    // A rising level, so a silent block in the window changes the reading.
    let swell: Vec<f32> = tone(rate, 4.0, 997.0, 0.2)
        .iter()
        .enumerate()
        .map(|(i, v)| v * (i as f32 / (4 * rate) as f32))
        .collect();
    write_wav(&file, rate, &[swell]);
    let doc = || project(30.0, 4.0, &[audio("A", "s", 0.0, 4.0, "", "")], &[], "");
    let at = pm_audio::source::BLOCK as i64;
    let metered = |wait: bool| RenderOptions {
        wait,
        meters: true,
        ..RenderOptions::default()
    };
    let mut out = vec![vec![]; 2];

    let rig = Rig::new(&[("s", &file)]);
    let mut played = rig.mixer(doc(), "C", MixFormat::stereo(rate));
    for start in (at - 2048..at).step_by(256) {
        let report = played.render(start, 256, &mut out, &metered(true)).unwrap();
        assert_eq!(report.starved, 0);
    }
    let report = played.render(at, 256, &mut out, &metered(false)).unwrap();
    assert_eq!(report.starved, 1, "the next decode block is cold");
    let report = played.render(at, 256, &mut out, &metered(true)).unwrap();
    assert_eq!(report.starved, 0, "the retry completes");

    let rig = Rig::new(&[("s", &file)]);
    let mut reference = rig.mixer(doc(), "C", MixFormat::stereo(rate));
    for start in (at - 2048..=at).step_by(256) {
        reference
            .render(start, 256, &mut out, &metered(true))
            .unwrap();
    }
    assert_eq!(played.meters(at + 256), reference.meters(at + 256));
}
