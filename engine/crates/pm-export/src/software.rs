//! Portable, headless software delivery through an explicitly selected FFmpeg
//! executable. No shell, Node, browser, display, or GPU is involved. Video is
//! streamed with pipe backpressure; PCM is spooled in a private job directory.
//! A file is published only after probing and decoding the sampled pictures.
//!
//! Distribution must supply appropriately licensed software encoders. The
//! app's LGPL FFmpeg build deliberately does not include libx264/libx265.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::atomic::AtomicOutput;
use crate::convert::{EncoderInput, Plane, PlaneData};
use crate::pipeline::Sink;
use crate::timescale::video_timing;
use crate::verify::{self, AudioReport, BlockSample, TimecodeReport, VerifyReport};
use crate::*;

/// Tools are explicit so tests and headless hosts can select their own build.
#[derive(Clone, Debug)]
pub struct SoftwareTools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}
impl Default for SoftwareTools {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        }
    }
}

struct Process(Child);
impl Process {
    fn wait(&mut self, cancel: &AtomicBool) -> Result<(), Error> {
        self.0.stdin.take();
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(Error::Cancelled);
            }
            if let Some(status) = self.0.try_wait().map_err(io)? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(Error::Encode(format!(
                        "software codec exited with {status}"
                    )))
                };
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.0.stdin.take();
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn io(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}
fn private_file(path: &Path) -> Result<File, Error> {
    let mut open = OpenOptions::new();
    open.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    open.open(path).map_err(io)
}
fn command(tool: &Path) -> Command {
    let mut c = Command::new(tool);
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    c
}
fn run(cmd: Command, cancel: &AtomicBool, scratch: &Path) -> Result<Vec<u8>, Error> {
    run_to_file(cmd, cancel, scratch)?;
    fs::read(scratch.join("command-output")).map_err(io)
}
fn run_to_file(mut cmd: Command, cancel: &AtomicBool, scratch: &Path) -> Result<(), Error> {
    let output = scratch.join("command-output");
    // Each operation is sequential; remove only this job's previous capture.
    if output.exists() {
        fs::remove_file(&output).map_err(io)?;
    }
    let file = private_file(&output)?;
    cmd.stdout(Stdio::from(file))
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    let mut process = Process(cmd.spawn().map_err(io)?);
    process.wait(cancel)?;
    Ok(())
}

/// Encodes the same engine-owned input planes as VideoToolbox, behind `Sink`.
/// HEVC alpha has no interoperable software encoder here and is rejected
/// before an output is created; it is never silently flattened.
pub struct SoftwareSink {
    // Processes close before the directory and AtomicOutput are removed.
    encoder: Process,
    audio_file: Option<File>,
    scratch: Scratch,
    output: AtomicOutput,
    spec: ExportSpec,
    tools: SoftwareTools,
    next: u64,
    audio_frames: u64,
    blocks: Vec<BlockSample>,
}

impl SoftwareSink {
    pub fn new(spec: &ExportSpec) -> Result<Self, Error> {
        Self::with_tools(spec, SoftwareTools::default())
    }
    pub fn with_tools(spec: &ExportSpec, tools: SoftwareTools) -> Result<Self, Error> {
        spec.validate()?;
        if !matches!(
            spec.container,
            Container::Mov | Container::Mp4 | Container::Webm
        ) {
            return Err(Error::InvalidSpec(
                "software video sink needs MOV, MP4, or WebM".into(),
            ));
        }
        if matches!(spec.codec, Codec::Hevc { .. }) && spec.alpha != AlphaOutput::None {
            return Err(Error::InvalidSpec(
                "HEVC alpha requires the VideoToolbox sink".into(),
            ));
        }
        let output = match spec.overwrite {
            Overwrite::InPlace => {
                AtomicOutput::in_place(&spec.destination, spec.container.extension(), spec.mode)?
            }
            o => AtomicOutput::new(&spec.destination, o, spec.mode)?,
        };
        let work = output.temp_path().with_extension("work");
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&work).map_err(io)?;
        let scratch = Scratch(work);
        let audio_file = spec
            .audio
            .map(|_| private_file(&scratch.0.join("audio.pcm")))
            .transpose()?;
        let mut cmd = command(&tools.ffmpeg);
        cmd.args([
            "-f",
            "rawvideo",
            "-pixel_format",
            raw_format(spec.input_format(), spec.alpha),
            "-video_size",
            &format!("{}x{}", spec.width, spec.height),
            "-framerate",
            &format!("{}/{}", spec.rate.num(), spec.rate.den()),
            "-color_primaries",
            &spec.tags.cicp().0.to_string(),
            "-color_trc",
            &spec.tags.cicp().1.to_string(),
            "-colorspace",
            &spec.tags.cicp().2.to_string(),
            "-color_range",
            "tv",
            "-i",
            "pipe:0",
            "-an",
        ]);
        video_options(&mut cmd, spec);
        mux_options(&mut cmd, spec)?;
        let video_path = scratch
            .0
            .join(format!("video.{}", spec.container.extension()));
        // Reserve every media file owner-only before handing it to FFmpeg.
        // -y is confined to this job's create_new file, never a destination.
        private_file(&video_path)?;
        cmd.arg("-y").arg(video_path);
        let log = private_file(&scratch.0.join("encoder.log"))?;
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log));
        let encoder = Process(cmd.spawn().map_err(io)?);
        Ok(Self {
            encoder,
            audio_file,
            scratch,
            output,
            spec: spec.clone(),
            tools,
            next: 0,
            audio_frames: 0,
            blocks: vec![],
        })
    }

    fn complete(mut self, cancel: &AtomicBool) -> Result<Delivered, Error> {
        if self.next != self.spec.frames {
            return Err(Error::InvalidFrame("incomplete video".into()));
        }
        self.encoder.wait(cancel).map_err(|e| {
            if matches!(e, Error::Cancelled) {
                e
            } else {
                // Bound diagnostics and keep them local to this job.
                let log =
                    fs::read_to_string(self.scratch.0.join("encoder.log")).unwrap_or_default();
                Error::Encode(format!(
                    "{e}: {}",
                    log.chars().take(4096).collect::<String>()
                ))
            }
        })?;
        self.audio_file.take();
        let video = self
            .scratch
            .0
            .join(format!("video.{}", self.spec.container.extension()));
        if let Some(audio) = self.spec.audio {
            if self.audio_frames == 0 {
                return Err(Error::InvalidFrame("audio track has no samples".into()));
            }
            let mut cmd = command(&self.tools.ffmpeg);
            cmd.arg("-i")
                .arg(&video)
                .args([
                    "-f",
                    audio_input(audio.codec),
                    "-ar",
                    &audio.sample_rate.to_string(),
                    "-ac",
                    &audio.channels.to_string(),
                    "-i",
                ])
                .arg(self.scratch.0.join("audio.pcm"))
                .args([
                    "-map",
                    "0:v:0",
                    "-map",
                    "1:a:0",
                    "-c:v",
                    "copy",
                    "-c:a",
                    match audio.codec {
                        AudioCodec::Pcm16 => "pcm_s16le",
                        AudioCodec::Pcm24 => "pcm_s24le",
                        AudioCodec::PcmF32 => "pcm_f32le",
                        AudioCodec::Aac => "aac",
                        AudioCodec::Opus => "libopus",
                    },
                ]);
            if audio.codec == AudioCodec::Aac {
                cmd.args(["-b:a", &(128_000 * u32::from(audio.channels)).to_string()]);
            }
            if audio.codec == AudioCodec::Opus {
                cmd.args(["-b:a", "160000"]);
            }
            mux_options(&mut cmd, &self.spec)?;
            private_file(self.output.temp_path())?;
            cmd.arg("-y").arg(self.output.temp_path());
            run(cmd, cancel, &self.scratch.0)?;
        } else {
            fs::rename(video, self.output.temp_path()).map_err(io)?;
        }
        let report = probe_and_verify(
            &self.tools,
            self.output.temp_path(),
            &self.spec,
            self.audio_frames,
            &self.blocks,
            cancel,
            &self.scratch.0,
        )?;
        if cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        Ok(Delivered {
            path: self.output.commit()?,
            report,
        })
    }
}

impl Sink for SoftwareSink {
    type Output = Delivered;
    fn video(&mut self, index: u64, input: &EncoderInput) -> Result<(), Error> {
        input.validate()?;
        if index != self.next
            || index >= self.spec.frames
            || input.width != self.spec.width
            || input.height != self.spec.height
            || input.format != self.spec.input_format()
        {
            return Err(Error::InvalidFrame(
                "software sink input index, size, or format mismatch".into(),
            ));
        }
        let bytes = pack(input, self.spec.alpha);
        self.encoder
            .0
            .stdin
            .as_mut()
            .ok_or_else(|| Error::Encode("encoder pipe closed".into()))?
            .write_all(&bytes)
            .map_err(io)?;
        if verify::sampled_frames(self.spec.frames).contains(&index) {
            self.blocks.extend(verify::block_means(input, index));
        }
        self.next += 1;
        Ok(())
    }
    fn audio(&mut self, samples: &[f32]) -> Result<(), Error> {
        let Some(spec) = self.spec.audio else {
            return Err(Error::InvalidFrame("audio was not requested".into()));
        };
        if !samples.len().is_multiple_of(usize::from(spec.channels))
            || samples.iter().any(|s| !s.is_finite())
        {
            return Err(Error::InvalidFrame("invalid interleaved PCM".into()));
        }
        let bytes = audio_bytes(samples, spec, self.audio_frames);
        self.audio_file
            .as_mut()
            .ok_or_else(|| Error::InvalidFrame("audio spool closed".into()))?
            .write_all(&bytes)
            .map_err(io)?;
        self.audio_frames += (samples.len() / usize::from(spec.channels)) as u64;
        Ok(())
    }
    fn finish(self) -> Result<Delivered, Error> {
        self.complete(&AtomicBool::new(false))
    }
    fn finish_with_cancel(self, cancel: &AtomicBool) -> Result<Delivered, Error> {
        self.complete(cancel)
    }
}

fn audio_input(codec: AudioCodec) -> &'static str {
    match codec {
        AudioCodec::Pcm16 => "s16le",
        AudioCodec::Pcm24 => "s24le",
        _ => "f32le",
    }
}

// The native PCM quantizer (pm-audio offline contract), kept portable here so
// selecting a muxer cannot change rounding or delegate dither to a codec build.
fn audio_bytes(samples: &[f32], spec: AudioSpec, first: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);
    for (k, &v) in samples.iter().enumerate() {
        match spec.codec {
            AudioCodec::Pcm16 => {
                let channels = usize::from(spec.channels);
                let frame = first.wrapping_add((k / channels) as u64);
                let mut z =
                    frame.wrapping_mul(64).wrapping_add((k % channels) as u64) ^ 0x5EED_D17E;
                z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                z ^= z >> 31;
                let noise =
                    ((z >> 40) as f64 - ((z >> 16) & 0xFF_FFFF) as f64) / (1u64 << 24) as f64;
                let value = (f64::from(v) * 32768.0 + noise)
                    .round()
                    .clamp(-32768.0, 32767.0) as i16;
                out.extend(value.to_le_bytes());
            }
            AudioCodec::Pcm24 => {
                let value = (f64::from(v) * 8_388_608.0)
                    .round()
                    .clamp(-8_388_608.0, 8_388_607.0) as i32;
                out.extend(&value.to_le_bytes()[..3]);
            }
            _ => out.extend(v.to_le_bytes()),
        }
    }
    out
}

fn raw_format(format: InputFormat, alpha: AlphaOutput) -> &'static str {
    match format {
        InputFormat::Y416 if alpha != AlphaOutput::None => "yuva444p16le",
        InputFormat::Y416 => "yuv444p16le",
        InputFormat::V216 => "yuv422p16le",
        InputFormat::V420 => "nv12",
        InputFormat::X420 => "p010le",
        InputFormat::I420 => "yuv420p",
        InputFormat::I420a => "yuva420p",
        _ => unreachable!("software codec validated before opening"),
    }
}
fn video_options(cmd: &mut Command, s: &ExportSpec) {
    cmd.args(["-threads", "2", "-filter_threads", "1"]);
    match s.codec {
        Codec::ProRes(p) => {
            let profile = match p {
                ProResProfile::Proxy => "0",
                ProResProfile::Lt => "1",
                ProResProfile::Standard => "2",
                ProResProfile::Hq => "3",
                ProResProfile::P4444 => "4",
                ProResProfile::P4444Xq => "5",
            };
            cmd.args([
                "-c:v",
                "prores_ks",
                "-profile:v",
                profile,
                "-pix_fmt",
                if p.is_4444() {
                    if s.alpha == AlphaOutput::None {
                        "yuv444p10le"
                    } else {
                        "yuva444p10le"
                    }
                } else {
                    "yuv422p10le"
                },
                "-alpha_bits",
                "16",
            ]);
        }
        Codec::H264 { .. } => {
            cmd.args([
                "-c:v",
                "libx264",
                "-profile:v",
                "high",
                "-pix_fmt",
                "yuv420p",
            ]);
        }
        Codec::Hevc { ten_bit, .. } => {
            cmd.args([
                "-c:v",
                "libx265",
                "-profile:v",
                if ten_bit { "main10" } else { "main" },
                "-pix_fmt",
                if ten_bit { "yuv420p10le" } else { "yuv420p" },
                "-x265-params",
                "pools=none:frame-threads=1:log-level=error",
                "-tag:v",
                "hvc1",
            ]);
        }
        Codec::Vp9 { .. } | Codec::Vp8 { .. } => {
            cmd.args([
                "-c:v",
                if matches!(s.codec, Codec::Vp9 { .. }) {
                    "libvpx-vp9"
                } else {
                    "libvpx"
                },
                "-pix_fmt",
                if s.alpha == AlphaOutput::None {
                    "yuv420p"
                } else {
                    "yuva420p"
                },
                "-auto-alt-ref",
                "0",
            ]);
        }
        Codec::Png { .. } => unreachable!(),
    }
    if let Some(rate) = s.codec.bitrate_bps() {
        cmd.args(["-b:v", &rate.to_string()]);
    }
    let (p, t, m) = s.tags.cicp();
    cmd.args([
        "-color_primaries",
        &p.to_string(),
        "-color_trc",
        &t.to_string(),
        "-colorspace",
        &m.to_string(),
        "-color_range",
        "tv",
    ]);
}
fn mux_options(cmd: &mut Command, s: &ExportSpec) -> Result<(), Error> {
    if s.container.is_quicktime_family() {
        let timing = video_timing(s.rate)?;
        cmd.args([
            "-video_track_timescale",
            &timing.timescale.to_string(),
            "-movie_timescale",
            &timing.timescale.to_string(),
            "-movflags",
            "+faststart+write_colr",
        ]);
        if let Some(tc) = &s.timecode {
            cmd.args(["-timecode", &tc.format.format(tc.start_frame)]);
            if let Some(reel) = &tc.reel {
                cmd.args(["-metadata:s:v:0", &format!("reel_name={reel}")]);
            }
        }
    }
    Ok(())
}

// Only repack the already converted planes. FFmpeg never chooses a matrix.
fn pack(input: &EncoderInput, alpha: AlphaOutput) -> Vec<u8> {
    let mut out = Vec::new();
    match input.format {
        InputFormat::Y416 => {
            let data = input.planes[0].u16().unwrap();
            for ch in [1, 2, 3, 0]
                .into_iter()
                .take(if alpha == AlphaOutput::None { 3 } else { 4 })
            {
                for pixel in data.chunks_exact(4) {
                    out.extend(pixel[ch].to_le_bytes());
                }
            }
        }
        InputFormat::V216 => {
            let data = input.planes[0].u16().unwrap();
            for p in data.chunks_exact(4) {
                out.extend(p[1].to_le_bytes());
                out.extend(p[3].to_le_bytes());
            }
            for ch in [0, 2] {
                for p in data.chunks_exact(4) {
                    out.extend(p[ch].to_le_bytes());
                }
            }
        }
        _ => {
            for p in &input.planes {
                match &p.data {
                    PlaneData::U8(v) => out.extend(v),
                    PlaneData::U16(v) => out.extend(v.iter().flat_map(|v| v.to_le_bytes())),
                }
            }
        }
    }
    out
}

fn unpack(bytes: &[u8], spec: &ExportSpec) -> EncoderInput {
    let w = spec.width as usize;
    let h = spec.height as usize;
    let format = spec.input_format();
    let planes = if matches!(format, InputFormat::Y416 | InputFormat::V216) {
        let data: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        let mut v = Vec::with_capacity(w * h * if format == InputFormat::Y416 { 4 } else { 2 });
        if format == InputFormat::Y416 {
            for i in 0..w * h {
                v.extend([
                    if spec.alpha == AlphaOutput::None {
                        65535
                    } else {
                        data[3 * w * h + i]
                    },
                    data[i],
                    data[w * h + i],
                    data[2 * w * h + i],
                ]);
            }
        } else {
            for i in 0..w * h / 2 {
                v.extend([
                    data[w * h + i],
                    data[2 * i],
                    data[w * h + w * h / 2 + i],
                    data[2 * i + 1],
                ]);
            }
        }
        vec![Plane {
            data: PlaneData::U16(v),
            row_samples: w * if format == InputFormat::Y416 { 4 } else { 2 },
            rows: h,
        }]
    } else {
        let mut start = 0;
        crate::convert::plane_layout(format, spec.width, spec.height)
            .into_iter()
            .map(|(row_samples, rows, u16s)| {
                let len = row_samples * rows * if u16s { 2 } else { 1 };
                let slice = &bytes[start..start + len];
                start += len;
                Plane {
                    row_samples,
                    rows,
                    data: if u16s {
                        PlaneData::U16(
                            slice
                                .chunks_exact(2)
                                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                                .collect(),
                        )
                    } else {
                        PlaneData::U8(slice.to_vec())
                    },
                }
            })
            .collect()
    };
    EncoderInput {
        format,
        width: spec.width,
        height: spec.height,
        planes,
    }
}

fn number(v: &Value, key: &str) -> u64 {
    v[key]
        .as_u64()
        .or_else(|| v[key].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}
fn rational(v: &Value, key: &str) -> Option<(u64, u64)> {
    let (n, d) = v[key].as_str()?.split_once('/')?;
    Some((n.parse().ok()?, d.parse().ok()?))
}

fn probe_and_verify(
    tools: &SoftwareTools,
    path: &Path,
    spec: &ExportSpec,
    audio_frames: u64,
    blocks: &[BlockSample],
    cancel: &AtomicBool,
    scratch: &Path,
) -> Result<VerifyReport, Error> {
    let mut cmd = Command::new(&tools.ffprobe);
    cmd.args([
        "-v",
        "error",
        "-count_frames",
        "-show_streams",
        "-of",
        "json",
    ])
    .arg(path);
    let probe: Value = serde_json::from_slice(&run(cmd, cancel, scratch)?)
        .map_err(|e| Error::Verify(format!("invalid probe: {e}")))?;
    let streams = probe["streams"]
        .as_array()
        .ok_or_else(|| Error::Verify("probe has no streams".into()))?;
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or_else(|| Error::Verify("video track missing".into()))?;
    let want_name = match spec.codec {
        Codec::H264 { .. } => "h264",
        Codec::Hevc { .. } => "hevc",
        Codec::ProRes(_) => "prores",
        Codec::Vp9 { .. } => "vp9",
        Codec::Vp8 { .. } => "vp8",
        _ => unreachable!(),
    };
    let mut problems = vec![];
    if video["codec_name"] != want_name
        || number(video, "width") != u64::from(spec.width)
        || number(video, "height") != u64::from(spec.height)
        || number(video, "nb_read_frames") != spec.frames
    {
        problems.push("codec, dimensions, or decoded frame count mismatch".into());
    }
    if let Some((n, d)) = rational(video, "avg_frame_rate") {
        if n * spec.rate.den() as u64 != d * spec.rate.num() as u64 {
            problems.push("frame rate mismatch".into());
        }
    } else {
        problems.push("missing frame rate".into());
    }
    let want_profile = match spec.codec {
        Codec::H264 { .. } => "High",
        Codec::Hevc { ten_bit: true, .. } => "Main 10",
        Codec::Hevc { .. } => "Main",
        Codec::ProRes(p) => match p {
            ProResProfile::Proxy => "Proxy",
            ProResProfile::Lt => "LT",
            ProResProfile::Standard => "Standard",
            ProResProfile::Hq => "HQ",
            ProResProfile::P4444 => "4444",
            ProResProfile::P4444Xq => "XQ",
        },
        Codec::Vp9 { .. } => "Profile 0",
        Codec::Vp8 { .. } => "0",
        _ => unreachable!(),
    };
    // VP8 has no standardized profile string in ffprobe.
    if !matches!(spec.codec, Codec::Vp8 { .. }) && video["profile"] != want_profile {
        problems.push(format!(
            "profile {:?}, expected {want_profile}",
            video["profile"]
        ));
    }
    let pix = video["pix_fmt"].as_str().unwrap_or("");
    let want_pix = match spec.codec {
        Codec::ProRes(p) if p.is_4444() => {
            if spec.alpha == AlphaOutput::None {
                "yuv444p12le"
            } else {
                "yuva444p12le"
            }
        }
        Codec::ProRes(_) => "yuv422p10le",
        Codec::Hevc { ten_bit: true, .. } => "yuv420p10le",
        _ => "yuv420p",
    };
    // FFmpeg's native VPX decoder omits the separate alpha stream. It is
    // validated below with the libvpx decoder and AlphaMode from the muxer.
    if pix != want_pix {
        problems.push(format!("pixel format {pix}, expected {want_pix}"));
    }
    let (p, t, m) = spec.tags.cicp();
    let primaries = if p == 9 { "bt2020" } else { "bt709" };
    let transfer = match t {
        16 => "smpte2084",
        18 => "arib-std-b67",
        13 => "iec61966-2-1",
        _ => "bt709",
    };
    let matrix = if m == 9 { "bt2020nc" } else { "bt709" };
    if video["color_primaries"] != primaries
        || video["color_transfer"] != transfer
        || video["color_space"] != matrix
        || (video["color_range"] != "tv"
            && !(matches!(spec.codec, Codec::ProRes(_)) && video["color_range"].is_null()))
    {
        problems.push(format!(
            "color tags/range mismatch: primaries={} transfer={} matrix={} range={}",
            video["color_primaries"],
            video["color_transfer"],
            video["color_space"],
            video["color_range"]
        ));
    }
    let timing = video_timing(spec.rate)?;
    let mut movie_timescale = timing.timescale as u32;
    let mut media_timescale = movie_timescale;
    let mut duration = timing.duration(spec.frames) as u64;
    let mut timecode = None;
    if spec.container.is_quicktime_family() {
        let movie = crate::mp4box::Movie::read(path)?;
        let track = movie
            .track(b"vide")
            .ok_or_else(|| Error::Verify("no MOV video track".into()))?;
        movie_timescale = movie.movie_timescale;
        media_timescale = track.media_timescale;
        duration = track.stts_duration();
        if movie_timescale != timing.timescale as u32
            || media_timescale != timing.timescale as u32
            || duration != timing.duration(spec.frames) as u64
            || track
                .stts
                .iter()
                .any(|&(_, d)| i64::from(d) != timing.frame_duration)
        {
            problems.push("movie/media timescale or frame duration mismatch".into());
        }
        timecode = movie
            .track(b"tmcd")
            .and_then(|t| t.tmcd.as_ref())
            .map(|t| TimecodeReport {
                start_frame: t.start_frame.unwrap_or(u32::MAX),
                drop_frame: t.drop_frame(),
                frames_per_second: t.frames_per_second,
                reel: t.reel.clone(),
            });
        match (&spec.timecode, &timecode) {
            (None, None) => {}
            (Some(w), Some(g))
                if i64::from(g.start_frame) == w.start_frame
                    && g.drop_frame == w.format.is_drop_frame()
                    && g.reel == w.reel => {}
            _ => problems.push("timecode mismatch".into()),
        }
    }
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    let audio_report = if let (Some(want), Some(got)) = (spec.audio, audio) {
        let name = match want.codec {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "opus",
            AudioCodec::Pcm16 => "pcm_s16le",
            AudioCodec::Pcm24 => "pcm_s24le",
            AudioCodec::PcmF32 => "pcm_f32le",
        };
        if got["codec_name"] != name
            || number(got, "sample_rate") != u64::from(want.sample_rate)
            || number(got, "channels") != u64::from(want.channels)
        {
            problems.push("audio coding mismatch".into());
        }
        // Decode after priming removal; AAC/Opus packet padding may extend the
        // final packet, but cannot lose an authored sample. Container length is
        // checked separately for exact presented samples when available.
        let mut cmd = command(&tools.ffmpeg);
        cmd.arg("-i")
            .arg(path)
            .args(["-map", "0:a:0", "-f", "f32le", "pipe:1"]);
        // Stream to a file rather than retain an unbounded audio buffer.
        run_to_file(cmd, cancel, scratch)?;
        let bytes = fs::metadata(scratch.join("command-output"))
            .map_err(io)?
            .len();
        let decoded = bytes / (4 * u64::from(want.channels));
        let padding = if want.codec.is_pcm() { 0 } else { 2048 };
        if decoded < audio_frames || decoded > audio_frames + padding {
            problems.push(format!(
                "decoded audio samples {decoded}, expected {audio_frames}"
            ));
        }
        let samples = if let Some((n, d)) = rational(got, "time_base") {
            let ts = number(got, "duration_ts");
            if ts > 0 && d > 0 {
                (ts as u128 * u128::from(n) * u128::from(want.sample_rate) / u128::from(d)) as u64
            } else {
                audio_frames
            }
        } else {
            audio_frames
        };
        if samples.abs_diff(audio_frames) > 1 {
            problems.push(format!(
                "presented audio samples {samples}, expected {audio_frames}"
            ));
        }
        Some(AudioReport {
            codec: name.into(),
            sample_rate: f64::from(want.sample_rate),
            channels: u32::from(want.channels),
            bits_per_channel: number(got, "bits_per_raw_sample").max(number(got, "bits_per_sample"))
                as u32,
            float: want.codec == AudioCodec::PcmF32,
            timescale: want.sample_rate,
            samples,
        })
    } else {
        if spec.audio.is_some() || audio.is_some() {
            problems.push("audio track presence mismatch".into());
        }
        None
    };
    let mut decoded = vec![];
    for index in verify::sampled_frames(spec.frames) {
        let mut cmd = command(&tools.ffmpeg);
        if matches!(spec.codec, Codec::Vp9 { .. } | Codec::Vp8 { .. })
            && spec.alpha != AlphaOutput::None
        {
            cmd.args([
                "-c:v",
                if matches!(spec.codec, Codec::Vp9 { .. }) {
                    "libvpx-vp9"
                } else {
                    "libvpx"
                },
            ]);
        }
        cmd.arg("-i").arg(path).args([
            "-map",
            "0:v:0",
            "-vf",
            &format!("select=eq(n\\,{index})"),
            "-frames:v",
            "1",
            "-threads",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            raw_format(spec.input_format(), spec.alpha),
            "pipe:1",
        ]);
        let bytes = run(cmd, cancel, scratch)?;
        let expected = crate::convert::plane_layout(spec.input_format(), spec.width, spec.height)
            .iter()
            .map(|(w, h, u16s)| w * h * if *u16s { 2 } else { 1 })
            .sum::<usize>();
        let expected =
            if spec.input_format() == InputFormat::Y416 && spec.alpha == AlphaOutput::None {
                expected / 4 * 3
            } else {
                expected
            };
        if bytes.len() != expected {
            return Err(Error::Verify("decoded picture length mismatch".into()));
        }
        decoded.push((index, unpack(&bytes, spec)));
    }
    let pixels = verify::check_pixels(spec.codec, blocks, &decoded, &mut problems);
    if !problems.is_empty() {
        return Err(Error::Verify(problems.join("; ")));
    }
    Ok(VerifyReport {
        codec: want_name.into(),
        profile: want_profile.into(),
        pixel_format: pix.into(),
        depth: spec.codec.depth(),
        cicp: spec.tags.cicp(),
        full_range: false,
        alpha: spec.alpha != AlphaOutput::None,
        frames: number(video, "nb_read_frames"),
        movie_timescale,
        media_timescale,
        duration,
        audio: audio_report,
        timecode,
        pixels,
        hardware_encoder: Some(false),
        bytes: fs::metadata(path).map_err(io)?.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcm_dither_is_chunk_independent_and_24_bit_rounding_is_exact() {
        let spec = AudioSpec {
            codec: AudioCodec::Pcm16,
            sample_rate: 48000,
            channels: 2,
        };
        let samples: Vec<f32> = (0..200).map(|n| (n as f32 - 100.0) * 0.000_001).collect();
        let whole = audio_bytes(&samples, spec, 0);
        let mut split = audio_bytes(&samples[..70], spec, 0);
        split.extend(audio_bytes(&samples[70..], spec, 35));
        assert_eq!(whole, split);
        let spec = AudioSpec {
            codec: AudioCodec::Pcm24,
            ..spec
        };
        assert_eq!(
            audio_bytes(&[0.0, 0.5, -0.5, 1.0, -1.0], spec, 0),
            vec![0, 0, 0, 0, 0, 64, 0, 0, 192, 255, 255, 127, 0, 0, 128]
        );
    }
    #[cfg(all(feature = "engine", target_os = "macos"))]
    #[test]
    fn portable_pcm_matches_the_engine_quantizer() {
        for (codec, format) in [
            (AudioCodec::Pcm16, pm_audio::offline::PcmFormat::S16),
            (AudioCodec::Pcm24, pm_audio::offline::PcmFormat::S24),
            (AudioCodec::PcmF32, pm_audio::offline::PcmFormat::F32),
        ] {
            let samples: Vec<f32> = (0..2000).map(|n| (n as f32 - 1000.0) / 900.0).collect();
            let spec = AudioSpec {
                codec,
                sample_rate: 48000,
                channels: 2,
            };
            let mut native = vec![];
            pm_audio::offline::encode(format, &samples, 2, 51, &mut native);
            assert_eq!(audio_bytes(&samples, spec, 51), native);
        }
    }
}
