//! The ffmpeg command lines behind playback proxies, previews, sequences,
//! and still conversions (the converters in `media-proxy.ts`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::process::{self, Spawn};

const TRANSCODE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const STILL_TIMEOUT: Duration = Duration::from_secs(2 * 60);

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Chromium decodes VP9 alpha; H.264 proxies flatten ProRes 4444 cutouts.
/// Keep full dimensions, timing, and optional audio, with bounded workers.
pub fn playback_args(source: &Path, output: &Path) -> Vec<String> {
    let mut args = strings(&["-hide_banner", "-loglevel", "error", "-nostdin", "-i"]);
    args.push(path_arg(source));
    args.extend(strings(&[
        "-map", "0:v:0", "-map", "0:a:0?",
        "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "0", "-crf", "18",
        // Bound random seeks to half a second of decode at 30 fps. Long GOPs
        // make timeline scrubbing and frame-by-frame exports decode seconds repeatedly.
        "-g", "15", "-deadline", "good", "-cpu-used", "4", "-row-mt", "1", "-threads", "4",
        "-c:a", "libopus", "-b:a", "192k", "-f", "webm", "-y",
    ]));
    args.push(path_arg(output));
    args
}

/// All-intra preview frames avoid decoding a long 4K GOP for every arrow press.
pub fn preview_args(source: &Path, output: &Path) -> Vec<String> {
    let mut args = strings(&["-hide_banner", "-loglevel", "error", "-nostdin"]);
    // libvpx preserves WebM alpha; the native VP9 decoder discards it.
    if source.extension().is_some_and(|extension| extension == "webm") {
        args.extend(strings(&["-c:v", "libvpx-vp9"]));
    }
    args.push("-i".into());
    args.push(path_arg(source));
    args.extend(strings(&[
        "-map", "0:v:0", "-an",
        "-vf", "scale=w='min(1280,iw)':h='min(1280,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2",
        "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "0", "-crf", "18",
        "-g", "1", "-deadline", "realtime", "-cpu-used", "6", "-row-mt", "1", "-threads", "4",
        "-f", "webm", "-y",
    ]));
    args.push(path_arg(output));
    args
}

pub fn sequence_args(pattern: &Path, fps: f64, count: u64, output: &Path) -> Vec<String> {
    let mut args = strings(&[
        "-hide_banner", "-loglevel", "error", "-nostdin",
        "-progress", "pipe:1", "-nostats", "-stats_period", "1",
        "-framerate",
    ]);
    args.push(fps.to_string());
    args.extend(strings(&["-start_number", "0", "-i"]));
    args.push(path_arg(pattern));
    args.push("-frames:v".into());
    args.push(count.to_string());
    args.extend(strings(&[
        "-an", "-c:v", "libvpx-vp9",
        "-pix_fmt", "yuva420p", "-lossless", "1", "-b:v", "0",
        "-g", "15", "-deadline", "good", "-cpu-used", "4", "-row-mt", "1", "-threads", "4",
        "-f", "webm", "-y",
    ]));
    args.push(path_arg(output));
    args
}

pub fn still_args(source: &Path, output: &Path) -> Vec<String> {
    let mut args = strings(&["-hide_banner", "-loglevel", "error", "-nostdin", "-i"]);
    args.push(path_arg(source));
    args.extend(strings(&["-frames:v", "1", "-update", "1", "-c:v", "png", "-f", "image2", "-y"]));
    args.push(path_arg(output));
    args
}

/// FFmpeg can split its line-oriented progress records across stdout reads;
/// this reports each new `frame=N`, capped at `count`.
pub struct ProgressReader {
    count: u64,
    previous: Option<u64>,
}

impl ProgressReader {
    pub fn new(count: u64) -> Self {
        Self { count, previous: None }
    }

    pub fn line(&mut self, line: &str) -> Option<u64> {
        let value = line.strip_prefix("frame=")?.trim();
        let completed = value.parse::<u64>().ok()?.min(self.count);
        if self.previous.is_some_and(|previous| completed <= previous) {
            return None;
        }
        self.previous = Some(completed);
        Some(completed)
    }
}

#[derive(Clone)]
pub struct Converters {
    pub ffmpeg: PathBuf,
}

impl Converters {
    fn spawn<'a>(&'a self, args: Vec<String>, operation: &'a str, timeout: (Duration, &'static str), cancel: &CancellationToken) -> Spawn<'a> {
        Spawn {
            binary: &self.ffmpeg,
            args,
            operation,
            stdin: false,
            stdout: false,
            timeout: Some(timeout),
            cancel: cancel.clone(),
        }
    }

    pub async fn playback(&self, source: &Path, output: &Path, cancel: &CancellationToken) -> Result<()> {
        process::run(self.spawn(playback_args(source, output), "video converter", (TRANSCODE_TIMEOUT, "Video conversion timed out."), cancel)).await
    }

    pub async fn preview(&self, source: &Path, output: &Path, cancel: &CancellationToken) -> Result<()> {
        process::run(self.spawn(preview_args(source, output), "preview converter", (TRANSCODE_TIMEOUT, "Preview conversion timed out."), cancel)).await
    }

    pub async fn sequence(
        &self,
        pattern: &Path,
        fps: f64,
        count: u64,
        output: &Path,
        cancel: &CancellationToken,
        mut on_progress: impl FnMut(u64) + Send,
    ) -> Result<()> {
        let mut options = self.spawn(
            sequence_args(pattern, fps, count, output),
            "image sequence encoder",
            (TRANSCODE_TIMEOUT, "Image sequence conversion timed out."),
            cancel,
        );
        options.stdout = true;
        let (process, _, stdout) = process::spawn(options)?;
        let mut reader = ProgressReader::new(count);
        let mut lines = BufReader::new(stdout.expect("stdout is piped")).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(completed) = reader.line(&line) {
                on_progress(completed);
            }
        }
        process.wait().await
    }

    /// Chromium decodes neither TIFF nor HEIF, so those stills become PNG on import.
    pub async fn still(&self, source: &Path, extension: &str, output: &Path, cancel: &CancellationToken) -> Result<()> {
        let result =
            process::run(self.spawn(still_args(source, output), "image converter", (STILL_TIMEOUT, "Image conversion timed out."), cancel)).await;
        // macOS used sips here. FFmpeg 6 opens only some HEIF variants, and
        // Windows has no built-in command-line decoder to fall back to.
        match result {
            Err(error) if extension == "heic" || extension == "heif" => {
                Err(anyhow!("this HEIC/HEIF image cannot be decoded on Windows; export it as JPEG or PNG first ({error})"))
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_forces_libvpx_only_for_webm_sources() {
        let webm = preview_args(Path::new(r"C:\t\source.webm"), Path::new(r"C:\t\preview.webm"));
        assert_eq!(&webm[4..8], ["-c:v", "libvpx-vp9", "-i", r"C:\t\source.webm"]);
        let other = preview_args(Path::new(r"C:\t\source"), Path::new(r"C:\t\preview.webm"));
        assert_eq!(&other[4..6], ["-i", r"C:\t\source"]);
        assert_eq!(other.last().unwrap(), r"C:\t\preview.webm");
    }

    #[test]
    fn sequence_args_carry_rate_count_and_pattern() {
        let args = sequence_args(Path::new(r"C:\t\frame-%08d.png"), 23.976, 48, Path::new(r"C:\t\sequence.webm"));
        let after = |flag: &str| args[args.iter().position(|arg| arg == flag).unwrap() + 1].as_str();
        assert_eq!(after("-framerate"), "23.976");
        assert_eq!(after("-start_number"), "0");
        assert_eq!(after("-i"), r"C:\t\frame-%08d.png");
        assert_eq!(after("-frames:v"), "48");
        assert_eq!(after("-lossless"), "1");
        // JS prints whole rates without a fraction; ffmpeg accepts both.
        assert!(sequence_args(Path::new("p"), 30.0, 2, Path::new("o")).contains(&"30".to_owned()));
    }

    #[test]
    fn progress_reports_increasing_frames_capped_at_count() {
        let mut reader = ProgressReader::new(10);
        let seen: Vec<u64> = ["frame=0", "fps=0.0", "frame=  4 ", "frame=4", "frame=3", "frame=12", "progress=end"]
            .into_iter()
            .filter_map(|line| reader.line(line))
            .collect();
        assert_eq!(seen, [0, 4, 10]);
    }
}
