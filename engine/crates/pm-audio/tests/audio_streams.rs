//! Every audio stream of a file can be heard. A camera or recorder file
//! with several mono streams lists them on its asset (`audioStreams`), and
//! the mix plays each listed stream that is on; an asset without the list
//! (a project made before streams were listed) plays the one stream it
//! always played.

mod common;

use common::*;
use pm_audio::MixFormat;
use pm_model::Document;
use pm_model::json::parse;

const SR: u32 = 48_000;
/// Stream `k` holds one click at this sample.
const CLICKS: [usize; 4] = [4_800, 14_400, 24_000, 33_600];

fn document(assets: &str, kind: &str) -> Document {
    let layer = match kind {
        "video" => layer(
            "V",
            "video",
            0.0,
            1.0,
            r#"{"asset":"m","trim":0,"speed":1,"embeddedAudio":true}"#,
            "",
        ),
        _ => audio("A", "m", 0.0, 1.0, "", ""),
    };
    let text = format!(
        r#"{{"id":"P","compId":"C","name":"T","w":320,"h":180,"fps":25,"dur":1,"layers":[{layer}],"comps":{{}},"assets":{assets},"params":{{}}}}"#
    );
    Document::new(Some(parse(&text).unwrap()), 1)
}

/// The clicks heard in the mix, as the streams they belong to.
fn heard(rig: &Rig, doc: Document) -> Vec<usize> {
    let mut mixer = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let out = rig.render(&mut mixer, 0, SR as i64, 4096);
    // A mono stream plays in the centre: the same in both channels.
    assert_eq!(out[0], out[1]);
    peaks(&out[0], 0.2)
        .into_iter()
        .map(|at| {
            CLICKS
                .iter()
                .position(|&c| c.abs_diff(at) <= 1)
                .unwrap_or_else(|| panic!("a click at {at} belongs to no stream"))
        })
        .collect()
}

#[test]
fn every_listed_stream_is_heard() {
    let dir = temp_dir("audio-streams");
    let mut command = std::process::Command::new(ffmpeg());
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    command.args([
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=64x64:rate=25:duration=1",
    ]);
    for (k, &at) in CLICKS.iter().enumerate() {
        let wav = dir.join(format!("{k}.wav"));
        write_wav(&wav, SR, &[clicks(SR as usize, &[at], 0.9)]);
        command.arg("-i").arg(wav);
    }
    // A recorder file: one picture and four mono tracks.
    let recorder = dir.join("recorder.mov");
    command.args([
        "-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:a", "-map", "4:a",
    ]);
    command.args([
        "-c:v",
        "mpeg4",
        "-q:v",
        "4",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "pcm_s16le",
    ]);
    assert!(command.arg(&recorder).status().unwrap().success());
    let rig = Rig::new(&[("m", &recorder)]);
    let entry = |index: usize, on: bool| {
        format!(r#"{{"index":{index},"channels":1,"sampleRate":48000,"on":{on}}}"#)
    };
    let listed = |on: [bool; 4]| {
        // The picture is stream 0; the audio streams follow it.
        let entries: Vec<String> = (0..4).map(|k| entry(k + 1, on[k])).collect();
        format!(
            r#"{{"m":{{"kind":"video","audioStreams":[{}]}}}}"#,
            entries.join(",")
        )
    };
    for kind in ["video", "audio"] {
        // Every stream on: all four are heard.
        assert_eq!(
            heard(&rig, document(&listed([true; 4]), kind)),
            [0, 1, 2, 3],
            "{kind}"
        );
        // Streams switched off are silent; the others stay.
        assert_eq!(
            heard(&rig, document(&listed([false, true, false, true]), kind)),
            [1, 3],
            "{kind}"
        );
        assert!(heard(&rig, document(&listed([false; 4]), kind)).is_empty());
        // A project made before streams were listed sounds as it did: the
        // file's one best stream.
        assert_eq!(
            heard(&rig, document(r#"{"m":{"kind":"video"}}"#, kind)),
            [0],
            "{kind}"
        );
    }
    // Each stream is its own voice on the layer's strip.
    let doc = document(&listed([true; 4]), "video");
    let graph = rig.graph(&doc, "C");
    let ids: Vec<&str> = graph.voices.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "V:embedded-audio",
            "V:embedded-audio:stream-2",
            "V:embedded-audio:stream-3",
            "V:embedded-audio:stream-4"
        ]
    );
    assert!(graph.voices.iter().all(|v| v.strip == "V"));
}

/// Each stream folds to the mix by its own layout: a silent surround
/// stream next to a mono one changes nothing a person hears.
#[test]
fn a_silent_surround_stream_does_not_change_a_mono_stream() {
    let dir = temp_dir("audio-streams-layouts");
    let wav = dir.join("click.wav");
    write_wav(&wav, SR, &[clicks(SR as usize, &[CLICKS[0]], 0.9)]);
    for layout in ["5.1", "quad"] {
        let file = dir.join(format!("{layout}.mov"));
        let status = std::process::Command::new(ffmpeg())
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&wav)
            .args(["-f", "lavfi", "-t", "1", "-i"])
            .arg(format!("anullsrc=r=48000:cl={layout}"))
            .args(["-map", "0:a", "-map", "1:a", "-c:a", "pcm_s16le"])
            .arg(&file)
            .status()
            .unwrap();
        assert!(status.success());
        let rig = Rig::new(&[("m", &file)]);
        let wide = if layout == "quad" { 4 } else { 6 };
        let level = |surround_on: bool| {
            let assets = format!(
                r#"{{"m":{{"kind":"audio","audioStreams":[{{"index":0,"channels":1,"sampleRate":48000,"on":true}},{{"index":1,"channels":{wide},"sampleRate":48000,"on":{surround_on}}}]}}}}"#
            );
            let mut mixer = rig.mixer(document(&assets, "audio"), "C", MixFormat::stereo(SR));
            let out = rig.render(&mut mixer, 0, SR as i64, 4096);
            [out[0][CLICKS[0]], out[1][CLICKS[0]]]
        };
        let alone = level(false);
        // Unity in both channels (16-bit samples).
        assert!(
            alone.iter().all(|x| (x - 0.9).abs() < 1e-4),
            "{layout}: the mono stream alone is {alone:?}"
        );
        assert_eq!(level(true), alone, "{layout}: with the silent stream on");
    }
    // A surround stream with something in it is heard through its own
    // layout: its centre channel at half power in both outputs, next to the
    // mono stream at unity.
    let surround = dir.join("surround.wav");
    let mut channels = vec![vec![0.0; SR as usize]; 6];
    channels[2] = clicks(SR as usize, &[CLICKS[1]], 0.9);
    write_wav(&surround, SR, &channels);
    let file = dir.join("centre.mov");
    let status = std::process::Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&wav)
        .arg("-i")
        .arg(&surround)
        .args(["-map", "0:a", "-map", "1:a", "-c:a", "pcm_s16le"])
        .arg(&file)
        .status()
        .unwrap();
    assert!(status.success());
    let rig = Rig::new(&[("m", &file)]);
    let assets = r#"{"m":{"kind":"audio","audioStreams":[{"index":0,"channels":1,"sampleRate":48000,"on":true},{"index":1,"channels":6,"sampleRate":48000,"on":true}]}}"#;
    let mut mixer = rig.mixer(document(assets, "audio"), "C", MixFormat::stereo(SR));
    let out = rig.render(&mut mixer, 0, SR as i64, 4096);
    for side in &out[..2] {
        assert!((side[CLICKS[0]] - 0.9).abs() < 1e-4, "{}", side[CLICKS[0]]);
        let centre = 0.9 * 0.5f32.sqrt();
        assert!(
            (side[CLICKS[1]] - centre).abs() < 1e-4,
            "{}",
            side[CLICKS[1]]
        );
    }
}
