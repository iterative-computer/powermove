//! A file's sound sits on its picture's timeline: clip time 0 is the first
//! presented video frame, and each audio stream is placed by its own
//! timestamps relative to that. A click authored at a known time against
//! the picture must come out of the mix at that time, whether the container
//! starts both streams late (transport streams), starts the sound after the
//! picture, or starts the picture after the sound.

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

/// Where the click lands in the mix of a video layer starting at 0 with
/// `trim`, in samples.
fn click_in_mix(path: &std::path::Path, trim: f64) -> (usize, f32) {
    let v = layer(
        "V",
        "video",
        0.0,
        2.0,
        &format!(r#"{{"asset":"v","trim":{trim},"speed":1,"embeddedAudio":true}}"#),
        "",
    );
    let doc = project(25.0, 2.0, &[v], &[], "");
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
            let (at, level) = click_in_mix(&path, trim);
            assert!(level.abs() > 0.2, "{name}: no click in the mix ({level})");
            assert!(
                (at as i64 - expected).abs() <= tolerance,
                "{name}, trim {trim}: the click is at sample {at}, {expected} expected",
            );
        }
    }
}
