//! AVAssetWriter muxing (section 17, "Muxers"): compressed video passed through from
//! our VTCompressionSession, PCM audio encoded by AVAssetWriter (24-bit LPCM in MOV,
//! AAC in MP4), and a `tmcd` track carrying the start timecode with the drop-frame
//! flag and the reel name, associated with the video track. Movie and media timescales
//! are always set (section 17; amendment 7).
//!
//! AVAssetWriter interleaves its inputs and reports an input "not ready" while another
//! input lags, so samples are queued per input and appended whenever their input is
//! ready; nothing blocks on one input while another starves.

#![allow(deprecated)]

use std::collections::VecDeque;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_av_foundation::*;
use objc2_avf_audio::*;
use objc2_core_audio_types::{
    AudioStreamBasicDescription, kAudioFormatFlagIsFloat, kAudioFormatFlagIsPacked,
    kAudioFormatLinearPCM, kAudioFormatMPEG4AAC,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::*;
use objc2_foundation::{NSString, NSURL};

use super::cf::{self, Dict};
use crate::Error;
use crate::settings::{AudioSpec, Container, TimecodeSpec};
use crate::timescale::{VideoTiming, audio_timescale};

/// How long the writer may make no progress before the export fails.
const STALL: Duration = Duration::from_secs(120);

struct Input {
    input: Retained<AVAssetWriterInput>,
    queue: VecDeque<CFRetained<CMSampleBuffer>>,
    finished: bool,
}

impl Input {
    /// Appends queued samples while the input is ready. Returns whether any were appended.
    fn drain(&mut self, writer: &AVAssetWriter) -> Result<bool, Error> {
        let mut progressed = false;
        while let Some(sb) = self.queue.front() {
            if !unsafe { self.input.isReadyForMoreMediaData() } {
                break;
            }
            if !cf::objc_try("appendSampleBuffer", || unsafe {
                self.input.appendSampleBuffer(sb)
            })? {
                return Err(writer_error(writer, "appendSampleBuffer"));
            }
            self.queue.pop_front();
            progressed = true;
        }
        Ok(progressed)
    }
}

fn writer_error(writer: &AVAssetWriter, what: &str) -> Error {
    let detail = unsafe { writer.error() }
        .map(|e| format!("{} ({})", e.localizedDescription(), cf::describe(&e)))
        .unwrap_or_default();
    Error::Mux(format!("{what}: {detail}"))
}

/// One AVAssetWriter session.
pub struct Writer {
    writer: Retained<AVAssetWriter>,
    file_type: Container,
    timing: VideoTiming,
    frames: u64,
    video: Option<Input>,
    audio: Option<Input>,
    audio_format: Option<CFRetained<CMFormatDescription>>,
    audio_written: u64,
    audio_spec: Option<AudioSpec>,
    timecode: Option<(Retained<AVAssetWriterInput>, CFRetained<CMSampleBuffer>)>,
    started: bool,
    done: bool,
}

unsafe impl Send for Writer {}

fn url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn key(k: Option<&'static NSString>) -> &'static NSString {
    k.expect("AVFoundation constant")
}

impl Writer {
    /// Creates the writer, the audio input and the timecode track. The video input is
    /// added when the first compressed sample brings its format description.
    pub fn new(
        path: &Path,
        container: Container,
        timing: VideoTiming,
        frames: u64,
        audio: Option<AudioSpec>,
        timecode: Option<&TimecodeSpec>,
    ) -> Result<Self, Error> {
        let file_type = unsafe {
            match container {
                Container::Mov => AVFileTypeQuickTimeMovie,
                Container::Mp4 => AVFileTypeMPEG4,
            }
        }
        .expect("AVFoundation file type");
        let writer =
            unsafe { AVAssetWriter::assetWriterWithURL_fileType_error(&url(path), file_type) }
                .map_err(|e| Error::Mux(format!("AVAssetWriter: {}", e.localizedDescription())))?;
        unsafe {
            writer.setMovieTimeScale(timing.timescale);
            // MP4 for the web: the movie header goes first (today's -movflags +faststart).
            writer.setShouldOptimizeForNetworkUse(container == Container::Mp4);
        }
        let mut this = Self {
            writer,
            file_type: container,
            timing,
            frames,
            video: None,
            audio: None,
            audio_format: None,
            audio_written: 0,
            audio_spec: audio,
            timecode: None,
            started: false,
            done: false,
        };
        if let Some(a) = audio {
            this.add_audio(a)?;
        }
        if let Some(tc) = timecode {
            this.add_timecode(tc)?;
        }
        Ok(this)
    }

    fn add_audio(&mut self, a: AudioSpec) -> Result<(), Error> {
        let settings = unsafe {
            match self.file_type {
                // Today's ProRes export carries 24-bit PCM.
                Container::Mov => Dict::new()
                    .u32(key(AVFormatIDKey), kAudioFormatLinearPCM)
                    .f64(key(AVSampleRateKey), a.sample_rate as f64)
                    .u32(key(AVNumberOfChannelsKey), a.channels as u32)
                    .u32(key(AVLinearPCMBitDepthKey), 24)
                    .bool(key(AVLinearPCMIsFloatKey), false)
                    .bool(key(AVLinearPCMIsBigEndianKey), false)
                    .bool(key(AVLinearPCMIsNonInterleaved), false)
                    .build(),
                Container::Mp4 => Dict::new()
                    .u32(key(AVFormatIDKey), kAudioFormatMPEG4AAC)
                    .f64(key(AVSampleRateKey), a.sample_rate as f64)
                    .u32(key(AVNumberOfChannelsKey), a.channels as u32)
                    .u32(key(AVEncoderBitRateKey), 128_000 * a.channels as u32)
                    .build(),
            }
        };
        let media = unsafe { AVMediaTypeAudio }.expect("AVMediaTypeAudio");
        let input = cf::objc_try("audio input", || unsafe {
            let input =
                AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings_sourceFormatHint(
                    media,
                    Some(&settings),
                    None,
                );
            input.setExpectsMediaDataInRealTime(false);
            // AVAssetWriter refuses a non-default audio timescale; its default is the
            // output sample rate, which section 17 asks for (verification checks it).
            input
        })?;
        if !unsafe { self.writer.canAddInput(&input) } {
            return Err(Error::Mux(
                "AVAssetWriter cannot add the audio input".into(),
            ));
        }
        cf::objc_try("addInput (audio)", || unsafe {
            self.writer.addInput(&input)
        })?;
        // The source: interleaved f32 PCM.
        let bytes = 4 * a.channels as u32;
        let asbd = AudioStreamBasicDescription {
            mSampleRate: a.sample_rate as f64,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
            mBytesPerPacket: bytes,
            mFramesPerPacket: 1,
            mBytesPerFrame: bytes,
            mChannelsPerFrame: a.channels as u32,
            mBitsPerChannel: 32,
            mReserved: 0,
        };
        let mut fd: *const CMFormatDescription = std::ptr::null();
        let st = unsafe {
            CMAudioFormatDescriptionCreate(
                None,
                NonNull::from(&asbd),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                None,
                NonNull::from(&mut fd),
            )
        };
        cf::check(st, "CMAudioFormatDescriptionCreate")?;
        self.audio_format = Some(unsafe {
            CFRetained::from_raw(NonNull::new_unchecked(fd as *mut CMFormatDescription))
        });
        self.audio = Some(Input {
            input,
            queue: VecDeque::new(),
            finished: false,
        });
        Ok(())
    }

    fn add_timecode(&mut self, tc: &TimecodeSpec) -> Result<(), Error> {
        let base = tc.format.base();
        let mut flags = kCMTimeCodeFlag_24HourMax;
        if tc.format.is_drop_frame() {
            flags |= kCMTimeCodeFlag_DropFrame;
        }
        let extensions = tc.reel.as_ref().map(|reel| {
            let name = Dict::new()
                .cfkey(
                    unsafe { kCMTimeCodeFormatDescriptionKey_Value },
                    &NSString::from_str(reel),
                )
                .i64(
                    cf::ns(unsafe { kCMTimeCodeFormatDescriptionKey_LangCode }),
                    0,
                )
                .build();
            Dict::new()
                .cfkey(
                    unsafe { kCMTimeCodeFormatDescriptionExtension_SourceReferenceName },
                    &name,
                )
                .build()
        });
        let mut fd: *const CMFormatDescription = std::ptr::null();
        let st = unsafe {
            CMTimeCodeFormatDescriptionCreate(
                None,
                kCMTimeCodeFormatType_TimeCode32,
                CMTime::new(self.timing.frame_duration, self.timing.timescale),
                base,
                flags,
                extensions.as_deref().map(cf::cf_dict),
                NonNull::from(&mut fd),
            )
        };
        cf::check(st, "CMTimeCodeFormatDescriptionCreate")?;
        let fd =
            unsafe { CFRetained::from_raw(NonNull::new_unchecked(fd as *mut CMFormatDescription)) };
        let media = unsafe { AVMediaTypeTimecode }.expect("AVMediaTypeTimecode");
        let input = cf::objc_try("timecode input", || unsafe {
            let input =
                AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings_sourceFormatHint(
                    media,
                    None,
                    Some(&fd),
                );
            input.setExpectsMediaDataInRealTime(false);
            input.setMediaTimeScale(self.timing.timescale);
            input
        })?;
        if !unsafe { self.writer.canAddInput(&input) } {
            return Err(Error::InvalidSpec(format!(
                "this container cannot carry a timecode track ({})",
                match self.file_type {
                    Container::Mov => "MOV",
                    Container::Mp4 => "MP4",
                }
            )));
        }
        cf::objc_try("addInput (timecode)", || unsafe {
            self.writer.addInput(&input)
        })?;
        // One sample for the whole movie: the frame number of the first frame.
        let start = (tc.start_frame as u32).to_be_bytes();
        let block = block_buffer(&start)?;
        let timing = CMSampleTimingInfo {
            duration: unsafe {
                CMTime::new(self.timing.duration(self.frames), self.timing.timescale)
            },
            presentationTimeStamp: unsafe { CMTime::new(0, self.timing.timescale) },
            decodeTimeStamp: unsafe { kCMTimeInvalid },
        };
        let size = start.len();
        let mut sb: *mut CMSampleBuffer = std::ptr::null_mut();
        let st = unsafe {
            CMSampleBuffer::create_ready(
                None,
                Some(&block),
                Some(&fd),
                1,
                1,
                &timing,
                1,
                &size,
                NonNull::from(&mut sb),
            )
        };
        cf::check(st, "CMSampleBufferCreateReady (timecode)")?;
        self.timecode = Some((input, unsafe {
            CFRetained::from_raw(NonNull::new_unchecked(sb))
        }));
        Ok(())
    }

    /// Starts the session once the video format is known.
    fn start(&mut self, format: &CMFormatDescription) -> Result<(), Error> {
        let media = unsafe { AVMediaTypeVideo }.expect("AVMediaTypeVideo");
        let input = cf::objc_try("video input", || unsafe {
            let input =
                AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings_sourceFormatHint(
                    media,
                    None,
                    Some(format),
                );
            input.setExpectsMediaDataInRealTime(false);
            input.setMediaTimeScale(self.timing.timescale);
            input
        })?;
        if !unsafe { self.writer.canAddInput(&input) } {
            return Err(Error::Mux(
                "AVAssetWriter cannot add the video input".into(),
            ));
        }
        cf::objc_try("addInput (video)", || unsafe {
            self.writer.addInput(&input)
        })?;
        if let Some((tc, _)) = &self.timecode {
            let assoc = unsafe { AVTrackAssociationTypeTimecode };
            if !unsafe { input.canAddTrackAssociationWithTrackOfInput_type(tc, assoc) } {
                return Err(Error::Mux(
                    "the timecode track cannot be associated with the video track".into(),
                ));
            }
            cf::objc_try("timecode association", || unsafe {
                input.addTrackAssociationWithTrackOfInput_type(tc, assoc)
            })?;
        }
        if !cf::objc_try("startWriting", || unsafe { self.writer.startWriting() })? {
            return Err(writer_error(&self.writer, "startWriting"));
        }
        cf::objc_try("startSession", || unsafe {
            self.writer
                .startSessionAtSourceTime(CMTime::new(0, self.timing.timescale))
        })?;
        self.video = Some(Input {
            input,
            queue: VecDeque::new(),
            finished: false,
        });
        self.started = true;
        if let Some((tc, sample)) = self.timecode.take() {
            let mut tc_input = Input {
                input: tc,
                queue: VecDeque::from([sample]),
                finished: false,
            };
            let t0 = Instant::now();
            while !tc_input.queue.is_empty() {
                tc_input.drain(&self.writer)?;
                self.fail_if_failed()?;
                if t0.elapsed() > STALL {
                    return Err(Error::Mux("the timecode track never became ready".into()));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            cf::objc_try("markAsFinished (timecode)", || unsafe {
                tc_input.input.markAsFinished()
            })?;
        }
        Ok(())
    }

    fn fail_if_failed(&self) -> Result<(), Error> {
        if unsafe { self.writer.status() } == AVAssetWriterStatus::Failed {
            return Err(writer_error(&self.writer, "AVAssetWriter failed"));
        }
        Ok(())
    }

    /// Queues compressed video samples (decode order) and appends what the writer takes.
    pub fn push_video(&mut self, samples: Vec<CFRetained<CMSampleBuffer>>) -> Result<(), Error> {
        let Some(first) = samples.first() else {
            return Ok(());
        };
        if !self.started {
            let fd = unsafe { first.format_description() }
                .ok_or_else(|| Error::Mux("compressed sample has no format description".into()))?;
            self.start(&fd)?;
        }
        self.video.as_mut().expect("started").queue.extend(samples);
        // Without audio nothing else competes: wait so the queue stays short.
        let block = self.audio.is_none();
        self.pump(block)
    }

    /// Queues interleaved f32 PCM.
    pub fn push_audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        let (Some(spec), Some(fd)) = (self.audio_spec, &self.audio_format) else {
            return Err(Error::InvalidFrame("this export has no audio track".into()));
        };
        let channels = spec.channels as usize;
        if interleaved.is_empty() {
            return Ok(());
        }
        if !interleaved.len().is_multiple_of(channels) {
            return Err(Error::InvalidFrame(
                "audio samples are not a whole number of frames".into(),
            ));
        }
        let frames = interleaved.len() / channels;
        let bytes: Vec<u8> = interleaved.iter().flat_map(|s| s.to_ne_bytes()).collect();
        let block = block_buffer(&bytes)?;
        let pts =
            unsafe { CMTime::new(self.audio_written as i64, audio_timescale(spec.sample_rate)) };
        let mut sb: *mut CMSampleBuffer = std::ptr::null_mut();
        let st = unsafe {
            CMAudioSampleBufferCreateReadyWithPacketDescriptions(
                None,
                &block,
                fd,
                frames as isize,
                pts,
                std::ptr::null(),
                NonNull::from(&mut sb),
            )
        };
        cf::check(st, "CMAudioSampleBufferCreateReadyWithPacketDescriptions")?;
        self.audio
            .as_mut()
            .expect("audio input")
            .queue
            .push_back(unsafe { CFRetained::from_raw(NonNull::new_unchecked(sb)) });
        self.audio_written += frames as u64;
        if self.started {
            self.pump(false)
        } else {
            Ok(())
        }
    }

    /// Appends whatever the writer is ready for; with `block`, until every queue is empty.
    fn pump(&mut self, block: bool) -> Result<(), Error> {
        let mut last = Instant::now();
        loop {
            let mut progressed = false;
            for input in [self.video.as_mut(), self.audio.as_mut()]
                .into_iter()
                .flatten()
            {
                progressed |= input.drain(&self.writer)?;
            }
            self.fail_if_failed()?;
            let pending = [&self.video, &self.audio]
                .into_iter()
                .flatten()
                .any(|i| !i.queue.is_empty());
            if !pending || !block {
                return Ok(());
            }
            if progressed {
                last = Instant::now();
            } else if last.elapsed() > STALL {
                return Err(Error::Mux("AVAssetWriter stopped accepting samples".into()));
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    /// Appends everything, closes the inputs and finishes the file.
    pub fn finish(mut self) -> Result<(), Error> {
        if !self.started {
            return Err(Error::Mux("no video was encoded".into()));
        }
        let mut last = Instant::now();
        loop {
            let mut progressed = false;
            for input in [self.video.as_mut(), self.audio.as_mut()]
                .into_iter()
                .flatten()
            {
                progressed |= input.drain(&self.writer)?;
                if input.queue.is_empty() && !input.finished {
                    cf::objc_try("markAsFinished", || unsafe { input.input.markAsFinished() })?;
                    input.finished = true;
                    progressed = true;
                }
            }
            self.fail_if_failed()?;
            if [&self.video, &self.audio]
                .into_iter()
                .flatten()
                .all(|i| i.finished)
            {
                break;
            }
            if progressed {
                last = Instant::now();
            } else if last.elapsed() > STALL {
                return Err(Error::Mux("AVAssetWriter stopped accepting samples".into()));
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let end = unsafe { CMTime::new(self.timing.duration(self.frames), self.timing.timescale) };
        cf::objc_try("endSession", || unsafe {
            self.writer.endSessionAtSourceTime(end)
        })?;
        let (tx, rx) = mpsc::channel::<()>();
        let done = RcBlock::new(move || {
            let _ = tx.send(());
        });
        cf::objc_try("finishWriting", || unsafe {
            self.writer.finishWritingWithCompletionHandler(&done)
        })?;
        rx.recv_timeout(Duration::from_secs(600))
            .map_err(|_| Error::Mux("finishWriting timed out".into()))?;
        self.done = true;
        if unsafe { self.writer.status() } != AVAssetWriterStatus::Completed {
            return Err(writer_error(&self.writer, "finishWriting"));
        }
        Ok(())
    }

    /// Audio frames queued so far.
    pub fn audio_frames(&self) -> u64 {
        self.audio_written
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if !self.done && self.started {
            unsafe { self.writer.cancelWriting() };
        }
    }
}

/// A CMBlockBuffer holding a copy of `bytes`.
fn block_buffer(bytes: &[u8]) -> Result<CFRetained<CMBlockBuffer>, Error> {
    let mut bb: *mut CMBlockBuffer = std::ptr::null_mut();
    let st = unsafe {
        CMBlockBuffer::create_with_memory_block(
            None,
            std::ptr::null_mut(),
            bytes.len(),
            None,
            std::ptr::null(),
            0,
            bytes.len(),
            kCMBlockBufferAssureMemoryNowFlag,
            NonNull::from(&mut bb),
        )
    };
    cf::check(st, "CMBlockBufferCreateWithMemoryBlock")?;
    let bb = unsafe { CFRetained::from_raw(NonNull::new_unchecked(bb)) };
    let st = unsafe {
        CMBlockBuffer::replace_data_bytes(NonNull::from(bytes).cast(), &bb, 0, bytes.len())
    };
    cf::check(st, "CMBlockBufferReplaceDataBytes")?;
    Ok(bb)
}
