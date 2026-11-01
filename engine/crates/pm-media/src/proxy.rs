//! Portable worker encode path. No shell, Node, UI or Apple framework dependency.
//! The macOS host implements `RenditionEncoder` using its VideoToolbox sink.
//! Software delivery is deliberately restricted to lossless FFV1/Matroska; an
//! unsupported profile fails rather than silently changing authored semantics.
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fraction {
    pub numerator: u32,
    pub denominator: u32,
}
impl Fraction {
    fn parse(text: &str) -> io::Result<Self> {
        let (n, d) = text
            .split_once('/')
            .ok_or_else(|| invalid("missing rational"))?;
        let value = Self {
            numerator: n.parse().map_err(|_| invalid("invalid rational"))?,
            denominator: d.parse().map_err(|_| invalid("invalid rational"))?,
        };
        if value.numerator == 0 || value.denominator == 0 {
            return Err(invalid("zero rational"));
        }
        Ok(value)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timing {
    pub samples: u64,
    pub start: i64,
    pub duration: u64,
    /// Ordered PTS, duration and audio sample counts, in the stream timebase.
    pub digest: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub rate: Fraction,
    pub time_base: Fraction,
    pub alpha: bool,
    pub colour: String,
    pub video: Timing,
    pub audio: Option<Audio>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Audio {
    pub channels: u16,
    pub sample_rate: u32,
    pub time_base: Fraction,
    pub timing: Timing,
}
impl Probe {
    pub fn verify_rendition(&self, output: &Self, width: u32, height: u32) -> io::Result<()> {
        if output.width != width
            || output.height != height
            || self.rate != output.rate
            || self.time_base != output.time_base
            || self.video != output.video
            || self.alpha != output.alpha
            || self.audio != output.audio
            || self.colour != output.colour
        {
            return Err(invalid(
                "encoded output changed timing, alpha, audio or interpretation",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct EncodeRequest {
    pub source: PathBuf,
    /// A new, private path in host-owned staging storage. Never an original.
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub codec: String,
    pub container: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Continue,
    Pause,
    Cancel,
}
impl Control {
    pub fn check(self) -> io::Result<()> {
        if self == Self::Continue {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "media work interrupted",
            ))
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeOutcome {
    Complete,
    Paused,
    Cancelled,
}
/// Implement on a worker using VideoToolbox on macOS. Returning control means
/// all codec/process resources have been released. Partial bytes remain private
/// for the host's archive policy. No adapter attaches media or mutates models.
pub trait RenditionEncoder: Send + Sync {
    fn probe(&self, path: &Path) -> io::Result<Probe>;
    /// Native adapters override this when probing can block. Interrupted is
    /// returned only after releasing probe resources; callers retain control.
    fn probe_controlled(
        &self,
        path: &Path,
        control: &mut dyn FnMut() -> Control,
    ) -> io::Result<Probe> {
        control().check()?;
        let result = self.probe(path)?;
        control().check()?;
        Ok(result)
    }
    fn encode(
        &self,
        request: &EncodeRequest,
        observe: &mut dyn FnMut(u64) -> Control,
    ) -> io::Result<EncodeOutcome>;
}
#[derive(Clone, Debug)]
pub struct SoftwareEncoder {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}
impl Default for SoftwareEncoder {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        }
    }
}
/// Every exit path, including an observer panic, kills and reaps the child.
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn child(command: &mut Command) -> io::Result<Process> {
    Ok(Process(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?,
    ))
}
fn bounded_output(command: &mut Command, limit: u64) -> io::Result<Vec<u8>> {
    let mut process = child(command)?;
    let mut bytes = Vec::new();
    process
        .0
        .stdout
        .take()
        .expect("piped")
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("decoder output exceeds safety bound"));
    }
    if !process.0.wait()?.success() {
        return Err(invalid("media tool failed"));
    }
    Ok(bytes)
}
// Bounded streaming records with cooperative control even while the tool is
// blocked in decoding and emits no stdout. No whole-movie timing allocation.
fn records(
    command: &mut Command,
    max_record: u64,
    control: &mut dyn FnMut() -> Control,
    mut consume: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    control().check()?;
    let mut process = child(command)?;
    let stdout = process.0.stdout.take().expect("piped");
    let (tx, rx) = mpsc::sync_channel(8);
    let reader = thread::spawn(move || {
        let mut input = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            let record = match input
                .by_ref()
                .take(max_record + 1)
                .read_until(b'\n', &mut line)
            {
                Ok(0) => break,
                Ok(_) if line.len() as u64 > max_record => {
                    Err(invalid("oversized media tool record"))
                }
                Ok(_) => Ok(line),
                Err(e) => Err(e),
            };
            let failed = record.is_err();
            if tx.send(record).is_err() || failed {
                break;
            }
        }
    });
    let result = (|| {
        loop {
            control().check()?;
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(line) => consume(&line?)?,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Some(status) = process.0.try_wait()? {
                        return if status.success() {
                            Ok(())
                        } else {
                            Err(invalid("media tool failed"))
                        };
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
        }
    })();
    drop(process);
    drop(rx);
    let _ = reader.join();
    result
}
fn metadata(command: &mut Command, control: &mut dyn FnMut() -> Control) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    records(command, 4 * 1024 * 1024, control, |line| {
        if data.len() + line.len() > 4 * 1024 * 1024 {
            return Err(invalid("oversized media metadata"));
        }
        data.extend_from_slice(line);
        Ok(())
    })?;
    Ok(data)
}
fn field<'a>(value: &'a serde_json::Value, name: &str) -> io::Result<&'a str> {
    value[name]
        .as_str()
        .ok_or_else(|| invalid(format!("missing {name}")))
}
fn integer(value: &serde_json::Value, name: &str) -> io::Result<u32> {
    value[name]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| invalid(format!("missing {name}")))
}
impl SoftwareEncoder {
    fn timing(
        &self,
        path: &Path,
        stream: &str,
        control: &mut dyn FnMut() -> Control,
    ) -> io::Result<Timing> {
        let mut command = Command::new(&self.ffprobe);
        command
            .args([
                "-v",
                "error",
                "-select_streams",
                stream,
                "-show_frames",
                "-show_entries",
                "frame=best_effort_timestamp,pkt_duration,duration,nb_samples",
                "-of",
                "compact=p=0",
            ])
            .arg(path);
        let mut digest = blake3::Hasher::new();
        let mut samples = 0u64;
        let mut start = i64::MAX;
        let mut end = i64::MIN;
        records(&mut command, 4096, control, |line| {
            let text = std::str::from_utf8(line).map_err(|_| invalid("invalid timing text"))?;
            let fields: std::collections::BTreeMap<_, _> = text
                .trim()
                .split('|')
                .filter_map(|s| s.split_once('='))
                .collect();
            if fields.is_empty() {
                return Ok(());
            }
            let number = |key: &str| fields.get(key).and_then(|s| s.parse::<i64>().ok());
            let pts =
                number("best_effort_timestamp").ok_or_else(|| invalid("missing frame PTS"))?;
            let duration = number("duration")
                .or_else(|| number("pkt_duration"))
                .filter(|d| *d > 0)
                .ok_or_else(|| invalid("missing exact sample duration"))?;
            start = start.min(pts);
            end = end.max(
                pts.checked_add(duration)
                    .ok_or_else(|| invalid("timing overflow"))?,
            );
            digest.update(&pts.to_le_bytes());
            digest.update(&duration.to_le_bytes());
            digest.update(&number("nb_samples").unwrap_or(0).to_le_bytes());
            samples = samples
                .checked_add(1)
                .ok_or_else(|| invalid("sample count overflow"))?;
            Ok(())
        })?;
        if samples == 0 {
            return Err(invalid("no decoded samples"));
        }
        Ok(Timing {
            samples,
            start,
            duration: u64::try_from(i128::from(end) - i128::from(start))
                .map_err(|_| invalid("duration overflow"))?,
            digest: *digest.finalize().as_bytes(),
        })
    }
    /// Decode one exact ordinal into straight RGB code values (no LUT/display
    /// transform). This is a Linux worker adapter, not pm-render E readback.
    pub fn rgba_frame(
        &self,
        path: &Path,
        ordinal: u64,
        width: u32,
        height: u32,
    ) -> io::Result<Vec<[f32; 4]>> {
        let size = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|n| n.checked_mul(16))
            .filter(|n| *n > 0 && *n <= 1024 * 1024 * 1024)
            .ok_or_else(|| invalid("frame size bound"))?;
        let bytes = bounded_output(
            Command::new(&self.ffmpeg)
                .args(["-v", "error", "-nostdin", "-i"])
                .arg(path)
                .args([
                    "-map",
                    "0:v:0",
                    "-vf",
                    &format!("select=eq(n\\,{ordinal})"),
                    "-frames:v",
                    "1",
                    "-pix_fmt",
                    "gbrapf32le",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ]),
            size,
        )?;
        if bytes.len() as u64 != size {
            return Err(invalid("decoded frame dimensions/ordinal mismatch"));
        }
        let pixels = bytes.len() / 16;
        let component = |plane: usize, i: usize| {
            let offset = (plane * pixels + i) * 4;
            f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four bytes"))
        };
        Ok((0..pixels)
            .map(|i| {
                [
                    component(2, i),
                    component(0, i),
                    component(1, i),
                    component(3, i),
                ]
            })
            .collect())
    }
}
impl RenditionEncoder for SoftwareEncoder {
    fn probe(&self, path: &Path) -> io::Result<Probe> {
        self.probe_controlled(path, &mut || Control::Continue)
    }
    fn probe_controlled(
        &self,
        path: &Path,
        control: &mut dyn FnMut() -> Control,
    ) -> io::Result<Probe> {
        let bytes = metadata(
            Command::new(&self.ffprobe)
                .args(["-v", "error", "-show_streams", "-of", "json"])
                .arg(path),
            control,
        )?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let streams = value["streams"]
            .as_array()
            .ok_or_else(|| invalid("missing streams"))?;
        let video: Vec<_> = streams
            .iter()
            .filter(|s| s["codec_type"] == "video")
            .collect();
        let audio: Vec<_> = streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .collect();
        if video.len() != 1 || audio.len() > 1 || streams.len() != video.len() + audio.len() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "requires one video and at most one audio stream",
            ));
        }
        let v = video[0];
        let pixel = field(v, "pix_fmt")?;
        let alpha = pixel.starts_with("yuva")
            || pixel.starts_with("gbrap")
            || [
                "rgba", "bgra", "argb", "abgr", "rgba64le", "bgra64le", "ya8", "ya16le",
            ]
            .contains(&pixel);
        let mut interpretation = serde_json::Map::new();
        for key in [
            "color_range",
            "color_space",
            "color_transfer",
            "color_primaries",
            "chroma_location",
            "sample_aspect_ratio",
            "field_order",
            "side_data_list",
        ] {
            if let Some(value) = v.get(key) {
                interpretation.insert(key.into(), value.clone());
            }
        }
        let audio = audio
            .first()
            .map(|a| -> io::Result<Audio> {
                Ok(Audio {
                    channels: u16::try_from(integer(a, "channels")?)
                        .map_err(|_| invalid("channel count"))?,
                    sample_rate: field(a, "sample_rate")?
                        .parse()
                        .map_err(|_| invalid("sample rate"))?,
                    time_base: Fraction::parse(field(a, "time_base")?)?,
                    timing: self.timing(path, "a:0", control)?,
                })
            })
            .transpose()?;
        Ok(Probe {
            width: integer(v, "width")?,
            height: integer(v, "height")?,
            rate: Fraction::parse(field(v, "r_frame_rate")?)?,
            time_base: Fraction::parse(field(v, "time_base")?)?,
            alpha,
            colour: serde_json::to_string(&interpretation)?,
            video: self.timing(path, "v:0", control)?,
            audio,
        })
    }
    fn encode(
        &self,
        request: &EncodeRequest,
        observe: &mut dyn FnMut(u64) -> Control,
    ) -> io::Result<EncodeOutcome> {
        if request.codec != "ffv1" || request.container != "mkv" {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "software profile requires ffv1/mkv",
            ));
        }
        if request.width == 0 || request.height == 0 || request.source == request.output {
            return Err(invalid("invalid encode paths/dimensions"));
        }
        match std::fs::symlink_metadata(&request.output) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "private output already exists",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match observe(0) {
            Control::Pause => return Ok(EncodeOutcome::Paused),
            Control::Cancel => return Ok(EncodeOutcome::Cancelled),
            _ => {}
        }
        // -n is defense in depth; the caller owns a freshly allocated staging
        // directory. Do not use -y or publish into a user-selected destination.
        let mut process = child(
            Command::new(&self.ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-nostdin",
                    "-n",
                    "-copyts",
                    "-noautorotate",
                    "-i",
                ])
                .arg(&request.source)
                .args([
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a:0?",
                    "-vf",
                    &format!("scale={}:{}:flags=lanczos", request.width, request.height),
                    "-c:v",
                    "ffv1",
                    "-threads:v",
                    "4",
                    "-g",
                    "1",
                    "-level",
                    "3",
                    "-c:a",
                    "copy",
                    "-fps_mode",
                    "passthrough",
                    "-enc_time_base:v",
                    "demux",
                    "-avoid_negative_ts",
                    "disabled",
                    "-f",
                    "matroska",
                    "-progress",
                    "pipe:1",
                    "-stats_period",
                    "0.05",
                ])
                .arg(&request.output),
        )?;
        let stdout = process.0.stdout.take().expect("piped");
        let (tx, rx) = mpsc::sync_channel(8);
        // Bounded channel and records; drop the receiver before joining on stop.
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Some(n) = line
                    .strip_prefix("frame=")
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    && tx.send(n).is_err()
                {
                    break;
                }
            }
        });
        let result = (|| {
            let mut completed = 0;
            loop {
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(n) => completed = completed.max(n),
                    Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                    }
                }
                match observe(completed) {
                    Control::Pause => return Ok(EncodeOutcome::Paused),
                    Control::Cancel => return Ok(EncodeOutcome::Cancelled),
                    Control::Continue => {}
                }
                if let Some(status) = process.0.try_wait()? {
                    if !status.success() {
                        return Err(invalid("software encode failed"));
                    }
                    File::open(&request.output)?.sync_all()?;
                    File::open(
                        request
                            .output
                            .parent()
                            .ok_or_else(|| invalid("output parent"))?,
                    )?
                    .sync_all()?;
                    return Ok(EncodeOutcome::Complete);
                }
            }
        })();
        drop(process); // codec fully reaped before control acknowledgement
        drop(rx);
        let _ = reader.join();
        result
    }
}
