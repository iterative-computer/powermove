//! The current desktop exporter writes a 16-bit WAV from OfflineAudioContext,
//! then asks Powermove's bundled FFmpeg to copy the video and encode the audio.
//! The WAV's quantization is observable even when the MOV asks for 24-bit PCM.

use std::ffi::CStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::Error;
use crate::settings::{AudioCodec, AudioSpec};

const WAV_HEADER: u64 = 80;

fn io(what: &str, e: std::io::Error) -> Error {
    Error::Io(format!("{what}: {e}"))
}

/// Resolve the executable beside the libavcodec dylib already loaded by this
/// process. This also works in the packaged app, where PM_FFMPEG_DIR is unset,
/// and guarantees the CLI encoder has the same revision as pm-media's dylibs.
pub(super) fn bundled_ffmpeg() -> Result<PathBuf, Error> {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    let address = pm_media::sys::ffmpeg::avcodec_version as *const () as *const libc::c_void;
    if unsafe { libc::dladdr(address, &mut info) } == 0 || info.dli_fname.is_null() {
        return Err(Error::Encode(
            "cannot locate the bundled FFmpeg library".into(),
        ));
    }
    let library = Path::new(std::ffi::OsStr::from_bytes(unsafe {
        CStr::from_ptr(info.dli_fname).to_bytes()
    }));
    let binary = library
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("bin/ffmpeg"))
        .ok_or_else(|| Error::Encode("invalid bundled FFmpeg library path".into()))?;
    if !binary.is_file() {
        return Err(Error::Encode(format!(
            "the bundled FFmpeg executable is missing: {}",
            binary.display()
        )));
    }
    Ok(binary)
}

/// The 80-byte header reserves a ds64 chunk, allowing RF64 above 4 GiB without
/// ever moving samples already written to disk. FFmpeg decodes this to the same
/// PCM16 samples as the legacy exporter's 44-byte RIFF WAV.
fn wav_header(channels: u16, rate: u32, frames: u64) -> [u8; WAV_HEADER as usize] {
    let data = frames * u64::from(channels) * 2;
    let riff_size = WAV_HEADER - 8 + data;
    let rf64 = riff_size > u32::MAX as u64 || data > u32::MAX as u64;
    let mut h = [0u8; WAV_HEADER as usize];
    h[0..4].copy_from_slice(if rf64 { b"RF64" } else { b"RIFF" });
    h[4..8].copy_from_slice(&(if rf64 { u32::MAX } else { riff_size as u32 }).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(if rf64 { b"ds64" } else { b"JUNK" });
    h[16..20].copy_from_slice(&28u32.to_le_bytes());
    if rf64 {
        h[20..28].copy_from_slice(&riff_size.to_le_bytes());
        h[28..36].copy_from_slice(&data.to_le_bytes());
        h[36..44].copy_from_slice(&frames.to_le_bytes());
    }
    h[48..52].copy_from_slice(b"fmt ");
    h[52..56].copy_from_slice(&16u32.to_le_bytes());
    h[56..58].copy_from_slice(&1u16.to_le_bytes());
    h[58..60].copy_from_slice(&channels.to_le_bytes());
    h[60..64].copy_from_slice(&rate.to_le_bytes());
    h[64..68].copy_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    h[68..70].copy_from_slice(&(channels * 2).to_le_bytes());
    h[70..72].copy_from_slice(&16u16.to_le_bytes());
    h[72..76].copy_from_slice(b"data");
    h[76..80].copy_from_slice(&(if rf64 { u32::MAX } else { data as u32 }).to_le_bytes());
    h
}

struct LegacyWav {
    file: File,
    spec: AudioSpec,
    frames: u64,
}

impl LegacyWav {
    fn new(path: &Path, spec: AudioSpec) -> Result<Self, Error> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| io("create export audio WAV", e))?;
        file.write_all(&wav_header(spec.channels, spec.sample_rate, 0))
            .map_err(|e| io("write export audio WAV header", e))?;
        Ok(Self {
            file,
            spec,
            frames: 0,
        })
    }

    fn push(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        let channels = usize::from(self.spec.channels);
        if !interleaved.len().is_multiple_of(channels) {
            return Err(Error::InvalidFrame(
                "audio samples are not a whole number of frames".into(),
            ));
        }
        let mut bytes = Vec::with_capacity(interleaved.len() * 2);
        for &sample in interleaved {
            // render-queue.ts wavBytes: clamp, multiply negative samples by
            // 32768 and nonnegative samples by 32767, then JS Math.round
            // (ties toward +infinity). NaN becomes zero in DataView.setInt16.
            let value = if sample.is_nan() {
                0.0
            } else {
                sample.clamp(-1.0, 1.0) as f64
            };
            let scaled = value * if value < 0.0 { 32768.0 } else { 32767.0 };
            let quantized = (scaled + 0.5).floor() as i16;
            bytes.extend_from_slice(&quantized.to_le_bytes());
        }
        self.file
            .write_all(&bytes)
            .map_err(|e| io("write export audio WAV", e))?;
        self.frames = self
            .frames
            .checked_add((interleaved.len() / channels) as u64)
            .ok_or_else(|| Error::InvalidFrame("too many audio samples".into()))?;
        Ok(())
    }

    fn finish(mut self) -> Result<u64, Error> {
        self.file
            .seek(SeekFrom::Start(0))
            .and_then(|_| {
                self.file.write_all(&wav_header(
                    self.spec.channels,
                    self.spec.sample_rate,
                    self.frames,
                ))
            })
            .and_then(|_| self.file.flush())
            .map_err(|e| io("finish export audio WAV", e))?;
        Ok(self.frames)
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Temporary video and WAV sidecars live beside AtomicOutput's own work file.
/// Dropping this guard removes both on success, error, or cancellation.
pub(super) struct LegacyMux {
    video: PathBuf,
    audio: PathBuf,
    output: PathBuf,
    wav: Option<LegacyWav>,
    codec: AudioCodec,
    spec_rate: u32,
    frames: u64,
    reel: Option<String>,
    /// The video's frame count and rate, for `-shortest`.
    track: Option<(u64, pm_time::Rate)>,
}

impl LegacyMux {
    pub fn new(output: &Path, spec: AudioSpec) -> Result<Self, Error> {
        let mut video = output.as_os_str().to_os_string();
        video.push(format!(
            ".video.{}",
            output.extension().and_then(|x| x.to_str()).unwrap_or("mov")
        ));
        let video = PathBuf::from(video);
        let mut audio = output.as_os_str().to_os_string();
        audio.push(".audio.wav");
        let audio = PathBuf::from(audio);
        let mut mux = Self {
            video,
            audio,
            output: output.to_path_buf(),
            wav: None,
            codec: spec.codec,
            spec_rate: spec.sample_rate,
            frames: 0,
            reel: None,
            track: None,
        };
        mux.wav = Some(LegacyWav::new(&mux.audio, spec)?);
        Ok(mux)
    }

    pub fn video_path(&self) -> &Path {
        &self.video
    }

    /// FFmpeg regenerates tmcd from the video's timecode metadata. Its MOV
    /// muxer reads the source video stream's `reel_name` for the tmcd `name` box.
    pub fn set_reel(&mut self, reel: Option<&str>) {
        self.reel = reel.map(str::to_owned);
    }

    /// The video track the audio is muxed with (`frames` at `rate`).
    pub fn set_video(&mut self, frames: u64, rate: pm_time::Rate) {
        self.track = Some((frames, rate));
    }

    /// Today's mux passes `-shortest`, which ends the file with the shorter
    /// track. When today's mix (`ceil(range × 48000)` samples) is shorter
    /// than the frames (29.97 fps exports of whole seconds), that drops the
    /// last frame: a missing frame in today's export (decision COMPCLOSE-1:
    /// such bugs are fixed, not reproduced). Then the option is left out and
    /// both tracks keep their length; otherwise it trims the audio as today.
    fn shortest(&self) -> bool {
        let Some((frames, rate)) = self.track else {
            return true;
        };
        let sample_rate = i128::from(self.spec_rate);
        // audio / sample_rate >= frames × den / num
        i128::from(self.frames) * i128::from(rate.num())
            >= i128::from(frames) * i128::from(rate.den()) * sample_rate
    }

    pub fn push(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        self.wav
            .as_mut()
            .expect("WAV open until mux")
            .push(interleaved)
    }

    pub fn frames(&self) -> u64 {
        self.wav.as_ref().map_or(self.frames, |w| w.frames)
    }

    pub fn finish(&mut self, cancel: Option<&AtomicBool>) -> Result<(), Error> {
        let wav = self.wav.take().expect("WAV open until mux");
        self.frames = wav.finish()?;
        if self.frames == 0 {
            return Err(Error::InvalidFrame("audio track has no samples".into()));
        }
        if std::env::var_os("POWERMOVE_BACKGROUND_TEST").as_deref()
            == Some(std::ffi::OsStr::new("1"))
            && let Some(dir) = std::env::var_os("PM_EXPORT_INPUT_CAPTURE_DIR")
        {
            let dir = PathBuf::from(dir);
            fs::create_dir_all(&dir).map_err(|e| io("create export input capture directory", e))?;
            fs::copy(&self.audio, dir.join("native.wav"))
                .map_err(|e| io("capture native export PCM", e))?;
        }
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(Error::Cancelled);
        }
        let ffmpeg = bundled_ffmpeg()?;
        let codec = match self.codec {
            AudioCodec::Aac => "aac",
            AudioCodec::Pcm24 => "pcm_s24le",
            _ => unreachable!("only legacy codecs use this mux"),
        };
        let mut command = Command::new(ffmpeg);
        command
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&self.video)
            .arg("-i")
            .arg(&self.audio)
            .args(["-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", codec]);
        if let Some(reel) = &self.reel {
            command
                .arg("-metadata:s:v:0")
                .arg(format!("reel_name={reel}"));
        }
        if self.shortest() {
            command.arg("-shortest");
        }
        let mut child = ChildGuard(
            command
                .arg("-y")
                .arg(&self.output)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| io("start bundled FFmpeg audio mux", e))?,
        );
        let stderr = child.0.stderr.take().expect("piped stderr");
        let errors = std::thread::spawn(move || {
            use std::io::Read;
            let mut stderr = stderr;
            let mut tail = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(count) = stderr.read(&mut buf) {
                if count == 0 {
                    break;
                }
                tail.extend_from_slice(&buf[..count]);
                if tail.len() > 8192 {
                    tail.drain(..tail.len() - 8192);
                }
            }
            String::from_utf8_lossy(&tail).into_owned()
        });
        let status = loop {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(Error::Cancelled);
            }
            match child
                .0
                .try_wait()
                .map_err(|e| io("wait for FFmpeg audio mux", e))?
            {
                Some(status) => break status,
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        };
        let detail = errors.join().unwrap_or_default();
        if !status.success() {
            return Err(Error::Mux(format!(
                "bundled FFmpeg audio mux exited with {status}: {}",
                detail.trim()
            )));
        }
        Ok(())
    }
}

impl Drop for LegacyMux {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.video);
        let _ = fs::remove_file(&self.audio);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "pm-export-wav-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn legacy_quantization_rounds_negative_ties_up_and_clips() {
        let dir = TestDir::new();
        let path = dir.0.join("test.wav");
        let spec = AudioSpec {
            sample_rate: 48_000,
            channels: 2,
            codec: AudioCodec::Aac,
        };
        let mut wav = LegacyWav::new(&path, spec).unwrap();
        wav.push(&[1.0, 0.5, -1.0, -0.5, -0.5 / 32768.0, f32::NAN])
            .unwrap();
        wav.finish().unwrap();
        let bytes = fs::read(&path).unwrap();
        let pcm: Vec<i16> = bytes[WAV_HEADER as usize..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(pcm, [32767, 16384, -32768, -16384, 0, 0]);
    }

    fn run(binary: &Path, args: &[&str], input: &[&Path]) {
        let mut command = Command::new(binary);
        command.args(args);
        for path in input {
            command.arg(path);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{} failed: {}",
            binary.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn audio_probe(file: &Path) -> Value {
        let probe = bundled_ffmpeg().unwrap().with_file_name("ffprobe");
        let output = Command::new(probe)
            .args([
                "-v", "error", "-select_streams", "a:0", "-show_streams",
                "-show_packets", "-show_data_hash", "sha256", "-show_entries",
                "stream=codec_name,profile,sample_fmt,sample_rate,channels,channel_layout,time_base,start_pts,duration_ts,bits_per_raw_sample:packet=pts,dts,duration,size,data_hash",
                "-of", "json",
            ])
            .arg(file)
            .output()
            .unwrap();
        assert!(output.status.success());
        serde_json::from_slice(&output.stdout).unwrap()
    }

    #[test]
    fn ffmpeg_mux_matches_legacy_wav_packets_and_alignment() {
        let ffmpeg = bundled_ffmpeg().unwrap();
        for (extension, codec) in [("mov", AudioCodec::Pcm24), ("mp4", AudioCodec::Aac)] {
            let dir = TestDir::new();
            let output = dir.0.join(format!("native.{extension}"));
            let reference = dir.0.join(format!("reference.{extension}"));
            let reference_wav = dir.0.join("reference.wav");
            let spec = AudioSpec {
                sample_rate: 48_000,
                channels: 2,
                codec,
            };
            let mut mux = LegacyMux::new(&output, spec).unwrap();
            let samples: Vec<f32> = (0..24_000 * 2)
                .map(|n| ((n / 2) as f32 * 0.019).sin() * if n % 2 == 0 { 0.7 } else { 0.3 })
                .collect();
            for chunk in samples.chunks(4800) {
                mux.push(chunk).unwrap();
            }
            let mut video_args = vec![
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=64x64:r=24:d=0.5",
                "-c:v",
                "mpeg4",
                "-q:v",
                "2",
            ];
            if extension == "mov" {
                video_args.extend(["-timecode", "01:02:03:04"]);
            }
            video_args.push("-y");
            run(&ffmpeg, &video_args, &[mux.video_path()]);
            // Legacy render-queue.ts produces a 44-byte RIFF header. The
            // production streamer reserves ds64, but the decoder's PCM and
            // FFmpeg's encoded packets must be identical.
            let streamed = fs::read(&mux.audio).unwrap();
            let data = &streamed[WAV_HEADER as usize..];
            let mut legacy_header = [0u8; 44];
            legacy_header[0..4].copy_from_slice(b"RIFF");
            legacy_header[4..8].copy_from_slice(&(data.len() as u32 + 36).to_le_bytes());
            legacy_header[8..16].copy_from_slice(b"WAVEfmt ");
            legacy_header[16..20].copy_from_slice(&16u32.to_le_bytes());
            legacy_header[20..22].copy_from_slice(&1u16.to_le_bytes());
            legacy_header[22..24].copy_from_slice(&2u16.to_le_bytes());
            legacy_header[24..28].copy_from_slice(&48_000u32.to_le_bytes());
            legacy_header[28..32].copy_from_slice(&(48_000u32 * 4).to_le_bytes());
            legacy_header[32..34].copy_from_slice(&4u16.to_le_bytes());
            legacy_header[34..36].copy_from_slice(&16u16.to_le_bytes());
            legacy_header[36..40].copy_from_slice(b"data");
            legacy_header[40..44].copy_from_slice(&(data.len() as u32).to_le_bytes());
            let mut wav = File::create(&reference_wav).unwrap();
            wav.write_all(&legacy_header).unwrap();
            wav.write_all(data).unwrap();
            drop(wav);
            mux.finish(None).unwrap();
            let status = Command::new(&ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-i"])
                .arg(mux.video_path())
                .arg("-i")
                .arg(&reference_wav)
                .args(["-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a"])
                .arg(if codec == AudioCodec::Aac {
                    "aac"
                } else {
                    "pcm_s24le"
                })
                .args(["-shortest", "-y"])
                .arg(&reference)
                .output()
                .unwrap();
            assert!(
                status.status.success(),
                "{}",
                String::from_utf8_lossy(&status.stderr)
            );
            let native_audio = audio_probe(&output);
            let legacy_audio = audio_probe(&reference);
            assert_eq!(native_audio, legacy_audio, "{extension} audio differs");
            assert_eq!(native_audio["streams"][0]["channel_layout"], "stereo");
            assert!(
                native_audio["packets"]
                    .as_array()
                    .is_some_and(|p| !p.is_empty())
            );
            if extension == "mov" {
                let probe = bundled_ffmpeg().unwrap().with_file_name("ffprobe");
                let tc = Command::new(probe)
                    .args([
                        "-v",
                        "error",
                        "-select_streams",
                        "d:0",
                        "-show_entries",
                        "stream=codec_tag_string:stream_tags=timecode",
                        "-of",
                        "json",
                    ])
                    .arg(&output)
                    .output()
                    .unwrap();
                assert!(tc.status.success());
                let tracks: Value = serde_json::from_slice(&tc.stdout).unwrap();
                assert_eq!(tracks["streams"][0]["codec_tag_string"], "tmcd");
                assert_eq!(tracks["streams"][0]["tags"]["timecode"], "01:02:03:04");
            }
            let video = mux.video_path().to_path_buf();
            let audio = mux.audio.clone();
            drop(mux);
            assert!(!video.exists() && !audio.exists());
        }
    }

    #[test]
    fn shortest_trims_audio_but_never_drops_frames() {
        let dir = TestDir::new();
        let spec = AudioSpec {
            sample_rate: 48_000,
            channels: 2,
            codec: AudioCodec::Pcm24,
        };
        let mut mux = LegacyMux::new(&dir.0.join("tc.mov"), spec).unwrap();
        // Today's one-second mix under 30 frames at 29.97 fps (1.001 s):
        // `-shortest` would end the file before the last frame.
        mux.set_video(30, pm_time::Rate::FPS_29_97);
        mux.frames = 48_000;
        assert!(!mux.shortest());
        mux.frames = 48_048;
        assert!(mux.shortest(), "audio as long as the frames");
        // 24 frames at 24 fps with a 1.01 s range: today trims the audio.
        mux.set_video(24, pm_time::Rate::FPS_24);
        mux.frames = 48_480;
        assert!(mux.shortest());
    }

    #[test]
    fn cancellation_removes_audio_sidecars() {
        let dir = TestDir::new();
        let output = dir.0.join("cancel.mp4");
        let spec = AudioSpec {
            sample_rate: 48_000,
            channels: 2,
            codec: AudioCodec::Aac,
        };
        let mut mux = LegacyMux::new(&output, spec).unwrap();
        mux.push(&[0.25, -0.25]).unwrap();
        let video = mux.video_path().to_path_buf();
        let audio = mux.audio.clone();
        assert!(matches!(
            mux.finish(Some(&AtomicBool::new(true))),
            Err(Error::Cancelled)
        ));
        drop(mux);
        assert!(!output.exists() && !video.exists() && !audio.exists());
    }
}
