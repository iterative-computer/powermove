//! Today's MOV/MP4 formats through the desktop exporter's own encoder: the
//! RGBA8 frames today's renderer reads back, piped to the bundled FFmpeg with
//! `render-encoder.ts`'s exact arguments (swscale's conversion, then
//! `h264_videotoolbox` or `prores_videotoolbox`). Decision EXPORT-1: the
//! frames, the encoder and its settings are today's, so the pictures are.
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use super::{
    legacy_audio::{LegacyMux, bundled_ffmpeg},
    reader,
};
use crate::atomic::AtomicOutput;
use crate::convert::{ConvertParams, EncoderInput, WorkingFrame, convert, convert_rgba8_into};
use crate::pipeline::Sink;
use crate::settings::*;
use crate::verify::{self, BlockSample, Expected};
use crate::{Delivered, Error};

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

fn io(error: std::io::Error) -> Error {
    Error::Io(error.to_string())
}

/// Which of today's encoders a job export uses (`render-encoder.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Desktop {
    /// `mp4`: H.264 High, 4:2:0, at the quality tier's bit rate.
    H264,
    /// `prores`: ProRes 4444 from opaque frames (`p416le`) or straight-alpha
    /// frames (`ayuv64le`).
    ProRes4444 { alpha: bool },
}

impl Desktop {
    /// Today's format for `spec`, or `None` for formats only the engine
    /// writes (they keep VideoToolbox and AVAssetWriter).
    pub(crate) fn of(spec: &ExportSpec) -> Option<Desktop> {
        if spec.tags != ColorTags::Rec709QuickTime
            || spec
                .audio
                .is_some_and(|a| !a.ffmpeg_mux(spec.codec, spec.container))
            || spec
                .timecode
                .as_ref()
                .is_some_and(|tc| tc.format.format(tc.start_frame).len() != 11)
        {
            return None;
        }
        match (spec.codec, spec.container, spec.alpha) {
            (Codec::H264 { .. }, Container::Mp4, AlphaOutput::None) => Some(Desktop::H264),
            (Codec::ProRes(ProResProfile::P4444), Container::Mov, AlphaOutput::None) => {
                Some(Desktop::ProRes4444 { alpha: false })
            }
            (Codec::ProRes(ProResProfile::P4444), Container::Mov, AlphaOutput::Straight) => {
                Some(Desktop::ProRes4444 { alpha: true })
            }
            _ => None,
        }
    }
}

/// Exactly `render-encoder.ts::encoderArgs`, then the start timecode and reel
/// the delivery options add to a MOV (FFmpeg writes them as its `tmcd`).
fn args(spec: &ExportSpec, kind: Desktop, output: &std::path::Path) -> Vec<std::ffi::OsString> {
    // `String(options.fps)`: JavaScript's shortest round-trip decimal, which
    // Rust's `Display` for f64 also prints.
    let fps = spec.rate.num() as f64 / spec.rate.den() as f64;
    let mut values: Vec<String> = vec![
        "-hide_banner".into(), "-loglevel".into(), "error".into(), "-f".into(), "rawvideo".into(),
        "-pixel_format".into(), "rgba".into(), "-video_size".into(), format!("{}x{}", spec.width, spec.height),
        "-framerate".into(), fps.to_string(), "-i".into(), "pipe:0".into(), "-an".into(),
        "-vf".into(), "scale=in_range=full:out_range=limited:out_color_matrix=bt709,setparams=color_primaries=bt709:color_trc=iec61966-2-1:colorspace=bt709:range=tv".into(),
    ];
    match kind {
        Desktop::H264 => {
            let bitrate = spec.codec.bitrate_bps().unwrap_or(16_000_000);
            values.extend([
                "-c:v".into(),
                "h264_videotoolbox".into(),
                "-allow_sw".into(),
                "1".into(),
                "-profile:v".into(),
                "high".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
                "-b:v".into(),
                format!("{}M", bitrate as f64 / 1_000_000.0),
                "-g".into(),
                "250".into(),
            ]);
        }
        Desktop::ProRes4444 { alpha } => values.extend([
            "-c:v".into(),
            "prores_videotoolbox".into(),
            "-allow_sw".into(),
            "1".into(),
            "-profile:v".into(),
            "4444".into(),
            "-pix_fmt".into(),
            if alpha { "ayuv64le" } else { "p416le" }.into(),
        ]),
    }
    values.extend([
        "-color_primaries".into(),
        "bt709".into(),
        "-color_trc".into(),
        "iec61966-2-1".into(),
        "-colorspace".into(),
        "bt709".into(),
        "-color_range".into(),
        "tv".into(),
        "-movflags".into(),
        "+faststart".into(),
    ]);
    if let Some(tc) = spec
        .timecode
        .as_ref()
        .filter(|_| spec.container == Container::Mov)
    {
        values.extend(["-timecode".into(), tc.format.format(tc.start_frame)]);
        if let Some(reel) = &tc.reel {
            values.extend(["-metadata:s:v:0".into(), format!("reel_name={reel}")]);
        }
    }
    values.push("-y".into());
    let mut args: Vec<std::ffi::OsString> = values.into_iter().map(Into::into).collect();
    args.push(output.as_os_str().to_owned());
    args
}

struct Process {
    child: Child,
    input: Option<ChildStdin>,
    errors: Option<JoinHandle<Vec<u8>>>,
}
impl Process {
    fn finish(&mut self, cancel: Option<&AtomicBool>) -> Result<(), Error> {
        self.input.take();
        loop {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(Error::Cancelled);
            }
            if let Some(status) = self.child.try_wait().map_err(io)? {
                let errors = self
                    .errors
                    .take()
                    .and_then(|t| t.join().ok())
                    .unwrap_or_default();
                return if status.success() {
                    Ok(())
                } else {
                    Err(Error::Encode(String::from_utf8_lossy(&errors).into_owned()))
                };
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(errors) = self.errors.take() {
            let _ = errors.join();
        }
    }
}

pub struct LegacyVideoSink {
    // Stop the subprocess before deleting either of its files.
    process: Process,
    kind: Desktop,
    mux: Option<LegacyMux>,
    output: AtomicOutput,
    spec: ExportSpec,
    sampled: Vec<u64>,
    blocks: Vec<BlockSample>,
    capture: Option<File>,
    rgba8: Vec<u8>,
}
impl LegacyVideoSink {
    pub fn new(spec: &ExportSpec) -> Result<Self, Error> {
        spec.validate()?;
        let kind = Desktop::of(spec).ok_or_else(|| {
            Error::InvalidSpec("this format is not one of the desktop exporter's".into())
        })?;
        let output = match spec.overwrite {
            Overwrite::InPlace => {
                AtomicOutput::in_place(&spec.destination, spec.container.extension(), spec.mode)?
            }
            _ => AtomicOutput::new(&spec.destination, spec.overwrite, spec.mode)?,
        };
        let mut mux = spec
            .audio
            .map(|audio| LegacyMux::new(output.temp_path(), audio))
            .transpose()?;
        if let Some(mux) = mux.as_mut() {
            mux.set_reel(spec.timecode.as_ref().and_then(|tc| tc.reel.as_deref()));
            mux.set_video(spec.frames, spec.rate);
        }
        let video = mux.as_ref().map_or(output.temp_path(), |m| m.video_path());
        let mut child = Command::new(bundled_ffmpeg()?)
            .args(args(spec, kind, video))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(io)?;
        let input = child.stdin.take();
        let mut stderr = child.stderr.take().expect("piped stderr");
        let errors = std::thread::spawn(move || {
            let mut tail = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&chunk[..n]);
                if tail.len() > 8192 {
                    tail.drain(..tail.len() - 8192);
                }
            }
            tail
        });
        let process = Process {
            child,
            input,
            errors: Some(errors),
        };
        let capture = if std::env::var("POWERMOVE_BACKGROUND_TEST").as_deref() == Ok("1") {
            std::env::var_os("PM_EXPORT_INPUT_CAPTURE_DIR")
                .map(|dir| {
                    OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .mode(0o600)
                        .open(std::path::PathBuf::from(dir).join("native.rgba"))
                        .map_err(io)
                })
                .transpose()?
        } else {
            None
        };
        Ok(Self {
            process,
            kind,
            mux,
            output,
            spec: spec.clone(),
            sampled: verify::sampled_frames(spec.frames),
            blocks: vec![],
            capture,
            rgba8: Vec::new(),
        })
    }
    fn complete(mut self, cancel: Option<&AtomicBool>) -> Result<Delivered, Error> {
        self.process.finish(cancel)?;
        let audio_frames = if let Some(mux) = &mut self.mux {
            mux.finish(cancel)?;
            mux.frames()
        } else {
            0
        };
        let timing = self.spec.timing()?;
        let temp = self.output.temp_path();
        let movie = crate::mp4box::Movie::read(temp)?;
        let decoded = self
            .sampled
            .iter()
            .map(|&n| {
                reader::decode_frame(temp, n, timing, self.spec.input_format()).map(|f| (n, f))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut expected = Expected::new(
            &self.spec,
            timing,
            audio_frames,
            std::mem::take(&mut self.blocks),
        );
        if self.mux.is_none() {
            // The encoder's time base is the frame duration, den/num reduced.
            let (num, den) = (
                self.spec.rate.num().unsigned_abs(),
                self.spec.rate.den().unsigned_abs(),
            );
            expected.movie_timescale = Some((num / gcd(num, den)).max(1000));
        }
        let mut report = verify::check(&expected, &movie, &decoded)?;
        report.bytes = std::fs::metadata(temp).map_err(io)?.len();
        let path = self.output.commit()?;
        Ok(Delivered { path, report })
    }
}
impl Sink for LegacyVideoSink {
    type Output = Delivered;
    fn video_rgba(&mut self, index: u64, frame: &WorkingFrame) -> Result<bool, Error> {
        // Today's readback: opaque frames over black at full alpha
        // (`renderOpaqueFrame`), straight-alpha frames through `alphaFrame`'s
        // truncating unpremultiply and the Canvas2D round trip.
        let alpha = matches!(self.kind, Desktop::ProRes4444 { alpha: true });
        convert_rgba8_into(
            frame,
            &ConvertParams {
                working: self.spec.working,
                alpha: if alpha {
                    AlphaOutput::Straight
                } else {
                    AlphaOutput::None
                },
                dither: false,
                frame: index as u32,
            },
            &mut self.rgba8,
        )?;
        let bytes = &self.rgba8;
        if let Some(capture) = &mut self.capture {
            capture.write_all(bytes).map_err(io)?;
        }
        self.process
            .input
            .as_mut()
            .expect("encoder open until finish")
            .write_all(bytes)
            .map_err(io)?;
        if self.sampled.contains(&index) {
            // Verify against the frame the encoder was given: the RGBA8
            // bytes (already output-encoded, straight when alpha is kept).
            let given: Vec<f32> = bytes
                .chunks_exact(4)
                .flat_map(|p| {
                    let a = f32::from(p[3]) / 255.0;
                    [
                        f32::from(p[0]) / 255.0 * a,
                        f32::from(p[1]) / 255.0 * a,
                        f32::from(p[2]) / 255.0 * a,
                        a,
                    ]
                })
                .collect();
            let reference = convert(
                &WorkingFrame {
                    width: frame.width,
                    height: frame.height,
                    rgba: &given,
                },
                self.spec.input_format(),
                &ConvertParams {
                    working: WorkingEncoding::Standard,
                    alpha: self.spec.alpha,
                    dither: false,
                    frame: index as u32,
                },
            )?;
            self.blocks.extend(verify::block_means(&reference, index));
        }
        Ok(true)
    }
    fn video(&mut self, _: u64, _: &EncoderInput) -> Result<(), Error> {
        Err(Error::InvalidFrame(
            "desktop H.264 conversion requires original RGBA working frames".into(),
        ))
    }
    fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        self.mux
            .as_mut()
            .ok_or_else(|| Error::InvalidFrame("export has no audio track".into()))?
            .push(interleaved)
    }
    fn finish(self) -> Result<Delivered, Error> {
        self.complete(None)
    }
    fn finish_with_cancel(self, cancel: &AtomicBool) -> Result<Delivered, Error> {
        self.complete(Some(cancel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spec(codec: Codec, container: Container, alpha: AlphaOutput) -> ExportSpec {
        ExportSpec {
            width: 320,
            height: 180,
            rate: pm_time::Rate::FPS_29_97,
            frames: 30,
            codec,
            container,
            tags: ColorTags::Rec709QuickTime,
            alpha,
            working: WorkingEncoding::Standard,
            dither: Dither::On,
            audio: Some(AudioSpec::default_for(container, 48_000, 2)),
            timecode: None,
            destination: PathBuf::from("/tmp/out"),
            overwrite: Overwrite::InPlace,
            mode: None,
        }
    }

    fn strings(args: Vec<std::ffi::OsString>) -> Vec<String> {
        args.into_iter().map(|a| a.into_string().unwrap()).collect()
    }

    /// `render-encoder.ts` `encoderArgs` for `{ width: 320, height: 180,
    /// fps: 30000 / 1001, format, alpha, bitrateMbps: 16 }`.
    fn today(codec: &[&str]) -> Vec<String> {
        let mut args = vec![
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            "320x180",
            "-framerate",
            "29.97002997002997",
            "-i",
            "pipe:0",
            "-an",
            "-vf",
            "scale=in_range=full:out_range=limited:out_color_matrix=bt709,setparams=color_primaries=bt709:color_trc=iec61966-2-1:colorspace=bt709:range=tv",
        ];
        args.extend_from_slice(codec);
        args.extend_from_slice(&[
            "-color_primaries",
            "bt709",
            "-color_trc",
            "iec61966-2-1",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
            "-movflags",
            "+faststart",
        ]);
        args.into_iter().map(str::to_owned).collect()
    }

    #[test]
    fn todays_formats_use_todays_ffmpeg_arguments() {
        let out = std::path::Path::new("/tmp/out.mp4");
        let h264 = spec(
            Codec::H264 {
                bitrate_bps: 16_000_000,
            },
            Container::Mp4,
            AlphaOutput::None,
        );
        assert_eq!(Desktop::of(&h264), Some(Desktop::H264));
        let mut want = today(&[
            "-c:v",
            "h264_videotoolbox",
            "-allow_sw",
            "1",
            "-profile:v",
            "high",
            "-pix_fmt",
            "yuv420p",
            "-b:v",
            "16M",
            "-g",
            "250",
        ]);
        want.extend(["-y".into(), "/tmp/out.mp4".into()]);
        assert_eq!(strings(args(&h264, Desktop::H264, out)), want);

        for (alpha, pix) in [
            (AlphaOutput::None, "p416le"),
            (AlphaOutput::Straight, "ayuv64le"),
        ] {
            let mut prores = spec(Codec::ProRes(ProResProfile::P4444), Container::Mov, alpha);
            let kind = Desktop::of(&prores).unwrap();
            assert_eq!(
                kind,
                Desktop::ProRes4444 {
                    alpha: alpha == AlphaOutput::Straight
                }
            );
            let mut want = today(&[
                "-c:v",
                "prores_videotoolbox",
                "-allow_sw",
                "1",
                "-profile:v",
                "4444",
                "-pix_fmt",
                pix,
            ]);
            want.extend(["-y".into(), "/tmp/out.mp4".into()]);
            assert_eq!(strings(args(&prores, kind, out)), want);
            // The delivery's start timecode and reel follow today's arguments.
            let format = pm_time::TimecodeFormat::drop_frame(prores.rate).unwrap();
            prores.timecode = Some(TimecodeSpec {
                start_frame: format.parse("01:00:00;00").unwrap(),
                format,
                reel: Some("A001".into()),
            });
            let got = strings(args(&prores, kind, out));
            assert_eq!(
                &got[got.len() - 6..],
                [
                    "-timecode",
                    "01:00:00;00",
                    "-metadata:s:v:0",
                    "reel_name=A001",
                    "-y",
                    "/tmp/out.mp4"
                ]
            );
        }
    }

    #[test]
    fn other_formats_keep_videotoolbox() {
        let premultiplied = spec(
            Codec::ProRes(ProResProfile::P4444),
            Container::Mov,
            AlphaOutput::Premultiplied,
        );
        assert_eq!(Desktop::of(&premultiplied), None);
        let hq = spec(
            Codec::ProRes(ProResProfile::Hq),
            Container::Mov,
            AlphaOutput::None,
        );
        assert_eq!(Desktop::of(&hq), None);
        let mut broadcast = spec(
            Codec::H264 {
                bitrate_bps: 16_000_000,
            },
            Container::Mp4,
            AlphaOutput::None,
        );
        broadcast.tags = ColorTags::Rec709Broadcast;
        assert_eq!(Desktop::of(&broadcast), None);
        let mut pcm = spec(
            Codec::H264 {
                bitrate_bps: 16_000_000,
            },
            Container::Mp4,
            AlphaOutput::None,
        );
        pcm.audio = Some(AudioSpec {
            sample_rate: 48_000,
            channels: 2,
            codec: AudioCodec::Pcm16,
        });
        assert_eq!(Desktop::of(&pcm), None);
    }
}
