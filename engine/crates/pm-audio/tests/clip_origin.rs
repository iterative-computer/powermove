//! Where a file's sound sits on its clip's timeline.
//!
//! An asset with `timing: 2` reads the file by its presentation: clip time
//! 0 is the earliest presented sample of any stream, and each stream keeps
//! its offset from it (a late soundtrack is silent until it starts; a late
//! picture shows nothing until its first frame). One exception: a picture
//! starting within one frame after that is the origin, so a clip never
//! opens on a blank frame because the sound leads by milliseconds, and the
//! sound before it is not part of the clip. A click authored at a known
//! time must come out of the mix at that time.
//!
//! An asset without the member is an older project's: its source time is
//! the program sample, as the earlier app's decoded buffer was addressed,
//! wherever the container starts the stream.

mod common;

use common::*;
use pm_audio::MixFormat;

const SR: u32 = 48_000;
/// The click, in samples from the start of the authored soundtrack.
const CLICK: usize = 12_000;

/// One second of 25 fps picture with a one-second soundtrack holding one
/// click, muxed with the picture or the sound shifted by `-itsoffset`.
fn clip(
    path: &std::path::Path,
    wav: &std::path::Path,
    picture_at: f64,
    sound_at: f64,
    codecs: &[&str],
) {
    let status = std::process::Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-itsoffset", &picture_at.to_string()])
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x64:rate=25:duration=1",
        ])
        .args(["-itsoffset", &sound_at.to_string()])
        .arg("-i")
        .arg(wav)
        .args(["-map", "0:v", "-map", "1:a", "-pix_fmt", "yuv420p"])
        .args(codecs)
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg failed for {}", path.display());
}

/// Where the click lands in the mix of a video layer starting at 0 whose
/// content is `d` (trim, speed or remap keys), in samples. `timing` is the
/// asset record's member.
fn click_with(path: &std::path::Path, d: &str, timing: Option<u64>) -> (usize, f32) {
    let v = layer(
        "V",
        "video",
        0.0,
        2.0,
        &format!(r#"{{"asset":"v",{d},"embeddedAudio":true}}"#),
        "",
    );
    let timing = timing.map_or(String::new(), |t| format!(r#","timing":{t}"#));
    let text = format!(
        r#"{{"id":"P","compId":"C","name":"T","w":320,"h":180,"fps":25,"dur":2,"layers":[{v}],"comps":{{}},"assets":{{"v":{{"id":"v","kind":"video"{timing}}}}},"params":{{}}}}"#
    );
    let doc = pm_model::Document::new(Some(pm_model::json::parse(&text).unwrap()), 1);
    let rig = Rig::new(&[("v", path)]);
    let mut mixer = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut mixer, 0, 2 * SR as i64, 4096);
    out[0]
        .iter()
        .copied()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
}

#[test]
fn sound_is_placed_against_the_start_of_the_presentation() {
    let dir = temp_dir("clip-origin");
    let wav = dir.join("click.wav");
    let click = clicks(SR as usize, &[CLICK], 0.9);
    write_wav(&wav, SR, &[click.clone(), click]);
    let pcm_mov: &[&str] = &["-c:v", "mpeg4", "-q:v", "4", "-c:a", "pcm_s16le"];
    let ts: &[&str] = &[
        "-c:v",
        "mpeg2video",
        "-q:v",
        "4",
        "-c:a",
        "mp2",
        "-b:a",
        "384k",
        "-f",
        "mpegts",
    ];
    // (file, picture start, sound start, codecs, tolerance in samples)
    let cases: [(&str, f64, f64, &[&str], i64); 6] = [
        // Both streams start together, late: nothing moves, nothing is cut.
        ("together.ts", 0.0, 0.0, ts, 48),
        // The sound starts a quarter second after the picture.
        ("late-sound.ts", 0.0, 0.25, ts, 48),
        ("late-sound.mov", 0.0, 0.25, pcm_mov, 1),
        // The picture starts five frames after the sound: the sound before
        // it is part of the clip, as authored.
        ("late-picture.ts", 0.2, 0.0, ts, 48),
        ("late-picture.mov", 0.2, 0.0, pcm_mov, 1),
        // The picture starts no more than a frame after the sound: the clip
        // opens on its first frame and the sound before it is dropped.
        ("picture-one-frame-late.mov", 0.04, 0.0, pcm_mov, 1),
    ];
    for (name, picture_at, sound_at, codecs, tolerance) in cases {
        let path = dir.join(name);
        clip(&path, &wav, picture_at, sound_at, codecs);
        // A transport stream stamps MPEG audio from the start of its
        // encoder's 481 samples of padding, which are presented.
        let padding = if name.ends_with(".ts") { 481.0 } else { 0.0 };
        let sound_starts = sound_at - padding / SR as f64;
        let origin = if (0.0..=0.04).contains(&(picture_at - sound_starts)) {
            picture_at
        } else {
            f64::min(picture_at, sound_starts)
        };
        let lead = ((sound_at - origin) * SR as f64).round() as i64;
        for trim in [0.0, 0.04] {
            let expected = CLICK as i64 + lead - (trim * SR as f64).round() as i64;
            let (at, level) = click_with(
                &path,
                &format!(r#""trim":{trim},"speed":1"#),
                Some(pm_media::clip::ASSET_TIMING),
            );
            assert!(level.abs() > 0.2, "{name}: no click in the mix ({level})");
            assert!(
                (at as i64 - expected).abs() <= tolerance,
                "{name}, trim {trim}: the click is at sample {at}, {expected} expected",
            );
        }
    }
}

/// An older project's trim, speed change and remap keys keep the sound they
/// were made with: source time t is program sample t of the stream, with
/// no offset for where the container starts it (measured in the earlier
/// app's decoder: a soundtrack delayed 0.25 s still gives its click at the
/// click's own program position).
#[test]
fn an_older_project_hears_the_program_sample_its_edits_address() {
    let dir = temp_dir("clip-origin-legacy");
    let wav = dir.join("click.wav");
    let click = clicks(SR as usize, &[CLICK], 0.9);
    write_wav(&wav, SR, &[click.clone(), click]);
    let pcm_mov: &[&str] = &["-c:v", "mpeg4", "-q:v", "4", "-c:a", "pcm_s16le"];
    let ts: &[&str] = &[
        "-c:v",
        "mpeg2video",
        "-q:v",
        "4",
        "-c:a",
        "mp2",
        "-b:a",
        "384k",
        "-f",
        "mpegts",
    ];
    // MPEG audio's decoder delay (481 samples) is part of the decoded
    // program: no timestamp is consulted to remove it.
    // (file, picture start, sound start, codecs, the click's program
    // sample, tolerance in samples)
    type Case = (&'static str, f64, f64, &'static [&'static str], usize, i64);
    let cases: [Case; 5] = [
        ("zero.mov", 0.0, 0.0, pcm_mov, CLICK, 1),
        ("late-sound.mov", 0.0, 0.25, pcm_mov, CLICK, 1),
        ("late-picture.mov", 0.2, 0.0, pcm_mov, CLICK, 1),
        ("late-sound.ts", 0.0, 0.25, ts, CLICK + 481, 2),
        ("late-picture.ts", 0.2, 0.0, ts, CLICK + 481, 2),
    ];
    for (name, picture_at, sound_at, codecs, program_click, tolerance) in cases {
        let click = program_click as f64 / SR as f64;
        let path = dir.join(name);
        clip(&path, &wav, picture_at, sound_at, codecs);
        // (edit, the mix time at which source time reaches the click)
        let edits = [
            (r#""trim":0,"speed":1"#.to_owned(), click),
            (r#""trim":0.04,"speed":1"#.to_owned(), click - 0.04),
            (r#""trim":0.05,"speed":2"#.to_owned(), (click - 0.05) / 2.0),
            // Remap keys: source time 0.1 at 0 to 0.6 at 1 (half speed).
            (
                format!(
                    r#""timeRemap":true,"sourceTime":{}"#,
                    keys(&[(0.0, 0.1), (1.0, 0.6)])
                ),
                (click - 0.1) / 0.5,
            ),
        ];
        for (edit, at_seconds) in edits {
            let expected = (at_seconds * SR as f64).round() as i64;
            let (at, level) = click_with(&path, &edit, None);
            assert!(level.abs() > 0.1, "{name}, {edit}: no click ({level})");
            // A retimed click is a resampled pulse: its peak may sit a few
            // samples either side.
            let slack = if edit.contains("speed\":1") { 0 } else { 96 };
            assert!(
                (at as i64 - expected).abs() <= tolerance + slack,
                "{name}, {edit}: the click is at sample {at}, {expected} expected",
            );
        }
    }
}

/// A file without a picture: each stream keeps its offset from the
/// earliest one. A single stream is its own origin.
#[test]
fn streams_of_a_file_without_a_picture_keep_their_offsets() {
    let dir = temp_dir("clip-origin-audio-only");
    let (first, second) = (dir.join("first.wav"), dir.join("second.wav"));
    write_wav(&first, SR, &[clicks(SR as usize, &[CLICK], 0.9)]);
    write_wav(&second, SR, &[clicks(SR as usize, &[30_000], 0.9)]);
    let file = dir.join("two.mka");
    let status = std::process::Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&first)
        .args(["-itsoffset", "0.25", "-i"])
        .arg(&second)
        .args(["-map", "0:a", "-map", "1:a", "-c:a", "pcm_s16le"])
        .arg(&file)
        .status()
        .unwrap();
    assert!(status.success());
    let mix = |assets: &str, path: &std::path::Path| {
        let a = audio("A", "v", 0.0, 2.0, "", "");
        let text = format!(
            r#"{{"id":"P","compId":"C","name":"T","w":320,"h":180,"fps":25,"dur":2,"layers":[{a}],"comps":{{}},"assets":{{"v":{assets}}},"params":{{}}}}"#
        );
        let doc = pm_model::Document::new(Some(pm_model::json::parse(&text).unwrap()), 1);
        let rig = Rig::new(&[("v", path)]);
        let mut mixer = rig.mixer(doc, "C", MixFormat::stereo(SR));
        rig.render(&mut mixer, 0, 2 * SR as i64, 4096).remove(0)
    };
    let heard = |out: &[f32]| -> Vec<usize> {
        (1..out.len())
            .filter(|&i| out[i].abs() > 0.3 && out[i - 1].abs() <= 0.3)
            .collect()
    };
    let streams = r#""audioStreams":[{"index":0,"channels":1,"sampleRate":48000,"on":true},{"index":1,"channels":1,"sampleRate":48000,"on":true}]"#;
    let out = mix(
        &format!(r#"{{"id":"v","kind":"audio","timing":2,{streams}}}"#),
        &file,
    );
    assert_eq!(
        heard(&out),
        [CLICK, 30_000 + 12_000],
        "two streams, the second 0.25 s late"
    );
    // One stream alone is at zero, with the member or without.
    for timing in ["", r#","timing":2"#] {
        let out = mix(&format!(r#"{{"id":"v","kind":"audio"{timing}}}"#), &second);
        assert_eq!(heard(&out), [30_000], "one stream{timing}");
    }
}

/// A fade out on a retimed clip ends where the clip's sound ends on the
/// clip's own timeline, wherever the container starts the soundtrack: a
/// late soundtrack is not silenced early, and one that leads the picture
/// is not cut off at full fade.
#[test]
fn a_retimed_fade_out_ends_where_the_sound_ends() {
    let dir = temp_dir("clip-origin-fade");
    let wav = dir.join("tone.wav");
    write_wav(&wav, SR, &[tone(SR, 1.0, 1000.0, 0.5)]);
    let pcm_mov: &[&str] = &["-c:v", "mpeg4", "-q:v", "4", "-c:a", "pcm_s16le"];
    // (file, picture start, sound start, where the sound starts on the clip)
    for (name, picture_at, sound_at, lead) in [
        ("late-sound.mov", 0.0, 0.25, 0.25),
        // The picture a frame late is the origin: the sound leads it.
        ("early-sound.mov", 0.04, 0.0, -0.04),
    ] {
        let path = dir.join(name);
        clip(&path, &wav, picture_at, sound_at, pcm_mov);
        let a = layer(
            "A",
            "audio",
            0.0,
            2.0,
            r#"{"asset":"v","gain":1,"trim":0,"speed":2,"pitch":"follow","fadeIn":0,"fadeOut":0.2}"#,
            "",
        );
        let text = format!(
            r#"{{"id":"P","compId":"C","name":"T","w":320,"h":180,"fps":25,"dur":2,"layers":[{a}],"comps":{{}},"assets":{{"v":{{"id":"v","kind":"video","timing":2}}}},"params":{{}}}}"#
        );
        let doc = pm_model::Document::new(Some(pm_model::json::parse(&text).unwrap()), 1);
        let rig = Rig::new(&[("v", &path)]);
        // The clip's second of sound at double speed ends here.
        let end = (lead + 1.0) / 2.0;
        // An export's blocks and a device callback's.
        for block in [4096, 128] {
            let mut mixer = rig.mixer(doc.clone(), "C", MixFormat::stereo(SR));
            let out = rig.render(&mut mixer, 0, SR as i64, block);
            let level = |at: f64| {
                let from = (at * SR as f64) as usize;
                out[0][from..from + 96]
                    .iter()
                    .fold(0f32, |peak, x| peak.max(x.abs()))
            };
            for before in [0.15, 0.1, 0.05, 0.02] {
                let expected = 0.5 * (before / 0.2f32).min(1.0);
                let heard = level(end - f64::from(before));
                assert!(
                    (heard - expected).abs() < 0.03,
                    "{name}, block {block}: {heard} at {before} s before the end, {expected} expected"
                );
            }
            assert!(
                level(end + 0.01) < 0.01,
                "{name}, block {block}: sound after the end"
            );
        }
    }
}

/// A clip without a picture is as long as its sound, as each reading
/// places it: the length an import records ends at the last sample the mix
/// plays, for two streams starting 1 and 1.25 s into the file.
#[test]
fn a_clip_without_a_picture_is_as_long_as_its_sound() {
    let dir = temp_dir("clip-origin-audio-length");
    let wav = dir.join("tone.wav");
    write_wav(&wav, SR, &[tone(SR, 1.0, 1000.0, 0.5)]);
    let file = dir.join("two.mov");
    let status = std::process::Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-itsoffset", "1", "-i"])
        .arg(&wav)
        .args(["-itsoffset", "1.25", "-i"])
        .arg(&wav)
        .args(["-map", "0:a", "-map", "1:a", "-c:a", "pcm_s16le"])
        .arg(&file)
        .status()
        .unwrap();
    assert!(status.success());
    let streams = r#""audioStreams":[{"index":0,"channels":1,"sampleRate":48000,"on":true},{"index":1,"channels":1,"sampleRate":48000,"on":true}]"#;
    for (timing, member) in [
        (pm_media::clip::ClipTiming::Presentation, r#","timing":2"#),
        (pm_media::clip::ClipTiming::Legacy, ""),
    ] {
        let a = audio("A", "v", 0.0, 3.0, "", "");
        let text = format!(
            r#"{{"id":"P","compId":"C","name":"T","w":320,"h":180,"fps":25,"dur":3,"layers":[{a}],"comps":{{}},"assets":{{"v":{{"id":"v","kind":"audio"{member},{streams}}}}},"params":{{}}}}"#
        );
        let doc = pm_model::Document::new(Some(pm_model::json::parse(&text).unwrap()), 1);
        let rig = Rig::new(&[("v", &file)]);
        let mut mixer = rig.mixer(doc, "C", MixFormat::stereo(SR));
        let out = rig.render(&mut mixer, 0, 3 * SR as i64, 4096).remove(0);
        let last = out.iter().rposition(|v| v.abs() > 1e-3).unwrap() + 1;
        let facts = pm_media::facts::media_facts_as(&file, None, timing).unwrap();
        let length = facts.duration.unwrap() * f64::from(SR);
        assert!(
            (length - last as f64).abs() <= 48.0,
            "{timing:?}: imported as {length} samples, the mix plays {last}"
        );
    }
}
