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

/// Drop just the output samples whose sinc support is missing. Keep the
/// ready prefix bit-exact, including inside a render crossing a cache block.
#[test]
fn dropped_input_preserves_ready_samples_at_a_block_boundary() {
    let dir = temp_dir("readiness-drop-boundary");
    for (source_rate, mapping) in [
        (48_000, ""),
        (44_100, ""),
        (48_000, r#", "speed":1.5, "pitch":"follow""#),
    ] {
        let file = dir.join(format!("{source_rate}.wav"));
        write_wav(&file, source_rate, &[tone(source_rate, 4.0, 997.0, 0.2)]);
        let rig = Rig::new(&[("s", &file)]);
        let doc = || {
            project(
                30.0,
                4.0,
                &[audio("A", "s", 0.0, 4.0, mapping, "")],
                &[],
                "",
            )
        };
        let mut played = rig.mixer(doc(), "C", MixFormat::stereo(48_000));
        let source = rig.cache.open(&file, None).unwrap();
        rig.cache.block(&source, 0, true).unwrap();
        let ratio = if mapping.is_empty() {
            source_rate as f64 / 48_000.0
        } else {
            1.5
        };
        let block = pm_audio::source::BLOCK;
        let start = (block as f64 / ratio) as i64 - 512;
        let frames = 1024;
        let mut out = vec![vec![]; 2];
        let options = RenderOptions {
            drop_starved: true,
            ..RenderOptions::default()
        };
        let report = played.render(start, frames, &mut out, &options).unwrap();
        assert_eq!(report.starved, 1);
        let mut reference = rig.mixer(doc(), "C", MixFormat::stereo(48_000));
        let expected = rig.render(&mut reference, start, start + frames as i64, frames);
        let bank = if mapping.is_empty() && source_rate == 48_000 {
            None
        } else if mapping.is_empty() {
            pm_audio::kernel::grid_bank(source_rate, 48_000)
        } else {
            Some(pm_audio::kernel::free_bank(ratio))
        };
        let mut kept = 0;
        let mut dropped = 0;
        for i in 0..frames {
            let position = (start + i as i64) as f64 * ratio;
            let ready = bank.as_ref().map_or(position < block as f64, |b| {
                position.floor() as usize + b.taps - b.before() <= block
            });
            for c in 0..2 {
                if ready {
                    assert_eq!(out[c][i], expected[c][i], "{source_rate} {mapping} at {i}");
                } else {
                    assert_eq!(out[c][i], 0.0, "missing sinc support at {i}");
                }
            }
            if ready {
                kept += 1;
            } else {
                dropped += 1;
            }
        }
        assert_eq!(report.dropped_voice_frames, dropped as u64);
        assert!(
            kept > 0 && dropped > 0,
            "both sides of the boundary are exercised"
        );
        let report = played
            .render(start + frames as i64, 256, &mut out, &options)
            .unwrap();
        assert_eq!(report.starved, 0);
        let expected = rig.render(
            &mut reference,
            start + frames as i64,
            start + frames as i64 + 256,
            256,
        );
        assert_eq!(out, expected, "re-entry through the same reader");
    }
}

#[test]
fn a_dropped_outgoing_voice_does_not_hold_the_retune_or_meters() {
    let dir = temp_dir("readiness-drop-glide");
    let file = dir.join("tone.wav");
    write_wav(&file, 48_000, &[tone(48_000, 2.0, 997.0, 0.2)]);
    let rig = Rig::new(&[("s", &file)]);
    let mut mixer = rig.mixer(
        project(30.0, 2.0, &[audio("late", "s", 0.0, 2.0, "", "")], &[], ""),
        "C",
        MixFormat::stereo(48_000),
    );
    let silent = project(30.0, 2.0, &[], &[], "");
    let graph = Arc::new(rig.graph(&silent, "C"));
    mixer.glide_to(Arc::new(silent), graph, None);
    let options = RenderOptions {
        drop_starved: true,
        meters: true,
        ..RenderOptions::default()
    };
    for at in [0, 256, 512] {
        let mut out = vec![vec![]; 2];
        let report = mixer.render(at, 256, &mut out, &options).unwrap();
        assert_eq!(report.starved_voices, ["late"]);
        assert!(out.iter().flatten().all(|&v| v == 0.0));
    }
    assert!(
        !mixer.is_gliding(),
        "the envelope advanced over committed silence"
    );
    assert!(
        mixer
            .meters(768)
            .master
            .iter()
            .all(|level| level.peak == 0.0)
    );
}

#[test]
fn offline_waits_past_the_playback_bound_without_dropping() {
    use std::time::{Duration, Instant};
    let dir = temp_dir("readiness-offline-wait");
    let file = dir.join("tone.wav");
    let samples = tone(48_000, 1.0, 997.0, 0.2);
    write_wav(&file, 48_000, std::slice::from_ref(&samples));
    let rig = Rig::new(&[("s", &file)]);
    let source = rig.cache.open(&file, None).unwrap();
    rig.cache.set_decode_blocked(&source, true);
    let mut mixer = rig.mixer(
        project(30.0, 1.0, &[audio("A", "s", 0.0, 1.0, "", "")], &[], ""),
        "C",
        MixFormat::stereo(48_000),
    );
    let cache = rig.cache.clone();
    let release = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while cache.blocked_decoders() == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(160));
        cache.set_decode_blocked(&source, false);
    });
    let started = Instant::now();
    let out = rig.render(&mut mixer, 0, 48_000, 256);
    release.join().unwrap();
    assert!(started.elapsed() >= Duration::from_millis(160));
    assert_eq!(out, [samples.clone(), samples]);
}

/// Dropping one pitch-preserving voice must not reset another voice's
/// vocoder. Recovery starts at the current sample with bounded pre-roll.
#[test]
fn dropped_stretch_preserves_other_voices_and_reenters_at_the_current_sample() {
    let dir = temp_dir("readiness-drop-stretch");
    let good_file = dir.join("good.wav");
    let late_file = dir.join("late.wav");
    let n = 8 * 48_000;
    write_wav(
        &good_file,
        48_000,
        &[tone(48_000, 8.0, 997.0, 0.2), vec![0.0; n]],
    );
    write_wav(
        &late_file,
        48_000,
        &[vec![0.0; n], tone(48_000, 8.0, 631.0, 0.1)],
    );
    let rig = Rig::new(&[("g", &good_file), ("s", &late_file)]);
    let good = rig.cache.open(&good_file, None).unwrap();
    for block in 0..(n / pm_audio::source::BLOCK + 1) {
        rig.cache.block(&good, block as u64, true).unwrap();
    }
    let clip = |id, asset| audio(id, asset, 0.0, 4.0, r#", "speed":1.5"#, "");
    let doc = |clips: &[String]| project(30.0, 4.0, clips, &[], "");
    let mut played = rig.mixer(
        doc(&[clip("good", "g"), clip("late", "s")]),
        "C",
        MixFormat::stereo(48_000),
    );
    let mut uninterrupted = rig.mixer(doc(&[clip("good", "g")]), "C", MixFormat::stereo(48_000));
    let mut out = vec![vec![]; 2];
    let mut expected = vec![vec![]; 2];
    let options = RenderOptions {
        drop_starved: true,
        ..RenderOptions::default()
    };
    let first = 48_000;
    let resume = first + 24_576; // Past a fresh-start grid, not just one chunk.
    for start in (first..resume).step_by(256) {
        let report = played.render(start, 256, &mut out, &options).unwrap();
        assert_eq!(report.starved_voices, ["late"]);
        uninterrupted
            .render(start, 256, &mut expected, &options)
            .unwrap();
        assert_eq!(out[0], expected[0], "other voice history at {start}");
        assert!(out[1].iter().all(|&v| v == 0.0));
    }
    let late = rig.cache.open(&late_file, None).unwrap();
    for block in 0..(n / pm_audio::source::BLOCK + 1) {
        rig.cache.block(&late, block as u64, true).unwrap();
    }
    let mut fresh = rig.mixer(doc(&[clip("late", "s")]), "C", MixFormat::stereo(48_000));
    let late_reference = rig.render(&mut fresh, resume, resume + 4_096, 256);
    for start in (resume..resume + 4_096).step_by(256) {
        let report = played.render(start, 256, &mut out, &options).unwrap();
        assert_eq!(report.starved, 0);
        uninterrupted
            .render(start, 256, &mut expected, &options)
            .unwrap();
        assert_eq!(out[0], expected[0], "other voice after recovery at {start}");
        let offset = (start - resume) as usize;
        assert_eq!(
            out[1],
            late_reference[1][offset..offset + 256],
            "current sample at {start}"
        );
    }
}

#[test]
fn a_memoized_decode_failure_reads_as_the_first() {
    let dir = temp_dir("memoized-failure");
    let file = dir.join("bad.wav");
    write_wav(&file, 48_000, &[vec![0.5; 4_096]]);
    let cache = pm_audio::SourceCache::new(64 << 20, 0);
    let source = cache.open(&file, None).unwrap();
    cache.set_decode_failing(&source, true);
    let first = cache.block(&source, 0, true).unwrap_err().to_string();
    let again = cache.block(&source, 0, true).unwrap_err().to_string();
    let nonblocking = cache.block(&source, 0, false).unwrap_err().to_string();
    assert_eq!(first, "media: decode: block 0: injected decode failure");
    assert_eq!(again, first);
    assert_eq!(nonblocking, first);
}
