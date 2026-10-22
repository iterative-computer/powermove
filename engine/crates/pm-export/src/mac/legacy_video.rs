//! H.264 job delivery through the desktop exporter's exact RGB conversion and
//! bundled FFmpeg VideoToolbox settings. Never reconstruct RGBA from encoded YUV.
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
use crate::convert::{ConvertParams, EncoderInput, WorkingFrame, convert};
use crate::pipeline::Sink;
use crate::settings::*;
use crate::verify::{self, BlockSample, Expected};
use crate::{Delivered, Error};

fn io(error: std::io::Error) -> Error {
    Error::Io(error.to_string())
}

/// Exactly `render-encoder.ts::encoderArgs` for the MP4 path.
fn args(spec: &ExportSpec, output: &std::path::Path) -> Vec<std::ffi::OsString> {
    let Codec::H264 { bitrate_bps } = spec.codec else {
        unreachable!()
    };
    let fps = spec.rate.num() as f64 / spec.rate.den() as f64;
    let values = vec![
        "-hide_banner".into(), "-loglevel".into(), "error".into(), "-f".into(), "rawvideo".into(),
        "-pixel_format".into(), "rgba".into(), "-video_size".into(), format!("{}x{}", spec.width, spec.height),
        "-framerate".into(), fps.to_string(), "-i".into(), "pipe:0".into(), "-an".into(),
        "-vf".into(), "scale=in_range=full:out_range=limited:out_color_matrix=bt709,setparams=color_primaries=bt709:color_trc=iec61966-2-1:colorspace=bt709:range=tv".into(),
        "-c:v".into(), "h264_videotoolbox".into(), "-allow_sw".into(), "1".into(),
        "-profile:v".into(), "high".into(), "-pix_fmt".into(), "yuv420p".into(),
        "-b:v".into(), format!("{}M", bitrate_bps as f64 / 1_000_000.0), "-g".into(), "250".into(),
        "-color_primaries".into(), "bt709".into(), "-color_trc".into(), "iec61966-2-1".into(),
        "-colorspace".into(), "bt709".into(), "-color_range".into(), "tv".into(),
        "-movflags".into(), "+faststart".into(), "-y".into(),
    ];
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
    mux: Option<LegacyMux>,
    output: AtomicOutput,
    spec: ExportSpec,
    sampled: Vec<u64>,
    blocks: Vec<BlockSample>,
    capture: Option<File>,
}
impl LegacyVideoSink {
    pub fn new(spec: &ExportSpec) -> Result<Self, Error> {
        spec.validate()?;
        let output = match spec.overwrite {
            Overwrite::InPlace => {
                AtomicOutput::in_place(&spec.destination, spec.container.extension(), spec.mode)?
            }
            _ => AtomicOutput::new(&spec.destination, spec.overwrite, spec.mode)?,
        };
        let mux = spec
            .audio
            .map(|audio| LegacyMux::new(output.temp_path(), audio))
            .transpose()?;
        let video = mux.as_ref().map_or(output.temp_path(), |m| m.video_path());
        let mut child = Command::new(bundled_ffmpeg()?)
            .args(args(spec, video))
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
            mux,
            output,
            spec: spec.clone(),
            sampled: verify::sampled_frames(spec.frames),
            blocks: vec![],
            capture,
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
        let expected = Expected::new(
            &self.spec,
            timing,
            audio_frames,
            std::mem::take(&mut self.blocks),
        );
        let mut report = verify::check(&expected, &movie, &decoded)?;
        report.bytes = std::fs::metadata(temp).map_err(io)?.len();
        let path = self.output.commit()?;
        Ok(Delivered { path, report })
    }
}
impl Sink for LegacyVideoSink {
    type Output = Delivered;
    fn video_rgba(&mut self, index: u64, frame: &WorkingFrame) -> Result<bool, Error> {
        let rgba = convert(
            frame,
            InputFormat::Rgba8,
            &ConvertParams {
                working: self.spec.working,
                alpha: AlphaOutput::None,
                dither: false,
                frame: index as u32,
                legacy_png: false,
            },
        )?;
        let bytes = rgba.planes[0].u8().expect("RGBA8");
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
            let reference = convert(
                frame,
                self.spec.input_format(),
                &ConvertParams {
                    working: self.spec.working,
                    alpha: self.spec.alpha,
                    dither: false,
                    frame: index as u32,
                    legacy_png: false,
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
