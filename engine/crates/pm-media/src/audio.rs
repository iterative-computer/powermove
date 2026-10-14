//! Streaming audio decode (plan sections 13 and 16): FFmpeg through
//! Powermove's LGPL build, program samples only, sample-exact positions, no
//! length limit.
//!
//! # Program samples
//!
//! Sample 0 is the first sample of the program: encoder priming is never
//! returned. FFmpeg already discards the priming that the container or codec
//! announces (MP4 edit lists, the LAME header, Ogg Opus pre-skip, ...) and
//! moves the frame timestamps accordingly, so its stream `start_time` is the
//! first program sample. When [`crate::probe::timing::priming`] knows of more
//! priming than FFmpeg skipped (an iTunes `iTunSMPB` tag without an edit list,
//! Apple-encoded AAC without metadata), the difference is skipped here too.
//!
//! # Positions
//!
//! With a timebase at least as fine as one sample, every decoded frame is
//! placed by its timestamp, so seeks land on the exact sample, stream gaps
//! become silence, and overlaps (a frame stamped before the end of the
//! previous one) drop the samples the timeline already has; within one
//! sample of the expected position, a frame continues the line (container
//! rounding). Coarser timebases (Matroska and WebM use 1 ms) cannot
//! place a frame to the sample; such streams are positioned by counting
//! samples from the start, and a seek decodes forward from the start of the
//! stream (exact, but its cost grows with the seek distance).
//!
//! # Seeks
//!
//! A seek to program sample `n` starts decoding [`AudioStreamInfo::preroll`]
//! samples earlier (MDCT overlap, the MP3 bit reservoir, the Opus decoder
//! state) and discards everything before `n`. Lossless codecs need no
//! pre-roll and seek bit-exactly; lossy codecs converge to the sequential
//! decode within the pre-roll.

use crate::error::{MediaError, Result};
use crate::ff::{Input, check, cstr, err_str};
use crate::probe::timing;
use crate::sys::ffmpeg as ff;
use serde::Serialize;
use std::ffi::c_int;
use std::path::{Path, PathBuf};
use std::ptr;

/// A loudspeaker position (FFmpeg's `AVChannel`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Speaker {
    FrontLeft,
    FrontRight,
    FrontCenter,
    LowFrequency,
    BackLeft,
    BackRight,
    FrontLeftOfCenter,
    FrontRightOfCenter,
    BackCenter,
    SideLeft,
    SideRight,
    TopCenter,
    TopFrontLeft,
    TopFrontCenter,
    TopFrontRight,
    TopBackLeft,
    TopBackCenter,
    TopBackRight,
    StereoLeft,
    StereoRight,
    WideLeft,
    WideRight,
    SurroundDirectLeft,
    SurroundDirectRight,
    LowFrequency2,
    TopSideLeft,
    TopSideRight,
    BottomFrontCenter,
    BottomFrontLeft,
    BottomFrontRight,
    SideSurroundLeft,
    SideSurroundRight,
    TopSurroundLeft,
    TopSurroundRight,
    /// A position without a speaker meaning (unknown, unused, ambisonic, or
    /// one this list does not name); the value is FFmpeg's `AVChannel`.
    Other(i32),
}

impl Speaker {
    pub fn from_av(c: ff::AVChannel) -> Speaker {
        match c {
            ff::AV_CHAN_FRONT_LEFT => Speaker::FrontLeft,
            ff::AV_CHAN_FRONT_RIGHT => Speaker::FrontRight,
            ff::AV_CHAN_FRONT_CENTER => Speaker::FrontCenter,
            ff::AV_CHAN_LOW_FREQUENCY => Speaker::LowFrequency,
            ff::AV_CHAN_BACK_LEFT => Speaker::BackLeft,
            ff::AV_CHAN_BACK_RIGHT => Speaker::BackRight,
            ff::AV_CHAN_FRONT_LEFT_OF_CENTER => Speaker::FrontLeftOfCenter,
            ff::AV_CHAN_FRONT_RIGHT_OF_CENTER => Speaker::FrontRightOfCenter,
            ff::AV_CHAN_BACK_CENTER => Speaker::BackCenter,
            ff::AV_CHAN_SIDE_LEFT => Speaker::SideLeft,
            ff::AV_CHAN_SIDE_RIGHT => Speaker::SideRight,
            ff::AV_CHAN_TOP_CENTER => Speaker::TopCenter,
            ff::AV_CHAN_TOP_FRONT_LEFT => Speaker::TopFrontLeft,
            ff::AV_CHAN_TOP_FRONT_CENTER => Speaker::TopFrontCenter,
            ff::AV_CHAN_TOP_FRONT_RIGHT => Speaker::TopFrontRight,
            ff::AV_CHAN_TOP_BACK_LEFT => Speaker::TopBackLeft,
            ff::AV_CHAN_TOP_BACK_CENTER => Speaker::TopBackCenter,
            ff::AV_CHAN_TOP_BACK_RIGHT => Speaker::TopBackRight,
            ff::AV_CHAN_STEREO_LEFT => Speaker::StereoLeft,
            ff::AV_CHAN_STEREO_RIGHT => Speaker::StereoRight,
            ff::AV_CHAN_WIDE_LEFT => Speaker::WideLeft,
            ff::AV_CHAN_WIDE_RIGHT => Speaker::WideRight,
            ff::AV_CHAN_SURROUND_DIRECT_LEFT => Speaker::SurroundDirectLeft,
            ff::AV_CHAN_SURROUND_DIRECT_RIGHT => Speaker::SurroundDirectRight,
            ff::AV_CHAN_LOW_FREQUENCY_2 => Speaker::LowFrequency2,
            ff::AV_CHAN_TOP_SIDE_LEFT => Speaker::TopSideLeft,
            ff::AV_CHAN_TOP_SIDE_RIGHT => Speaker::TopSideRight,
            ff::AV_CHAN_BOTTOM_FRONT_CENTER => Speaker::BottomFrontCenter,
            ff::AV_CHAN_BOTTOM_FRONT_LEFT => Speaker::BottomFrontLeft,
            ff::AV_CHAN_BOTTOM_FRONT_RIGHT => Speaker::BottomFrontRight,
            ff::AV_CHAN_SIDE_SURROUND_LEFT => Speaker::SideSurroundLeft,
            ff::AV_CHAN_SIDE_SURROUND_RIGHT => Speaker::SideSurroundRight,
            ff::AV_CHAN_TOP_SURROUND_LEFT => Speaker::TopSurroundLeft,
            ff::AV_CHAN_TOP_SURROUND_RIGHT => Speaker::TopSurroundRight,
            other => Speaker::Other(other),
        }
    }
}

/// What a stream decodes to.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStreamInfo {
    pub path: PathBuf,
    /// FFmpeg stream index.
    pub stream: usize,
    pub container: String,
    pub codec: String,
    pub sample_rate: u32,
    /// One speaker per decoded channel, in channel order. A stream without a
    /// channel layout gets FFmpeg's default layout for its channel count.
    pub speakers: Vec<Speaker>,
    /// FFmpeg's description of the layout (`stereo`, `5.1(side)`, ...).
    pub layout: Option<String>,
    /// Priming samples removed before program sample 0: what FFmpeg skipped
    /// plus what `probe::timing` knows of beyond that.
    pub priming: i64,
    /// Of [`Self::priming`], the part skipped here rather than by FFmpeg.
    pub extra_priming: i64,
    /// Program samples per channel, from the container's duration. A pure
    /// function of the file's metadata (never refined by decoding), so every
    /// use of it is deterministic. `None` when the container has no duration.
    pub length: Option<u64>,
    /// Samples decoded and discarded before a seek target.
    pub preroll: u64,
    /// The timebase places frames to the sample (see the module notes).
    pub sample_exact_timestamps: bool,
}

impl AudioStreamInfo {
    pub fn channels(&self) -> usize {
        self.speakers.len()
    }

    /// The program length in seconds, when known.
    pub fn duration(&self) -> Option<f64> {
        self.length.map(|n| n as f64 / self.sample_rate as f64)
    }
}

/// Decode pre-roll per codec, in samples at the stream rate.
fn preroll_for(codec: ff::AVCodecID, sample_rate: u32) -> u64 {
    match codec {
        // Lossless and PCM: every packet decodes on its own.
        ff::AV_CODEC_ID_FLAC
        | ff::AV_CODEC_ID_ALAC
        | ff::AV_CODEC_ID_WAVPACK
        | ff::AV_CODEC_ID_TTA
        | ff::AV_CODEC_ID_APE
        | ff::AV_CODEC_ID_TRUEHD
        | ff::AV_CODEC_ID_MLP => 0,
        c if (ff::AV_CODEC_ID_PCM_S16LE..ff::AV_CODEC_ID_ADPCM_IMA_QT).contains(&c) => 0,
        // RFC 7845 section 4.6: 80 ms.
        ff::AV_CODEC_ID_OPUS => (sample_rate as u64 * 80).div_ceil(1000),
        // Two frames of overlap and SBR history.
        ff::AV_CODEC_ID_AAC | ff::AV_CODEC_ID_AAC_LATM => 4096,
        // The bit reservoir reaches back up to 511 bytes, several frames at
        // low bit rates, plus the filterbank overlap.
        ff::AV_CODEC_ID_MP3 | ff::AV_CODEC_ID_MP2 | ff::AV_CODEC_ID_MP1 => 8 * 1152,
        ff::AV_CODEC_ID_AC3 | ff::AV_CODEC_ID_EAC3 => 2 * 1536,
        // One long block of overlap.
        ff::AV_CODEC_ID_VORBIS => 8192,
        _ => 8192,
    }
}

/// A planar run of decoded samples not yet returned.
struct Pending {
    data: Vec<Vec<f32>>,
    /// Program index of `data[_][offset]`.
    start: i64,
    offset: usize,
}

impl Pending {
    fn len(&self) -> usize {
        self.data.first().map_or(0, |c| c.len()) - self.offset
    }
    fn clear(&mut self) {
        for c in &mut self.data {
            c.clear();
        }
        self.offset = 0;
    }
}

/// A streaming decoder for one audio stream.
pub struct AudioDecoder {
    input: Input,
    ctx: *mut ff::AVCodecContext,
    pkt: *mut ff::AVPacket,
    frame: *mut ff::AVFrame,
    info: AudioStreamInfo,
    /// Stream timebase.
    tb: (i64, i64),
    /// Stream timestamp of program sample 0, in the stream timebase, before
    /// the extra priming (FFmpeg's `start_time`).
    start_pts: i64,
    /// Program index of the next sample FFmpeg will deliver, once known.
    expected: Option<i64>,
    /// Samples below this index are discarded (the seek target).
    target: i64,
    pending: Pending,
    /// Program index of the next sample `read` returns.
    position: i64,
    /// The decoder was sent its final flush.
    draining: bool,
    eof: bool,
}

// The FFmpeg contexts are used from one thread at a time.
unsafe impl Send for AudioDecoder {}

impl Drop for AudioDecoder {
    fn drop(&mut self) {
        unsafe {
            ff::av_packet_free(&mut self.pkt);
            ff::av_frame_free(&mut self.frame);
            ff::avcodec_free_context(&mut self.ctx);
        }
    }
}

fn layout_speakers(layout: &ff::AVChannelLayout) -> Vec<Speaker> {
    let n = layout.nb_channels.max(0) as usize;
    unsafe {
        let mut fallback: ff::AVChannelLayout = std::mem::zeroed();
        let source = if layout.order == ff::AV_CHANNEL_ORDER_UNSPEC {
            ff::av_channel_layout_default(&mut fallback, n as c_int);
            &fallback
        } else {
            layout
        };
        let out = (0..n)
            .map(|i| Speaker::from_av(ff::av_channel_layout_channel_from_index(source, i as u32)))
            .collect();
        ff::av_channel_layout_uninit(&mut fallback);
        out
    }
}

fn describe_layout(layout: &ff::AVChannelLayout) -> Option<String> {
    let mut buf = [0 as std::ffi::c_char; 128];
    unsafe {
        (ff::av_channel_layout_describe(layout, buf.as_mut_ptr(), buf.len()) > 0)
            .then(|| cstr(buf.as_ptr()))
            .flatten()
    }
}

/// `ts × num / den × rate` rounded to the nearest sample (ties to even).
fn ts_to_samples(ts: i64, tb: (i64, i64), rate: u32) -> i64 {
    let n = ts as i128 * tb.0 as i128 * rate as i128;
    let d = tb.1 as i128;
    let q = n.div_euclid(d);
    let r = n.rem_euclid(d);
    let up = match (2 * r).cmp(&d) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => q % 2 != 0,
        std::cmp::Ordering::Less => false,
    };
    (q + up as i128) as i64
}

/// Samples to the stream timebase, rounded down.
fn samples_to_ts(samples: i64, tb: (i64, i64), rate: u32) -> i64 {
    let n = samples as i128 * tb.1 as i128;
    let d = tb.0 as i128 * rate as i128;
    n.div_euclid(d) as i64
}

impl AudioDecoder {
    /// Opens `stream` (the best audio stream when `None`).
    pub fn open(path: &Path, stream: Option<usize>) -> Result<AudioDecoder> {
        let input = Input::open(path)?;
        let s = match stream {
            Some(s) if s < input.streams() => s,
            Some(s) => return Err(MediaError::invalid(format!("no stream {s}"))),
            None => input
                .best_audio()
                .ok_or_else(|| MediaError::invalid("no audio stream"))?,
        };
        if input.media_type(s) != ff::AVMEDIA_TYPE_AUDIO {
            return Err(MediaError::invalid(format!("stream {s} is not audio")));
        }
        let container = input.format_name();
        let codec = input.codec_name(s);
        unsafe {
            // Only this stream's packets are read.
            for i in 0..input.streams() {
                (*input.st(i)).discard = if i == s {
                    ff::AVDISCARD_DEFAULT
                } else {
                    ff::AVDISCARD_ALL
                };
            }
            let st = input.st(s);
            let par = (*st).codecpar;
            let decoder = ff::avcodec_find_decoder((*par).codec_id);
            if decoder.is_null() {
                return Err(MediaError::unsupported(format!(
                    "no FFmpeg decoder for {codec}"
                )));
            }
            let ctx = ff::avcodec_alloc_context3(decoder);
            if ctx.is_null() {
                return Err(MediaError::OutOfMemory(0));
            }
            let mut out = AudioDecoder {
                input,
                ctx,
                pkt: ff::av_packet_alloc(),
                frame: ff::av_frame_alloc(),
                info: AudioStreamInfo {
                    path: path.to_owned(),
                    stream: s,
                    container,
                    codec,
                    sample_rate: 0,
                    speakers: vec![],
                    layout: None,
                    priming: 0,
                    extra_priming: 0,
                    length: None,
                    preroll: 0,
                    sample_exact_timestamps: false,
                },
                tb: (1, 1),
                start_pts: 0,
                expected: None,
                target: 0,
                pending: Pending {
                    data: vec![],
                    start: 0,
                    offset: 0,
                },
                position: 0,
                draining: false,
                eof: false,
            };
            if out.pkt.is_null() || out.frame.is_null() {
                return Err(MediaError::OutOfMemory(0));
            }
            check(
                ff::avcodec_parameters_to_context(ctx, par),
                "audio parameters",
            )?;
            (*ctx).pkt_timebase = (*st).time_base;
            check(
                ff::avcodec_open2(ctx, decoder, ptr::null_mut()),
                "open audio decoder",
            )?;
            let rate = (*ctx).sample_rate.max((*par).sample_rate);
            if rate <= 0 {
                return Err(MediaError::invalid("audio stream without a sample rate"));
            }
            let rate = rate as u32;
            let layout = if (*ctx).ch_layout.nb_channels > 0 {
                &(*ctx).ch_layout
            } else {
                &(*par).ch_layout
            };
            if layout.nb_channels <= 0 {
                return Err(MediaError::invalid("audio stream without channels"));
            }
            let speakers = layout_speakers(layout);
            let layout_name = describe_layout(layout);
            let tb = ((*st).time_base.num as i64, (*st).time_base.den as i64);
            if tb.0 <= 0 || tb.1 <= 0 {
                return Err(MediaError::invalid("audio stream without a timebase"));
            }
            let start_pts = if (*st).start_time != ff::AV_NOPTS_VALUE {
                (*st).start_time
            } else {
                0
            };
            let ffmpeg_skip = out.first_packet_skip()?;
            let probe_priming = timing::priming(
                path,
                &out.info.container,
                out.input.stream_id(s),
                &out.info.codec,
                rate as i64,
                (*par).initial_padding as i64,
            )
            .map_or(0, |p| p.samples);
            let extra = (probe_priming - ffmpeg_skip).max(0);
            let duration = (*st).duration;
            let mut length = (duration != ff::AV_NOPTS_VALUE && duration > 0)
                .then(|| ts_to_samples(duration, tb, rate))
                .or_else(|| {
                    // Matroska and others keep only the container duration
                    // (microseconds), measured from the container start.
                    let fmt = out.input.fmt;
                    let total = (*fmt).duration;
                    (total != ff::AV_NOPTS_VALUE && total > 0).then(|| {
                        let container_start = if (*fmt).start_time != ff::AV_NOPTS_VALUE {
                            (*fmt).start_time
                        } else {
                            0
                        };
                        let stream_start = ts_to_samples(start_pts, tb, rate);
                        ts_to_samples(total + container_start, (1, 1_000_000), rate) - stream_start
                    })
                });
            // Ogg reports the last granule, which counts the pre-skip.
            if out.info.container.contains("ogg") {
                length = length.map(|n| n - ffmpeg_skip);
            }
            let length = length.map(|n| (n - extra).max(0) as u64);
            out.tb = tb;
            out.start_pts = start_pts;
            out.info.sample_rate = rate;
            out.info.speakers = speakers;
            out.info.layout = layout_name;
            out.info.priming = ffmpeg_skip + extra;
            out.info.extra_priming = extra;
            out.info.length = length;
            out.info.preroll = preroll_for((*par).codec_id, rate);
            out.info.sample_exact_timestamps = tb.1 as i128 >= tb.0 as i128 * rate as i128;
            out.pending.data = vec![Vec::new(); out.info.channels()];
            out.restart()?;
            Ok(out)
        }
    }

    pub fn info(&self) -> &AudioStreamInfo {
        &self.info
    }

    /// Program index of the next sample [`Self::read`] returns.
    pub fn position(&self) -> u64 {
        self.position.max(0) as u64
    }

    /// The skip FFmpeg will apply at the start (`AV_PKT_DATA_SKIP_SAMPLES`
    /// of the stream's first packet), then rewinds.
    fn first_packet_skip(&mut self) -> Result<i64> {
        let s = self.info.stream;
        unsafe {
            let mut skip = 0i64;
            loop {
                let r = ff::av_read_frame(self.input.fmt, self.pkt);
                if r < 0 {
                    break;
                }
                if (*self.pkt).stream_index as usize == s {
                    let mut size = 0usize;
                    let data = ff::av_packet_get_side_data(
                        self.pkt,
                        ff::AV_PKT_DATA_SKIP_SAMPLES,
                        &mut size,
                    );
                    if !data.is_null() && size >= 4 {
                        skip = u32::from_le_bytes([*data, *data.add(1), *data.add(2), *data.add(3)])
                            as i64;
                    }
                    ff::av_packet_unref(self.pkt);
                    break;
                }
                ff::av_packet_unref(self.pkt);
            }
            Ok(skip)
        }
    }

    /// Replaces the codec context with a newly opened one. A flushed decoder
    /// can keep state across a seek (the AAC noise-substitution generator,
    /// for one), so every seek starts from a fresh decoder: what a seek
    /// returns depends on the file and the target only, never on what was
    /// decoded before.
    fn fresh_codec(&mut self) -> Result<()> {
        unsafe {
            let st = self.input.st(self.info.stream);
            let par = (*st).codecpar;
            let decoder = ff::avcodec_find_decoder((*par).codec_id);
            let mut ctx = ff::avcodec_alloc_context3(decoder);
            if ctx.is_null() {
                return Err(MediaError::OutOfMemory(0));
            }
            let opened = check(
                ff::avcodec_parameters_to_context(ctx, par),
                "audio parameters",
            )
            .and_then(|_| {
                (*ctx).pkt_timebase = (*st).time_base;
                check(
                    ff::avcodec_open2(ctx, decoder, ptr::null_mut()),
                    "open audio decoder",
                )
            });
            if let Err(e) = opened {
                ff::avcodec_free_context(&mut ctx);
                return Err(e);
            }
            ff::avcodec_free_context(&mut self.ctx);
            self.ctx = ctx;
        }
        Ok(())
    }

    /// Back to the start of the stream, positioned by counting.
    fn restart(&mut self) -> Result<()> {
        unsafe {
            // Aim well before program sample 0, so the demuxer lands on the
            // stream's first packet (an MP4 edit list's priming packet sits
            // before `start_time`) and applies its start skip again.
            let before = samples_to_ts(
                self.info.priming + self.info.preroll as i64 + self.info.sample_rate as i64,
                self.tb,
                self.info.sample_rate,
            );
            let first = self.start_pts.saturating_sub(before);
            let r = ff::avformat_seek_file(
                self.input.fmt,
                self.info.stream as c_int,
                i64::MIN,
                first,
                first,
                0,
            );
            if r < 0 {
                // Some demuxers only seek by byte; the stream starts at byte 0
                // of its data.
                check(
                    ff::av_seek_frame(self.input.fmt, -1, 0, ff::AVSEEK_FLAG_BYTE as c_int),
                    "rewind audio",
                )?;
            }
            self.fresh_codec()?;
        }
        self.pending.clear();
        self.expected = Some(-self.info.extra_priming);
        self.target = 0;
        self.position = 0;
        self.draining = false;
        self.eof = false;
        Ok(())
    }

    /// Positions the decoder so the next [`Self::read`] starts at program
    /// sample `index`.
    pub fn seek(&mut self, index: u64) -> Result<()> {
        self.seek_with(index, false)
    }

    /// [`Self::seek`] that never continues the running decode: the samples
    /// that follow depend only on the file and `index` (for lossless codecs
    /// both are the same). Block caches use it so a block decodes the same
    /// way whichever block was decoded before it.
    pub fn seek_anchored(&mut self, index: u64) -> Result<()> {
        self.seek_with(index, self.info.preroll > 0)
    }

    fn seek_with(&mut self, index: u64, anchored: bool) -> Result<()> {
        let index = index.min(i64::MAX as u64 / 4) as i64;
        if index == self.position && !anchored {
            return Ok(());
        }
        let rate = self.info.sample_rate as i64;
        // Close ahead: decode forward, which keeps the sequential decode.
        if !anchored && index > self.position && index - self.position <= rate * 2 {
            self.target = index;
            return self.skip_to_target();
        }
        if !self.info.sample_exact_timestamps || index <= self.info.preroll as i64 {
            self.restart()?;
            self.target = index;
            return self.skip_to_target();
        }
        let from = index - self.info.preroll as i64 + self.info.extra_priming;
        let ts = self.start_pts + samples_to_ts(from, self.tb, self.info.sample_rate);
        unsafe {
            let r = ff::avformat_seek_file(
                self.input.fmt,
                self.info.stream as c_int,
                i64::MIN,
                ts,
                ts,
                0,
            );
            if r < 0 {
                self.restart()?;
                self.target = index;
                return self.skip_to_target();
            }
            self.fresh_codec()?;
        }
        self.pending.clear();
        self.expected = None;
        self.target = index;
        self.draining = false;
        self.eof = false;
        self.position = index;
        if let Err(e) = self.fill() {
            if matches!(e, MediaError::Unsupported(_)) {
                // The demuxer landed after the target: decode from the start.
                self.restart()?;
                self.target = index;
                return self.skip_to_target();
            }
            return Err(e);
        }
        Ok(())
    }

    /// Discards decoded samples up to the target.
    fn skip_to_target(&mut self) -> Result<()> {
        while self.position < self.target {
            if self.pending.len() == 0 {
                if !self.fill()? {
                    self.position = self.target;
                    return Ok(());
                }
                continue;
            }
            let start = self.pending.start + self.pending.offset as i64;
            let drop = ((self.target - start).max(0) as usize).min(self.pending.len());
            self.pending.offset += drop;
            self.position = start + drop as i64;
            if drop == 0 {
                break;
            }
        }
        self.position = self.target;
        Ok(())
    }

    /// Decodes until `pending` holds samples at or after the target.
    /// Returns false at the end of the stream.
    fn fill(&mut self) -> Result<bool> {
        loop {
            if self.pending.len() > 0 {
                return Ok(true);
            }
            if self.eof {
                return Ok(false);
            }
            let r = unsafe { ff::avcodec_receive_frame(self.ctx, self.frame) };
            if r == 0 {
                self.take_frame()?;
                unsafe { ff::av_frame_unref(self.frame) };
                continue;
            }
            if r == ff::AVERROR_EOF {
                self.eof = true;
                return Ok(false);
            }
            if r == ff::AVERROR_INVALIDDATA {
                continue;
            }
            if r != ff::averror(libc::EAGAIN) {
                check(r, "receive audio")?;
            }
            if self.draining {
                self.eof = true;
                return Ok(false);
            }
            self.send_next_packet()?;
        }
    }

    fn send_next_packet(&mut self) -> Result<()> {
        unsafe {
            loop {
                let r = ff::av_read_frame(self.input.fmt, self.pkt);
                if r == ff::AVERROR_EOF
                    || r == ff::averror(libc::EIO) && ff::avio_feof((*self.input.fmt).pb) != 0
                {
                    self.draining = true;
                    let s = ff::avcodec_send_packet(self.ctx, ptr::null());
                    if s < 0 && s != ff::AVERROR_EOF {
                        check(s, "flush audio decoder")?;
                    }
                    return Ok(());
                }
                if r < 0 {
                    check(r, "read audio")?;
                }
                if (*self.pkt).stream_index as usize != self.info.stream {
                    ff::av_packet_unref(self.pkt);
                    continue;
                }
                let s = ff::avcodec_send_packet(self.ctx, self.pkt);
                ff::av_packet_unref(self.pkt);
                if s < 0 && s != ff::AVERROR_INVALIDDATA && s != ff::averror(libc::EAGAIN) {
                    check(s, "send audio")?;
                }
                return Ok(());
            }
        }
    }

    /// Places the decoded frame on the program timeline and appends what
    /// lies at or after the target to `pending`.
    fn take_frame(&mut self) -> Result<()> {
        let f = self.frame;
        let (n, pts) = unsafe { ((*f).nb_samples.max(0) as usize, (*f).best_effort_timestamp) };
        if n == 0 {
            return Ok(());
        }
        let by_pts = (self.info.sample_exact_timestamps && pts != ff::AV_NOPTS_VALUE).then(|| {
            ts_to_samples(pts - self.start_pts, self.tb, self.info.sample_rate)
                - self.info.extra_priming
        });
        let start = match (self.expected, by_pts) {
            // Sequential: timestamps only move the line for a real gap or
            // overlap, not for rounding in the container.
            (Some(e), Some(p)) if (p - e).abs() > 1 => p,
            (Some(e), _) => e,
            (None, Some(p)) => p,
            (None, None) => {
                return Err(MediaError::decode(
                    "an audio frame without a timestamp after a seek",
                ));
            }
        };
        if self.expected.is_none() && start > self.target {
            // A seek that landed past its target.
            return Err(MediaError::unsupported("the seek landed after its target"));
        }
        // A gap (the frame starts after the expected sample) becomes
        // silence; an overlap (it starts before) drops the frame's samples
        // the timeline already has, so the rest sits at its timestamp.
        let (gap, lead) = match self.expected {
            Some(e) if start > e => (start - e, 0),
            Some(e) if start < e => (0, (e - start).min(n as i64)),
            _ => (0, 0),
        };
        let lead = lead as usize;
        self.expected = Some((start + n as i64).max(self.expected.unwrap_or(i64::MIN)));
        let channels = self.info.channels();
        let frame_channels = unsafe { (*f).ch_layout.nb_channels.max(0) as usize };
        if frame_channels != channels {
            return Err(MediaError::decode(format!(
                "the stream changed from {channels} to {frame_channels} channels"
            )));
        }
        let rate = unsafe { (*f).sample_rate };
        if rate > 0 && rate as u32 != self.info.sample_rate {
            return Err(MediaError::decode(format!(
                "the stream changed sample rate from {} to {rate}",
                self.info.sample_rate
            )));
        }
        // What of this frame (and of a gap before it) is at or after the
        // target: the timeline continues at `first`.
        let first = start - gap + lead as i64;
        let keep_from = (self.target - first).max(0) as usize;
        let total = gap as usize + n - lead;
        if keep_from >= total {
            return Ok(());
        }
        if self.pending.len() == 0 {
            self.pending.clear();
            self.pending.start = first + keep_from as i64;
        }
        let fmt = unsafe { (*f).format };
        let planar = unsafe { ff::av_sample_fmt_is_planar(fmt) != 0 };
        let bytes = unsafe { ff::av_get_bytes_per_sample(fmt) as usize };
        if bytes == 0 {
            return Err(MediaError::unsupported(format!(
                "audio sample format {fmt}"
            )));
        }
        for (c, out) in self.pending.data.iter_mut().enumerate() {
            let silence_from = keep_from.min(gap as usize);
            out.extend(std::iter::repeat_n(0.0, gap as usize - silence_from));
            let skip = lead + keep_from.saturating_sub(gap as usize);
            unsafe {
                let (base, stride) = if planar {
                    (*(*f).extended_data.add(c), bytes)
                } else {
                    ((*(*f).extended_data).add(c * bytes), bytes * channels)
                };
                convert(base, stride, skip, n, fmt, out);
            }
        }
        Ok(())
    }

    /// Reads up to `frames` samples per channel into `out` (one vector per
    /// channel; samples are appended). Returns how many were read: fewer
    /// than asked only at the end of the stream.
    pub fn read(&mut self, out: &mut [Vec<f32>], frames: usize) -> Result<usize> {
        if out.len() != self.info.channels() {
            return Err(MediaError::invalid(format!(
                "{} output channels for a {}-channel stream",
                out.len(),
                self.info.channels()
            )));
        }
        let mut done = 0;
        while done < frames {
            if self.pending.len() == 0 && !self.fill()? {
                break;
            }
            let take = (frames - done).min(self.pending.len());
            let off = self.pending.offset;
            for (dst, src) in out.iter_mut().zip(&self.pending.data) {
                dst.extend_from_slice(&src[off..off + take]);
            }
            self.pending.offset += take;
            self.position += take as i64;
            done += take;
        }
        Ok(done)
    }

    /// The FFmpeg error text for a code (for diagnostics).
    pub fn error_text(code: i32) -> String {
        err_str(code)
    }
}

/// Converts `n` samples of one channel from FFmpeg memory to f32, skipping
/// the first `skip`.
unsafe fn convert(
    base: *const u8,
    stride: usize,
    skip: usize,
    n: usize,
    fmt: ff::AVSampleFormat,
    out: &mut Vec<f32>,
) {
    let at = |i: usize| unsafe { base.add(i * stride) };
    unsafe {
        match fmt {
            ff::AV_SAMPLE_FMT_U8 | ff::AV_SAMPLE_FMT_U8P => {
                out.extend((skip..n).map(|i| (*at(i) as f32 - 128.0) / 128.0))
            }
            ff::AV_SAMPLE_FMT_S16 | ff::AV_SAMPLE_FMT_S16P => out
                .extend((skip..n).map(|i| (at(i) as *const i16).read_unaligned() as f32 / 32768.0)),
            ff::AV_SAMPLE_FMT_S32 | ff::AV_SAMPLE_FMT_S32P => out
                .extend((skip..n).map(|i| {
                    ((at(i) as *const i32).read_unaligned() as f64 / 2147483648.0) as f32
                })),
            ff::AV_SAMPLE_FMT_S64 | ff::AV_SAMPLE_FMT_S64P => out.extend((skip..n).map(|i| {
                ((at(i) as *const i64).read_unaligned() as f64 / 9_223_372_036_854_775_808.0) as f32
            })),
            ff::AV_SAMPLE_FMT_FLT | ff::AV_SAMPLE_FMT_FLTP => {
                out.extend((skip..n).map(|i| (at(i) as *const f32).read_unaligned()))
            }
            ff::AV_SAMPLE_FMT_DBL | ff::AV_SAMPLE_FMT_DBLP => {
                out.extend((skip..n).map(|i| (at(i) as *const f64).read_unaligned() as f32))
            }
            _ => out.extend(std::iter::repeat_n(0.0, n - skip)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_conversion_rounds_to_the_nearest_sample() {
        // 90 kHz MPEG-TS clock at 48 kHz: 1.875 ticks per sample.
        assert_eq!(ts_to_samples(15, (1, 90000), 48000), 8);
        assert_eq!(ts_to_samples(-15, (1, 90000), 48000), -8);
        assert_eq!(ts_to_samples(324870, (1, 14112000), 48000), 1105);
        assert_eq!(samples_to_ts(1105, (1, 14112000), 48000), 324870);
        assert_eq!(samples_to_ts(-1, (1, 1000), 48000), -1);
    }
}
