//! Runs the real encoders against the dev ffmpeg
//! (`apps/desktop/node_modules/ffmpeg-static/ffmpeg.exe`); skipped without it.

use std::path::{Path, PathBuf};

use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::convert::Converters;
use super::process::{self, Spawn};
use super::proxy::ProxyService;
use super::render::{EncoderOptions, RenderEncoder};

fn dev_ffmpeg() -> Option<PathBuf> {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/node_modules/ffmpeg-static").join(name);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("skipping: {} not found", path.display());
        None
    }
}

#[tokio::test]
async fn exports_frames_and_converts_clips_and_animations() {
    let Some(ffmpeg) = dev_ffmpeg() else { return };
    let temp = tempfile::tempdir().unwrap();

    // Export: four 32x16 RGBA frames streamed in two writes, plus a WAV.
    let encoder = RenderEncoder::new(ffmpeg.clone(), temp.path().into());
    let options = EncoderOptions::parse(&json!({ "width": 32, "height": 16, "fps": 24, "format": "mp4", "name": "test" })).unwrap();
    let token = encoder.start(options, 7).await.unwrap();
    let frame: Vec<u8> = (0..32 * 16).flat_map(|pixel| [(pixel % 256) as u8, 0x80, 0x20, 0xff]).collect();
    let two_frames = [frame.as_slice(), frame.as_slice()].concat();
    assert!(encoder.write(&token, 8, &two_frames, false).await.is_err(), "another window cannot write");
    encoder.write(&token, 7, &two_frames, false).await.unwrap();
    encoder.write(&token, 7, &two_frames, false).await.unwrap();
    encoder.write(&token, 7, &wav(48_000, 0.2), true).await.unwrap();
    let file = encoder.finish(&token, 7).await.unwrap();
    assert_eq!(file.file_name().unwrap(), "final.mp4");
    let bytes = std::fs::read(&file).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp");
    let job_dir = file.parent().unwrap().to_path_buf();
    encoder.release(&token, 7).await.unwrap();
    assert!(!job_dir.exists());

    // Proxy: generate a short clip, convert it, read it back.
    let clip = temp.path().join("clip.mkv");
    run(&ffmpeg, &["-f", "lavfi", "-i", "testsrc=duration=0.5:size=64x48:rate=10", "-c:v", "libx264", "-y"], &clip).await;
    let proxies = ProxyService::new(temp.path().into(), Converters { ffmpeg: ffmpeg.clone() });
    let (token, size) = proxies.create(&clip.to_string_lossy(), 7, &CancellationToken::new()).await.unwrap();
    assert!(size > 0);
    let head = proxies.read(&token, 7, 0, 4).await.unwrap();
    assert_eq!(head.as_ref(), [0x1a, 0x45, 0xdf, 0xa3], "the proxy is WebM");
    assert_eq!(proxies.read(&token, 7, size, 4).await.unwrap().len(), 0);
    proxies.release(&token).await;
    assert!(proxies.read(&token, 7, 0, 4).await.is_err());

    // Animation: two decoded frames held for 2 and 3 slots encode 5 frames.
    let png = temp.path().join("frame.png");
    run(&ffmpeg, &["-f", "lavfi", "-i", "color=c=red:size=16x16", "-frames:v", "1", "-y"], &png).await;
    let png = std::fs::read(&png).unwrap();
    let token = proxies.begin_animation(10.0, Some(vec![2, 3]), 7).await.unwrap();
    proxies.write_animation_frame(&token, 7, 0, 0, &png).await.unwrap();
    assert!(proxies.finish_animation(&token, 7, &CancellationToken::new(), |_| {}).await.is_err(), "frame 1 is missing");
    let token = proxies.begin_animation(10.0, Some(vec![2, 3]), 7).await.unwrap();
    proxies.write_animation_frame(&token, 7, 0, 0, &png).await.unwrap();
    proxies.write_animation_frame(&token, 7, 1, 0, &png).await.unwrap();
    let mut progress = Vec::new();
    let size = proxies.finish_animation(&token, 7, &CancellationToken::new(), |completed| progress.push(completed)).await.unwrap();
    assert!(size > 0);
    assert_eq!(progress.last(), Some(&5));
}

async fn run(ffmpeg: &Path, args: &[&str], output: &Path) {
    let args = ["-hide_banner", "-loglevel", "error"].iter().chain(args).map(|arg| arg.to_string());
    process::run(Spawn {
        binary: ffmpeg,
        args: args.chain([output.to_string_lossy().into_owned()]).collect(),
        operation: "test input",
        stdin: false,
        stdout: false,
        timeout: None,
        cancel: CancellationToken::new(),
    })
    .await
    .unwrap();
}

/// A mono 16-bit PCM WAV of silence.
fn wav(rate: u32, seconds: f32) -> Vec<u8> {
    let samples = (rate as f32 * seconds) as u32;
    let data_len = samples * 2;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(out.len() + data_len as usize, 0);
    out
}
