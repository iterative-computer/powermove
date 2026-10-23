//! Opaque WebM through the host's Chromium WebCodecs encoders (decision
//! EXPORT-1; `pm_protocol::host_encode`).
//!
//! Today's exporter encodes WebM with Chromium's own `VideoEncoder` and
//! `AudioEncoder`. Their libvpx and libopus builds and settings are
//! Chromium's (pinned revisions, its GPU canvas readback to REC709 I420, its
//! rate control), so no other encoder gives the same pictures and samples.
//! When the job engine runs inside the desktop app, this sink renders and
//! mixes as usual and hands each frame's canvas bytes (RGBA8, the unorm
//! render-target write, [`crate::render::to_rgba8`]) and the mix to an
//! [`EncodeHost`]: the app's encoder window, which runs today's encoder code
//! and muxer and writes the file into the export's output. The sink then
//! verifies the file like any WebM export ([`crate::webm::verify_webm`]).
//!
//! Alpha WebM (which today's exporter does not write) and exports without a
//! host (the CLI) keep the libvpx/libopus sink ([`crate::webm::WebmSink`]).

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use crate::convert::{EncoderInput, WorkingFrame};
use crate::pipeline::Sink;
use crate::settings::{AlphaOutput, AudioSpec, Codec, Container, ExportSpec, Overwrite};
use crate::verify::{self, BlockSample};
use crate::{Delivered, Error};

/// What an encode is: today's WebCodecs settings follow from these.
#[derive(Clone, Debug, PartialEq)]
pub struct EncodeOpen {
    /// VP9 or VP8 (and the quality tier's bit rate).
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub rate: pm_time::Rate,
    pub frames: u64,
    pub audio: Option<AudioSpec>,
}

/// The host side of an encode: the desktop app's WebCodecs encoder window.
/// Calls come in order: `open`, `video` for frames 0, 1, …, `audio` chunks
/// in between, then `finish` (the host writes the file to the export's
/// destination) or `abort`.
pub trait EncodeHost: Send {
    fn open(&mut self, open: &EncodeOpen) -> Result<(), Error>;
    /// Frame `index`: top-down RGBA8, premultiplied (over black where
    /// transparent), `width × height × 4` bytes.
    fn video(&mut self, index: u64, rgba: Vec<u8>) -> Result<(), Error>;
    /// Interleaved samples of the mix, in order.
    fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error>;
    /// Flushes the encoders and writes the muxed file to the destination.
    fn finish(&mut self) -> Result<(), Error>;
    /// Abandons the encode. Called at most once, never after `finish`.
    fn abort(&mut self);
}

/// Whether `spec` is encoded by an [`EncodeHost`] when one is available:
/// an opaque VP9 or VP8 WebM written in place into a host output.
pub fn uses_host(spec: &ExportSpec) -> bool {
    spec.container == Container::Webm
        && spec.alpha == AlphaOutput::None
        && matches!(spec.codec, Codec::Vp9 { .. } | Codec::Vp8 { .. })
        && spec.overwrite == Overwrite::InPlace
        && spec
            .audio
            .is_none_or(|a| a.codec == crate::settings::AudioCodec::Opus)
}

/// Opt-in parity capture of the exact boundaries handed to the encoders
/// (`POWERMOVE_BACKGROUND_TEST=1` and `PM_EXPORT_INPUT_CAPTURE_DIR`); the
/// harness that sets the directory owns it and removes it.
struct Capture {
    rgba: File,
    pcm: File,
}

impl Capture {
    fn from_env() -> Result<Option<Self>, Error> {
        if std::env::var("POWERMOVE_BACKGROUND_TEST").as_deref() != Ok("1") {
            return Ok(None);
        }
        let Some(dir) = std::env::var_os("PM_EXPORT_INPUT_CAPTURE_DIR") else {
            return Ok(None);
        };
        let dir = PathBuf::from(dir);
        let open = |name: &str| {
            File::create(dir.join(name))
                .map_err(|e| Error::Io(format!("open the WebM input capture {name}: {e}")))
        };
        Ok(Some(Self {
            rgba: open("native.rgba")?,
            pcm: open("native.f32")?,
        }))
    }
}

/// [`crate::render::to_rgba8`] of a frame, row-parallel at the caller's QoS
/// (a 1080p frame is 8.3 million samples).
fn canvas_bytes(frame: &WorkingFrame) -> Vec<u8> {
    let row = frame.width as usize * 4;
    let mut out = vec![0u8; frame.rgba.len()];
    crate::par::rows(&mut out, row, |y, dst| {
        let src = &frame.rgba[y * row..(y + 1) * row];
        for (d, v) in dst.iter_mut().zip(src) {
            let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
            *d = (v * 255.0).round_ties_even() as u8;
        }
    });
    out
}

/// The sink: renders go to the host's encoders, then the file is verified.
pub struct WebCodecsSink {
    host: Box<dyn EncodeHost>,
    spec: ExportSpec,
    sampled: Vec<u64>,
    blocks: Vec<BlockSample>,
    audio_pushed: u64,
    /// Set once `finish` or `abort` reached the host.
    closed: bool,
    capture: Option<Capture>,
}

impl WebCodecsSink {
    pub fn new(spec: &ExportSpec, mut host: Box<dyn EncodeHost>) -> Result<Self, Error> {
        spec.validate()?;
        if !uses_host(spec) {
            return Err(Error::InvalidSpec(
                "only opaque VP9/VP8 WebM written in place goes to the host's encoders".into(),
            ));
        }
        let capture = Capture::from_env()?;
        host.open(&EncodeOpen {
            codec: spec.codec,
            width: spec.width,
            height: spec.height,
            rate: spec.rate,
            frames: spec.frames,
            audio: spec.audio,
        })?;
        Ok(Self {
            host,
            spec: spec.clone(),
            sampled: verify::sampled_frames(spec.frames),
            blocks: vec![],
            audio_pushed: 0,
            closed: false,
            capture,
        })
    }

    fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.host.abort();
        }
    }
}

impl Drop for WebCodecsSink {
    fn drop(&mut self) {
        self.close();
    }
}

impl Sink for WebCodecsSink {
    type Output = Delivered;

    fn video_rgba(&mut self, index: u64, frame: &WorkingFrame) -> Result<bool, Error> {
        let rgba = canvas_bytes(frame);
        if let Some(capture) = self.capture.as_mut() {
            capture
                .rgba
                .write_all(&rgba)
                .map_err(|e| Error::Io(format!("write the WebM RGBA capture: {e}")))?;
        }
        if self.sampled.contains(&index) {
            // Verification compares decoded blocks with Chromium's REC709
            // conversion of the same bytes.
            let input = crate::webm::canvas_rgba_to_i420(&rgba, frame.width, frame.height, false)?;
            self.blocks.extend(verify::block_means(&input, index));
        }
        self.host.video(index, rgba).inspect_err(|_| self.close())?;
        Ok(true)
    }

    fn video(&mut self, _: u64, _: &EncoderInput) -> Result<(), Error> {
        Err(Error::InvalidFrame(
            "the WebCodecs encoders take the rendered RGBA frames".into(),
        ))
    }

    fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        let Some(audio) = self.spec.audio else {
            return Err(Error::InvalidFrame("this export has no audio track".into()));
        };
        let channels = usize::from(audio.channels);
        if !interleaved.len().is_multiple_of(channels) {
            return Err(Error::InvalidFrame(
                "audio samples are not a whole number of frames".into(),
            ));
        }
        if let Some(capture) = self.capture.as_mut() {
            let bytes: Vec<u8> = interleaved.iter().flat_map(|s| s.to_le_bytes()).collect();
            capture
                .pcm
                .write_all(&bytes)
                .map_err(|e| Error::Io(format!("write the WebM PCM capture: {e}")))?;
        }
        self.audio_pushed += (interleaved.len() / channels) as u64;
        self.host.audio(interleaved).inspect_err(|_| self.close())
    }

    fn finish(mut self) -> Result<Delivered, Error> {
        self.closed = true;
        self.host.finish()?;
        let path = self.spec.destination.clone();
        let mut report = crate::webm::verify_webm(
            &path,
            &self.spec,
            &std::mem::take(&mut self.blocks),
            self.audio_pushed,
        )?;
        report.bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Ok(Delivered { path, report })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Log(Vec<String>);

    struct Recorder(Arc<Mutex<Log>>);

    impl EncodeHost for Recorder {
        fn open(&mut self, open: &EncodeOpen) -> Result<(), Error> {
            self.0.lock().unwrap().0.push(format!(
                "open {}x{} {}",
                open.width, open.height, open.frames
            ));
            Ok(())
        }
        fn video(&mut self, index: u64, rgba: Vec<u8>) -> Result<(), Error> {
            self.0
                .lock()
                .unwrap()
                .0
                .push(format!("video {index} {:?}", &rgba[..4]));
            if index == 1 {
                return Err(Error::Encode("the encoder window closed".into()));
            }
            Ok(())
        }
        fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
            self.0
                .lock()
                .unwrap()
                .0
                .push(format!("audio {}", interleaved.len()));
            Ok(())
        }
        fn finish(&mut self) -> Result<(), Error> {
            self.0.lock().unwrap().0.push("finish".into());
            Ok(())
        }
        fn abort(&mut self) {
            self.0.lock().unwrap().0.push("abort".into());
        }
    }

    fn spec(alpha: AlphaOutput) -> ExportSpec {
        ExportSpec {
            width: 4,
            height: 4,
            rate: pm_time::Rate::FPS_30,
            frames: 2,
            codec: Codec::Vp9 {
                bitrate_bps: 16_000_000,
            },
            container: Container::Webm,
            tags: crate::settings::ColorTags::Rec709QuickTime,
            alpha,
            working: crate::settings::WorkingEncoding::Standard,
            dither: crate::settings::Dither::Default,
            audio: Some(AudioSpec::default_for(Container::Webm, 48_000, 2)),
            timecode: None,
            destination: PathBuf::from("/tmp/never-written.webm"),
            overwrite: Overwrite::InPlace,
            mode: None,
        }
    }

    #[test]
    fn frames_go_to_the_host_as_canvas_bytes_and_failures_abort_once() {
        let log = Arc::new(Mutex::new(Log::default()));
        let s = spec(AlphaOutput::None);
        assert!(uses_host(&s));
        let mut sink = WebCodecsSink::new(&s, Box::new(Recorder(log.clone()))).unwrap();
        let rgba = [0.5f32, 0.25, 1.0, 1.0].repeat(16);
        let frame = WorkingFrame {
            rgba: &rgba,
            width: 4,
            height: 4,
        };
        assert!(sink.video_rgba(0, &frame).unwrap());
        sink.audio(&[0.0; 1920]).unwrap();
        assert!(sink.audio(&[0.0; 3]).is_err(), "half a stereo frame");
        assert!(sink.video_rgba(1, &frame).is_err());
        drop(sink);
        assert_eq!(
            log.lock().unwrap().0,
            [
                "open 4x4 2",
                "video 0 [128, 64, 255, 255]",
                "audio 1920",
                "video 1 [128, 64, 255, 255]",
                "abort",
            ]
        );
    }

    #[test]
    fn canvas_bytes_are_the_unorm_writes() {
        let rgba: Vec<f32> = (0..64 * 4)
            .map(|i| (i as f32 / 255.0) * 0.999 - 0.01)
            .collect();
        let frame = WorkingFrame {
            rgba: &rgba,
            width: 8,
            height: 8,
        };
        assert_eq!(canvas_bytes(&frame), crate::render::to_rgba8(&rgba));
    }

    #[test]
    fn alpha_and_new_files_keep_the_libvpx_sink() {
        assert!(!uses_host(&spec(AlphaOutput::Straight)));
        let mut s = spec(AlphaOutput::None);
        s.overwrite = Overwrite::Replace;
        assert!(!uses_host(&s));
        let log = Arc::new(Mutex::new(Log::default()));
        assert!(WebCodecsSink::new(&s, Box::new(Recorder(log.clone()))).is_err());
        assert!(log.lock().unwrap().0.is_empty());
    }
}
