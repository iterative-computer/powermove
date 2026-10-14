//! Export encoding (`RenderEncoder` in render-encoder.ts): the renderer streams
//! raw RGBA frames into ffmpeg's stdin and PCM audio into a WAV file, then the
//! two are muxed into the file the user picked.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use parking_lot::Mutex;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::ChildStdin;
use tokio_util::sync::CancellationToken;

use super::process::{self, Process, Spawn};
use super::proxy::make_temp_dir;

const MAX_CHUNK_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mp4,
    Prores,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::Prores => "mov",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EncoderOptions {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub format: Format,
    pub alpha: bool,
    pub name: String,
    pub bitrate_mbps: Option<f64>,
}

impl EncoderOptions {
    /// `validateEncoderOptions`
    pub fn parse(raw: &Value) -> Result<Self> {
        let invalid = || anyhow!("Invalid render settings");
        let dimension = |key: &str| {
            raw.get(key).and_then(Value::as_u64).filter(|n| (2..=8192).contains(n) && n % 2 == 0).map(|n| n as u32).ok_or_else(invalid)
        };
        let width = dimension("width")?;
        let height = dimension("height")?;
        let fps = raw.get("fps").and_then(Value::as_f64).filter(|fps| fps.is_finite() && *fps > 0.0 && *fps <= 240.0).ok_or_else(invalid)?;
        let format = match raw.get("format").and_then(Value::as_str) {
            Some("mp4") => Format::Mp4,
            Some("prores") => Format::Prores,
            _ => return Err(invalid()),
        };
        let name = raw.get("name").and_then(Value::as_str).filter(|name| name.encode_utf16().count() <= 240).ok_or_else(invalid)?;
        let bitrate_mbps = match raw.get("bitrateMbps") {
            None | Some(Value::Null) => None,
            Some(value) => {
                Some(value.as_f64().filter(|rate| rate.is_finite() && (1.0..=100.0).contains(rate)).context("Invalid video bitrate")?)
            }
        };
        let alpha = format == Format::Prores && truthy(raw.get("alpha"));
        Ok(Self { width, height, fps, format, alpha, name: name.to_owned(), bitrate_mbps })
    }

    fn frame_bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4
    }

    /// The save dialog's suggested file name. Windows forbids more characters
    /// than the `[\\/:]` macOS replaces.
    pub fn default_file_name(&self) -> String {
        let safe: String = self.name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
        format!("{safe}.{}", self.format.extension())
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

pub fn encoder_args(options: &EncoderOptions, output: &Path) -> Vec<String> {
    let mut args: Vec<String> = [
        "-hide_banner", "-loglevel", "error", "-f", "rawvideo", "-pixel_format", "rgba", "-video_size",
    ]
    .map(String::from)
    .into();
    args.push(format!("{}x{}", options.width, options.height));
    args.push("-framerate".into());
    args.push(options.fps.to_string());
    args.extend(["-i", "pipe:0", "-an", "-vf", "scale=in_range=full:out_range=limited:out_color_matrix=bt709"].map(String::from));
    match options.format {
        Format::Prores => {
            let pix_fmt = if options.alpha { "yuva444p10le" } else { "yuv444p10le" };
            args.extend(["-c:v", "prores_ks", "-profile:v", "4", "-pix_fmt", pix_fmt, "-alpha_bits", "16"].map(String::from));
        }
        Format::Mp4 => {
            args.extend(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-b:v"].map(String::from));
            args.push(format!("{}M", options.bitrate_mbps.unwrap_or(16.0)));
        }
    }
    args.extend(
        ["-color_primaries", "bt709", "-color_trc", "iec61966-2-1", "-colorspace", "bt709", "-color_range", "tv", "-movflags", "+faststart", "-y"]
            .map(String::from),
    );
    args.push(output.to_string_lossy().into_owned());
    args
}

pub fn mux_args(video: &Path, audio: &Path, format: Format, output: &Path) -> Vec<String> {
    let audio_codec = if format == Format::Prores { "pcm_s24le" } else { "aac" };
    let path = |path: &Path| path.to_string_lossy().into_owned();
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-i"].map(String::from).into();
    args.push(path(video));
    args.push("-i".into());
    args.push(path(audio));
    args.extend(["-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", audio_codec, "-shortest", "-y"].map(String::from));
    args.push(path(output));
    args
}

struct Pipes {
    stdin: Option<ChildStdin>,
    bytes: u64,
}

struct Job {
    owner: u64,
    options: EncoderOptions,
    dir: PathBuf,
    file: PathBuf,
    audio: PathBuf,
    process: Process,
    pipes: tokio::sync::Mutex<Pipes>,
    destination: Mutex<Option<PathBuf>>,
}

pub struct RenderEncoder {
    binary: PathBuf,
    temp_root: PathBuf,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}

impl RenderEncoder {
    pub fn new(binary: PathBuf, temp_root: PathBuf) -> Self {
        Self { binary, temp_root, jobs: Mutex::default() }
    }

    pub async fn start(&self, options: EncoderOptions, owner: u64) -> Result<String> {
        let dir = make_temp_dir(&self.temp_root, "powermove-render-").await?;
        let file = dir.join(format!("video.{}", options.format.extension()));
        let spawned = process::spawn(Spawn {
            binary: &self.binary,
            args: encoder_args(&options, &file),
            operation: "video encoder",
            stdin: true,
            stdout: false,
            timeout: None,
            cancel: CancellationToken::new(),
        });
        let (process, stdin, _) = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&dir).await;
                return Err(error);
            }
        };
        let token = uuid::Uuid::new_v4().to_string();
        let job = Job {
            owner,
            audio: dir.join("audio.wav"),
            options,
            dir,
            file,
            process,
            pipes: tokio::sync::Mutex::new(Pipes { stdin, bytes: 0 }),
            destination: Mutex::new(None),
        };
        self.jobs.lock().insert(token.clone(), Arc::new(job));
        Ok(token)
    }

    fn job(&self, token: &str, owner: u64) -> Result<Arc<Job>> {
        self.jobs.lock().get(token).filter(|job| job.owner == owner).cloned().context("Unknown render job")
    }

    pub fn set_destination(&self, token: &str, owner: u64, destination: PathBuf) -> Result<()> {
        *self.job(token, owner)?.destination.lock() = Some(destination);
        Ok(())
    }

    pub fn destination(&self, token: &str, owner: u64) -> Result<Option<PathBuf>> {
        Ok(self.job(token, owner)?.destination.lock().clone())
    }

    /// Frames go straight into ffmpeg; the reply waits until the pipe took
    /// them, so a fast renderer cannot outrun a slow encoder.
    pub async fn write(&self, token: &str, owner: u64, data: &[u8], audio: bool) -> Result<()> {
        let job = self.job(token, owner)?;
        if data.is_empty() || data.len() > MAX_CHUNK_BYTES {
            bail!("Invalid render chunk");
        }
        let mut pipes = job.pipes.lock().await;
        if audio {
            let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(&job.audio).await?;
            file.write_all(data).await?;
            return Ok(());
        }
        if job.process.exited() {
            job.process.wait().await?;
        }
        let stdin = pipes.stdin.as_mut().context("Video encoder input is closed")?;
        if let Err(error) = stdin.write_all(data).await {
            // A closed pipe means ffmpeg stopped; its stderr says why.
            job.process.wait().await?;
            return Err(error.into());
        }
        pipes.bytes += data.len() as u64;
        Ok(())
    }

    /// Close the input, wait for the encoder, and mux any audio. Returns the
    /// finished file inside the job's temp directory.
    pub async fn finish(&self, token: &str, owner: u64) -> Result<PathBuf> {
        let job = self.job(token, owner)?;
        {
            let mut pipes = job.pipes.lock().await;
            let frame_bytes = job.options.frame_bytes();
            if pipes.bytes == 0 || pipes.bytes % frame_bytes != 0 {
                bail!("Incomplete render frame");
            }
            if let Some(mut stdin) = pipes.stdin.take() {
                let _ = stdin.shutdown().await;
            }
        }
        job.process.wait().await.map_err(|error| anyhow!("{}", nonempty(error.to_string(), "Video encoder stopped")))?;
        let has_audio = tokio::fs::metadata(&job.audio).await.is_ok_and(|info| info.len() > 0);
        if !has_audio {
            return Ok(job.file.clone());
        }
        let muxed = job.dir.join(format!("final.{}", job.options.format.extension()));
        process::run(Spawn {
            binary: &self.binary,
            args: mux_args(&job.file, &job.audio, job.options.format, &muxed),
            operation: "audio muxer",
            stdin: false,
            stdout: false,
            timeout: None,
            cancel: CancellationToken::new(),
        })
        .await?;
        Ok(muxed)
    }

    /// A cancelled job's output is discarded. ffmpeg waits on the open stdin,
    /// so kill it outright rather than waiting for a graceful stop.
    pub async fn release(&self, token: &str, owner: u64) -> Result<()> {
        let job = self.job(token, owner)?;
        self.jobs.lock().remove(token);
        if !job.process.exited() {
            job.process.kill();
        }
        let _ = job.process.wait().await;
        let _ = tokio::fs::remove_dir_all(&job.dir).await;
        Ok(())
    }

    pub async fn release_owner(&self, owner: u64) {
        let tokens: Vec<String> = self.jobs.lock().iter().filter(|(_, job)| job.owner == owner).map(|(token, _)| token.clone()).collect();
        for token in tokens {
            let _ = self.release(&token, owner).await;
        }
    }
}

fn nonempty(message: String, fallback: &str) -> String {
    if message.trim().is_empty() { fallback.to_owned() } else { message }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn options(raw: Value) -> Result<EncoderOptions> {
        EncoderOptions::parse(&raw)
    }

    #[test]
    fn validates_render_settings() {
        let parsed = options(json!({ "width": 1920, "height": 1080, "fps": 29.97, "format": "mp4", "alpha": true, "name": "Cut" })).unwrap();
        assert!(!parsed.alpha, "alpha only applies to ProRes");
        assert!(options(json!({ "width": 1919, "height": 1080, "fps": 30, "format": "mp4", "name": "x" })).is_err());
        assert!(options(json!({ "width": 1920, "height": 1080, "fps": 0, "format": "mp4", "name": "x" })).is_err());
        assert!(options(json!({ "width": 1920, "height": 1080, "fps": 30, "format": "gif", "name": "x" })).is_err());
        let bitrate = options(json!({ "width": 2, "height": 2, "fps": 30, "format": "mp4", "name": "x", "bitrateMbps": 101 }));
        assert_eq!(bitrate.unwrap_err().to_string(), "Invalid video bitrate");
    }

    #[test]
    fn builds_mp4_and_prores_encoder_args() {
        let mut settings = options(json!({ "width": 640, "height": 360, "fps": 30, "format": "mp4", "name": "x", "bitrateMbps": 8 })).unwrap();
        let mp4 = encoder_args(&settings, Path::new(r"C:\t\video.mp4"));
        let after = |args: &[String], flag: &str| args[args.iter().position(|arg| arg == flag).unwrap() + 1].clone();
        assert_eq!(after(&mp4, "-video_size"), "640x360");
        assert_eq!(after(&mp4, "-framerate"), "30");
        assert_eq!(after(&mp4, "-c:v"), "libx264");
        assert_eq!(after(&mp4, "-b:v"), "8M");
        assert_eq!(mp4.last().unwrap(), r"C:\t\video.mp4");

        settings.format = Format::Prores;
        settings.alpha = true;
        let prores = encoder_args(&settings, Path::new("video.mov"));
        assert_eq!(after(&prores, "-c:v"), "prores_ks");
        assert_eq!(after(&prores, "-pix_fmt"), "yuva444p10le");
        assert!(!prores.contains(&"-b:v".to_owned()));
    }

    #[test]
    fn mux_keeps_video_and_picks_the_audio_codec() {
        let args = mux_args(Path::new("v.mov"), Path::new("a.wav"), Format::Prores, Path::new("final.mov"));
        assert_eq!(args[args.iter().position(|arg| arg == "-c:a").unwrap() + 1], "pcm_s24le");
        assert_eq!(args[args.iter().position(|arg| arg == "-c:v").unwrap() + 1], "copy");
    }

    #[test]
    fn default_file_name_is_safe_on_windows() {
        let parsed = options(json!({ "width": 2, "height": 2, "fps": 30, "format": "prores", "name": "a/b:c?" })).unwrap();
        assert_eq!(parsed.default_file_name(), "a_b_c_.mov");
    }
}
