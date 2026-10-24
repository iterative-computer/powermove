//! WebM's Opus packet timing and metadata must match the WebCodecs exporter.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use pm_export::{
    AlphaOutput, AudioSpec, Codec, ColorTags, Container, Dither, ExportSpec, Overwrite,
    WorkingEncoding, WorkingFrame, export,
};
use pm_time::Rate;
use serde_json::Value;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("pm-export-opus-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ffmpeg_tool(name: &str) -> PathBuf {
    let root = std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Library/Caches/powermove-engine/ffmpeg/out"))
        })
        .expect("PM_FFMPEG_DIR or HOME");
    let tool = root.join("bin").join(name);
    assert!(tool.is_file(), "{} is required", tool.display());
    tool
}

fn probe(file: &Path) -> Value {
    let output = Command::new(ffmpeg_tool("ffprobe"))
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_packets",
            "-of",
            "json",
        ])
        .arg(file)
        .output()
        .expect("ffprobe");
    assert!(
        output.status.success(),
        "ffprobe: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn opus_webm_keeps_webcodecs_packet_timing_and_untrimmed_tail() {
    let scratch = Scratch::new();
    let destination = scratch.0.join("opus.webm");
    let spec = ExportSpec {
        width: 32,
        height: 32,
        rate: Rate::FPS_24,
        frames: 24,
        codec: Codec::Vp9 {
            bitrate_bps: 1_000_000,
        },
        container: Container::Webm,
        tags: ColorTags::Rec709QuickTime,
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: Some(AudioSpec::default_for(Container::Webm, 48_000, 2)),
        timecode: None,
        destination,
        overwrite: Overwrite::Fail,
        mode: None,
    };
    let mut job = export(spec).unwrap();
    let rgba = [0.25_f32, 0.5, 0.75, 1.0].repeat(32 * 32);
    for frame in 0..24 {
        job.push_frame(&WorkingFrame {
            width: 32,
            height: 32,
            rgba: &rgba,
        })
        .unwrap();
        let pcm: Vec<f32> = (0..2000)
            .flat_map(|sample| {
                let t = (frame * 2000 + sample) as f32 / 48_000.0;
                [
                    (t * 440.0 * std::f32::consts::TAU).sin() * 0.2,
                    (t * 660.0 * std::f32::consts::TAU).sin() * 0.2,
                ]
            })
            .collect();
        job.push_audio(&pcm).unwrap();
    }
    let done = job.finish().unwrap();
    // WebCodecs' 312-sample lookahead prompts a 51st 20 ms packet. The legacy
    // muxer's OpusHead removes the pre-skip but has no DiscardPadding.
    assert_eq!(done.report.audio.as_ref().unwrap().samples, 48_648);

    // The current hand-written EBML has neither FFmpeg's mandatory Opus
    // SeekPreRoll nor CodecDelay in its track header.
    let ebml = std::fs::read(&done.path).unwrap();
    let cluster = ebml
        .windows(4)
        .position(|b| b == [0x1f, 0x43, 0xb6, 0x75])
        .unwrap();
    let tracks = &ebml[..cluster];
    assert!(
        !tracks.windows(2).any(|b| b == [0x56, 0xbb]),
        "unexpected SeekPreRoll"
    );
    assert!(
        !tracks.windows(2).any(|b| b == [0x56, 0xaa]),
        "unexpected CodecDelay"
    );

    let metadata = probe(&done.path);
    let streams = metadata["streams"].as_array().unwrap();
    let audio = streams.iter().find(|s| s["codec_name"] == "opus").unwrap();
    let video = streams.iter().find(|s| s["codec_name"] == "vp9").unwrap();
    assert_eq!(audio["initial_padding"], 0);
    assert!(video["color_primaries"].is_null());
    assert!(video["color_transfer"].is_null());
    assert_eq!(video["color_space"], "bt709");

    let audio_index = audio["index"].as_u64().unwrap();
    let packets: Vec<&Value> = metadata["packets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|packet| packet["stream_index"].as_u64() == Some(audio_index))
        .collect();
    assert_eq!(packets.len(), 51);
    assert_eq!(packets.first().unwrap()["pts"], 0);
    assert_eq!(packets.last().unwrap()["pts"], 1000);
    assert!(packets.last().unwrap()["side_data_list"].is_null());
    let coded_bytes: u64 = packets
        .iter()
        .map(|p| p["size"].as_str().unwrap().parse::<u64>().unwrap())
        .sum();
    assert!(
        (14_000..30_000).contains(&coded_bytes),
        "160 kbps total produced {coded_bytes} bytes"
    );

    let decoded = Command::new(ffmpeg_tool("ffmpeg"))
        .args(["-v", "error", "-i"])
        .arg(&done.path)
        .args(["-map", "0:a:0", "-f", "f32le", "-acodec", "pcm_f32le", "-"])
        .output()
        .expect("decode Opus");
    assert!(
        decoded.status.success(),
        "ffmpeg: {}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    assert_eq!(decoded.stdout.len() / (2 * size_of::<f32>()), 48_648);
}
