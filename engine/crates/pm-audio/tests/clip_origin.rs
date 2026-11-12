//! A file's sound sits on its picture's timeline: clip time 0 is the first
//! presented video frame, and each audio stream is placed by its own
//! timestamps relative to that. A click authored at a known time against
//! the picture must come out of the mix at that time, whether the container
//! starts both streams late (transport streams), starts the sound after the
//! picture, or starts the picture after the sound.
//!
//! That is the reading of an asset with `timing: 2`. An asset without the
//! member is an older project's: its source time is the program sample, as
//! the earlier app's decoded buffer was addressed, wherever the container
//! starts the stream.

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
fn sound_is_placed_against_the_first_video_frame() {
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
    let cases: [(&str, f64, f64, &[&str], i64); 5] = [
        // Both streams start together, late: nothing moves, nothing is cut.
        ("together.ts", 0.0, 0.0, ts, 48),
        // The sound starts a quarter second after the picture.
        ("late-sound.ts", 0.0, 0.25, ts, 48),
        ("late-sound.mov", 0.0, 0.25, pcm_mov, 1),
        // The picture starts after the sound: the sound before it is not
        // part of the clip.
        ("late-picture.ts", 0.2, 0.0, ts, 48),
        ("late-picture.mov", 0.2, 0.0, pcm_mov, 1),
    ];
    for (name, picture_at, sound_at, codecs, tolerance) in cases {
        let path = dir.join(name);
        clip(&path, &wav, picture_at, sound_at, codecs);
        let lead = ((sound_at - picture_at) * SR as f64).round() as i64;
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
