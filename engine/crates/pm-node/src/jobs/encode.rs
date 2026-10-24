//! The host's WebCodecs encoders for the job engine's WebM exports
//! (decision EXPORT-1; `pm_protocol::host_encode`, contract 14).
//!
//! Today's exporter encodes WebM with Chromium's `VideoEncoder` and
//! `AudioEncoder`; [`pm_export::webcodecs`] hands each rendered frame and the
//! mix to them through the host, one `hostEncode` request per step. Frames
//! above the inline limit travel as shared-memory segments main takes once
//! (the snapshot transport); the engine unlinks a segment itself whatever the
//! answer, so none outlives its step.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pm_export::Error;
use pm_export::settings::{AudioSpec, Codec};
use pm_export::webcodecs::{EncodeHost, EncodeOpen};
use pm_protocol::common::{ByteSize, Bytes, Count, FrameRate, JobId, SessionId};
use pm_protocol::envelope::{Request, Response};
use pm_protocol::host::CapabilityHandle;
use pm_protocol::host_encode::{
    EncodeAudio, EncodeStep, EncodeVideoCodec, HostEncode, MAX_AUDIO_BYTES,
};
use pm_protocol::snapshot::{INLINE_PIXEL_BYTES, PixelPayload};

use super::HostPort;

/// How long one frame or audio step may take: the encoder queue drains
/// within a frame's encode time; minutes mean the window is gone.
const STEP_TIMEOUT: Duration = Duration::from_secs(120);
/// Finishing flushes the encoders, encodes the whole mix as Opus (today's
/// exporter encodes it after the frames), muxes and writes the file.
const FINISH_TIMEOUT: Duration = Duration::from_secs(1800);

pub(crate) struct HostEncoder<P: HostPort + ?Sized> {
    port: Arc<P>,
    session: SessionId,
    job: JobId,
    cancel: Arc<AtomicBool>,
    output: CapabilityHandle,
    channels: usize,
    gpu: Option<super::encode_gpu::GpuEncoder>,
    gpu_ns: u128,
    host_video_ns: u128,
    /// Exact little-endian PCM bytes, held until one protocol-sized chunk is
    /// full or the encode finishes. Video steps do not consume this buffer.
    pending_audio: Vec<u8>,
    host_audio_ns: u128,
    audio_steps: u64,
}

impl<P: HostPort + ?Sized> HostEncoder<P> {
    pub(crate) fn new(
        port: Arc<P>,
        session: SessionId,
        job: JobId,
        cancel: Arc<AtomicBool>,
        output: CapabilityHandle,
    ) -> Self {
        HostEncoder {
            port,
            session,
            job,
            cancel,
            output,
            channels: 2,
            gpu: None,
            gpu_ns: 0,
            host_video_ns: 0,
            pending_audio: Vec::new(),
            host_audio_ns: 0,
            audio_steps: 0,
        }
    }

    pub(crate) fn with_gpu(
        mut self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        gate: Arc<dyn pm_export::render::GpuGate>,
        pass: u64,
    ) -> Self {
        if std::env::var_os(super::encode_gpu::ENV).is_some() {
            self.gpu = Some(super::encode_gpu::GpuEncoder::new(device, queue).with_gate(
                gate,
                self.cancel.clone(),
                pass,
            ));
        }
        self
    }

    fn step(&self, step: EncodeStep, timeout: Duration) -> Result<(), Error> {
        let answer = self.port.call_until(
            Request::HostEncode(HostEncode {
                job: self.job.clone(),
                step,
            }),
            Some(self.session),
            Instant::now() + timeout,
            &self.cancel,
        );
        match answer {
            Some(Ok(Response::Ack)) => Ok(()),
            Some(Ok(_)) => Err(Error::Encode(
                "the host's encoder answered something else".into(),
            )),
            Some(Err(e)) => Err(Error::Encode(e.message)),
            None if self.cancel.load(Ordering::Relaxed) => Err(Error::Cancelled),
            None => Err(Error::Encode(format!(
                "the host's encoder did not answer within {} s",
                timeout.as_secs()
            ))),
        }
    }

    fn flush_audio(&mut self) -> Result<(), Error> {
        if self.cancel.load(Ordering::Relaxed) {
            self.pending_audio.clear();
            return Err(Error::Cancelled);
        }
        if self.pending_audio.is_empty() {
            return Ok(());
        }
        let bytes = std::mem::take(&mut self.pending_audio);
        let started = Instant::now();
        let result = self.step(
            EncodeStep::Audio {
                samples: Bytes(bytes),
            },
            STEP_TIMEOUT,
        );
        self.host_audio_ns += started.elapsed().as_nanos();
        self.audio_steps += 1;
        result
    }
}

fn count(n: u64) -> Result<Count, Error> {
    Count::new(n).map_err(|_| Error::InvalidSpec(format!("{n} is past the safe range")))
}

impl<P: HostPort + ?Sized + 'static> EncodeHost for HostEncoder<P> {
    fn open(&mut self, open: &EncodeOpen) -> Result<(), Error> {
        let (codec, bitrate) = match open.codec {
            Codec::Vp9 { bitrate_bps } => (EncodeVideoCodec::Vp9, bitrate_bps),
            Codec::Vp8 { bitrate_bps } => (EncodeVideoCodec::Vp8, bitrate_bps),
            other => {
                return Err(Error::InvalidSpec(format!(
                    "{} is not a WebCodecs WebM codec",
                    other.name()
                )));
            }
        };
        let (num, den) = (open.rate.num(), open.rate.den());
        let rate = match (u32::try_from(num), u32::try_from(den)) {
            (Ok(num), Ok(den)) => FrameRate::new(num, den),
            _ => return Err(Error::InvalidSpec(format!("frame rate {}", open.rate))),
        };
        self.channels = open.audio.map_or(2, |a: AudioSpec| usize::from(a.channels));
        let audio = match open.audio {
            Some(a) => Some(EncodeAudio {
                sample_rate: a.sample_rate,
                channels: u8::try_from(a.channels)
                    .map_err(|_| Error::InvalidSpec("audio channels".into()))?,
            }),
            None => None,
        };
        self.step(
            EncodeStep::Open {
                codec,
                width: open.width,
                height: open.height,
                frame_rate: rate,
                bitrate,
                frames: count(open.frames)?,
                audio,
                output: self.output.clone(),
            },
            STEP_TIMEOUT,
        )
    }

    fn gpu_frames(&self) -> bool {
        self.gpu.is_some()
    }

    fn video_gpu(&mut self, index: u64, texture: &wgpu::Texture) -> Result<(), Error> {
        let gpu = self
            .gpu
            .as_mut()
            .ok_or_else(|| Error::Encode("GPU transport unavailable".into()))?;
        let started = Instant::now();
        let (generation, pool_index) = gpu.frame(index, texture)?;
        self.gpu_ns += started.elapsed().as_nanos();
        let started = Instant::now();
        let result = self.step(
            EncodeStep::VideoSurface {
                index: count(index)?,
                generation,
                pool_index,
            },
            STEP_TIMEOUT,
        );
        self.host_video_ns += started.elapsed().as_nanos();
        result
    }

    fn video(&mut self, index: u64, rgba: Vec<u8>) -> Result<(), Error> {
        let index = count(index)?;
        if rgba.len() <= INLINE_PIXEL_BYTES {
            return self.step(
                EncodeStep::Video {
                    index,
                    pixels: PixelPayload::Inline { bytes: Bytes(rgba) },
                },
                STEP_TIMEOUT,
            );
        }
        let name = pm_bridge::shm::publish(&rgba)
            .map_err(|e| Error::Encode(format!("the frame could not be shared: {e}")))?;
        let length = ByteSize::new(rgba.len() as u64)
            .map_err(|_| Error::InvalidFrame("frame size".into()))?;
        drop(rgba);
        let result = self.step(
            EncodeStep::Video {
                index,
                pixels: PixelPayload::SharedMemory {
                    name: name.clone(),
                    length,
                },
            },
            STEP_TIMEOUT,
        );
        // Main unlinks what it takes; this covers a step it never took.
        pm_bridge::shm::unlink(&name);
        result
    }

    fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        if self.cancel.load(Ordering::Relaxed) {
            self.pending_audio.clear();
            return Err(Error::Cancelled);
        }
        let frame_bytes = self.channels * 4;
        if frame_bytes == 0
            || frame_bytes > MAX_AUDIO_BYTES
            || !interleaved.len().is_multiple_of(self.channels)
        {
            return Err(Error::InvalidFrame(
                "audio samples are not a whole frame".into(),
            ));
        }
        let chunk_bytes = MAX_AUDIO_BYTES / frame_bytes * frame_bytes;
        for frame in interleaved.chunks(self.channels) {
            if self.cancel.load(Ordering::Relaxed) {
                self.pending_audio.clear();
                return Err(Error::Cancelled);
            }
            if self.pending_audio.capacity() == 0 {
                self.pending_audio.reserve_exact(chunk_bytes);
            }
            for sample in frame {
                self.pending_audio.extend_from_slice(&sample.to_le_bytes());
            }
            if self.pending_audio.len() == chunk_bytes {
                self.flush_audio()?;
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Error> {
        self.flush_audio()?;
        if let Some(gpu) = &self.gpu {
            gpu.log_profile();
        }
        if std::env::var("PM_EXPORT_PROFILE").as_deref() == Ok("1") {
            eprintln!(
                "[export timing] gpu_conversion_ms={:.3} host_video_ms={:.3} host_audio_ms={:.3} audio_steps={}",
                self.gpu_ns as f64 / 1e6,
                self.host_video_ns as f64 / 1e6,
                self.host_audio_ns as f64 / 1e6,
                self.audio_steps
            );
        }
        self.step(EncodeStep::Finish, FINISH_TIMEOUT)
    }

    fn abort(&mut self) {
        self.pending_audio.clear();
        // Best effort even when the job was cancelled: the host frees the
        // encoders and the window; its own job cleanup covers a lost answer.
        let _ = self.port.call_until(
            Request::HostEncode(HostEncode {
                job: self.job.clone(),
                step: EncodeStep::Abort,
            }),
            Some(self.session),
            Instant::now() + Duration::from_secs(10),
            &AtomicBool::new(false),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::error::{EngineError, ErrorCode, ErrorScope};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Port {
        steps: Mutex<Vec<HostEncode>>,
        fail_on_video: Option<u64>,
        fail_on_audio: Option<usize>,
    }

    impl HostPort for Port {
        fn call(
            &self,
            request: Request,
            session: Option<SessionId>,
        ) -> Result<Response, EngineError> {
            self.call_until(request, session, Instant::now(), &AtomicBool::new(false))
                .expect("answered")
        }
        fn call_until(
            &self,
            request: Request,
            _: Option<SessionId>,
            _: Instant,
            _: &AtomicBool,
        ) -> Option<Result<Response, EngineError>> {
            let Request::HostEncode(step) = request else {
                panic!("not an encode step")
            };
            let job = step.job.clone();
            let fail_video = matches!(&step.step, EncodeStep::Video { index, .. }
                if Some(index.get()) == self.fail_on_video);
            if let EncodeStep::Video {
                pixels: PixelPayload::SharedMemory { name, length },
                ..
            } = &step.step
            {
                // Main's side: take the segment once.
                let bytes = pm_bridge::shm::take(name, length.get() as usize).unwrap();
                assert!(bytes.iter().all(|&b| b == 7));
            }
            let mut steps = self.steps.lock().unwrap();
            let fail_audio = matches!(&step.step, EncodeStep::Audio { .. })
                && self.fail_on_audio
                    == Some(
                        steps
                            .iter()
                            .filter(|s| matches!(&s.step, EncodeStep::Audio { .. }))
                            .count(),
                    );
            steps.push(step);
            Some(if fail_video || fail_audio {
                Err(EngineError::new(
                    ErrorCode::ExportFailed,
                    ErrorScope::Job { job },
                    if fail_audio {
                        "AudioEncoder: closed"
                    } else {
                        "VideoEncoder: closed"
                    },
                ))
            } else {
                Ok(Response::Ack)
            })
        }
    }

    fn encoder(port: Arc<Port>) -> HostEncoder<Port> {
        HostEncoder::new(
            port,
            SessionId::new(3).unwrap(),
            JobId::from("job-9"),
            Arc::new(AtomicBool::new(false)),
            CapabilityHandle::from_random_bytes([4; 16]),
        )
    }

    fn open_audio(e: &mut HostEncoder<Port>) {
        e.open(&EncodeOpen {
            codec: Codec::Vp9 {
                bitrate_bps: 4_000_000,
            },
            width: 32,
            height: 32,
            rate: pm_time::Rate::FPS_30,
            frames: 120,
            audio: Some(AudioSpec::default_for(
                pm_export::settings::Container::Webm,
                48_000,
                2,
            )),
        })
        .unwrap();
    }

    #[test]
    fn steps_follow_the_encode_and_large_frames_use_shared_memory() {
        let port = Arc::new(Port::default());
        let mut e = encoder(port.clone());
        e.open(&EncodeOpen {
            codec: Codec::Vp9 {
                bitrate_bps: 16_000_000,
            },
            width: 1920,
            height: 1080,
            rate: pm_time::Rate::FPS_29_97,
            frames: 2,
            audio: Some(AudioSpec::default_for(
                pm_export::settings::Container::Webm,
                48_000,
                2,
            )),
        })
        .unwrap();
        e.video(0, vec![7; 1920 * 1080 * 4]).unwrap();
        e.video(1, vec![7; 16]).unwrap();
        // Three seconds of stereo: two messages of whole frames.
        e.audio(&vec![0.25; 48_000 * 2 * 3]).unwrap();
        e.finish().unwrap();
        let steps = port.steps.lock().unwrap();
        let kinds: Vec<String> = steps
            .iter()
            .map(|s| match &s.step {
                EncodeStep::Open {
                    frame_rate, codec, ..
                } => format!("open {codec:?} {}/{}", frame_rate.num, frame_rate.den),
                EncodeStep::Video { index, pixels } => format!(
                    "video {} {}",
                    index.get(),
                    matches!(pixels, PixelPayload::SharedMemory { .. })
                ),
                EncodeStep::VideoSurface {
                    index,
                    generation,
                    pool_index,
                } => format!("surface {} {generation}:{pool_index}", index.get()),
                EncodeStep::Audio { samples } => format!("audio {}", samples.0.len()),
                EncodeStep::Finish => "finish".into(),
                EncodeStep::Abort => "abort".into(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "open Vp9 30000/1001",
                "video 0 true",
                "video 1 false",
                &format!("audio {MAX_AUDIO_BYTES}"),
                &format!("audio {}", 48_000 * 2 * 4 * 3 - MAX_AUDIO_BYTES),
                "finish",
            ]
        );
        assert!(steps.iter().all(|s| s.job.as_str() == "job-9"));
    }

    #[test]
    fn a_failed_frame_surfaces_the_hosts_message() {
        let port = Arc::new(Port {
            fail_on_video: Some(0),
            ..Port::default()
        });
        let mut e = encoder(port);
        let error = e.video(0, vec![7; 2 * 1024 * 1024]).unwrap_err();
        assert!(matches!(&error, Error::Encode(m) if m.contains("VideoEncoder")));
    }

    #[test]
    fn small_audio_calls_batch_exact_sample_bits_in_order() {
        let port = Arc::new(Port::default());
        let mut e = encoder(port.clone());
        open_audio(&mut e);
        let bits = [0u32, 0x8000_0000, 0x7fc0_1234, 0xff80_0000, 0x3e80_0000];
        let mut expected = Vec::new();
        for call in 0..100 {
            let samples: Vec<f32> = (0..2_000)
                .map(|i| f32::from_bits(bits[(call * 2_000 + i) % bits.len()]))
                .collect();
            expected.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
            e.audio(&samples).unwrap();
        }
        assert_eq!(
            e.audio_steps, 1,
            "only the full chunk is sent before finish"
        );
        assert_eq!(e.pending_audio.len(), expected.len() - MAX_AUDIO_BYTES);
        e.finish().unwrap();
        assert_eq!(e.audio_steps, 2);
        let steps = port.steps.lock().unwrap();
        let audio: Vec<&[u8]> = steps
            .iter()
            .filter_map(|s| match &s.step {
                EncodeStep::Audio { samples } => Some(samples.0.as_slice()),
                _ => None,
            })
            .collect();
        assert_eq!(
            audio.iter().map(|b| b.len()).collect::<Vec<_>>(),
            [MAX_AUDIO_BYTES, 32_000]
        );
        assert_eq!(audio.concat(), expected);
        assert!(matches!(&steps.last().unwrap().step, EncodeStep::Finish));
    }

    #[test]
    fn exact_audio_boundary_sends_no_empty_tail_and_later_tail_precedes_finish() {
        let port = Arc::new(Port::default());
        let mut e = encoder(port.clone());
        open_audio(&mut e);
        e.audio(&vec![0.25; MAX_AUDIO_BYTES / 4]).unwrap();
        assert_eq!(e.audio_steps, 1);
        assert!(e.pending_audio.is_empty());
        e.audio(&[-0.0, f32::from_bits(0x7fc0_1234)]).unwrap();
        e.finish().unwrap();
        let steps = port.steps.lock().unwrap();
        let tail = match &steps[steps.len() - 2].step {
            EncodeStep::Audio { samples } => &samples.0,
            other => panic!("expected audio tail, got {other:?}"),
        };
        assert_eq!(tail, &[0, 0, 0, 128, 0x34, 0x12, 0xc0, 0x7f]);
        assert!(matches!(&steps.last().unwrap().step, EncodeStep::Finish));
        assert_eq!(e.audio_steps, 2);
    }

    #[test]
    fn failed_audio_step_and_cancel_discard_unsent_pcm() {
        let port = Arc::new(Port {
            fail_on_audio: Some(0),
            ..Port::default()
        });
        let mut e = encoder(port.clone());
        open_audio(&mut e);
        let error = e.audio(&vec![0.25; MAX_AUDIO_BYTES / 4]).unwrap_err();
        assert!(matches!(&error, Error::Encode(m) if m.contains("AudioEncoder")));
        assert!(e.pending_audio.is_empty());
        e.abort();
        let steps = port.steps.lock().unwrap();
        assert_eq!(
            steps
                .iter()
                .filter(|s| matches!(&s.step, EncodeStep::Audio { .. }))
                .count(),
            1
        );
        assert!(matches!(&steps.last().unwrap().step, EncodeStep::Abort));
        drop(steps);

        let port = Arc::new(Port::default());
        let mut e = encoder(port.clone());
        open_audio(&mut e);
        e.audio(&[0.25, -0.0]).unwrap();
        e.cancel.store(true, Ordering::Relaxed);
        assert!(matches!(e.finish(), Err(Error::Cancelled)));
        assert!(e.pending_audio.is_empty());
        e.abort();
        let steps = port.steps.lock().unwrap();
        assert!(
            !steps
                .iter()
                .any(|s| matches!(&s.step, EncodeStep::Audio { .. } | EncodeStep::Finish))
        );
        assert!(matches!(&steps.last().unwrap().step, EncodeStep::Abort));
    }
}
