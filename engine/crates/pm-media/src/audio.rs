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
//! samples from the start. A demux-only scan (`PacketIndex`) counts the
//! samples of every packet with FFmpeg's parsers (or the codec's fixed frame
//! size), so a seek starts at a packet whose program sample is known and
//! counts from there: its cost does not grow with the seek distance. Where
//! no packet duration is known (ALAC in Matroska has no parser), a seek
//! decodes forward from the start, and a seek forward continues the running
//! decode instead of starting over, so reading a stream in order costs its
//! length once.
//!
//! # Seeks
//!
//! A seek to program sample `n` starts decoding [`AudioStreamInfo::preroll`]
//! samples earlier (MDCT overlap, the MP3 bit reservoir, the Opus decoder
//! state, HE-AAC v2's wait for a Parametric Stereo header), and at least the
//! codec delay a fresh decoder drops (HE-AAC's 4,224 samples are more than
//! AAC's pre-roll), and discards everything before `n`. Lossless codecs need
//! no pre-roll and seek bit-exactly; lossy codecs converge to the sequential
//! decode within the pre-roll. A seek whose first sample still lands after
//! its target starts earlier again rather than from the start of the stream.

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
    /// AAC with Parametric Stereo (HE-AAC v2): seeks pre-roll
    /// [`PS_PREROLL_SECONDS`].
    pub parametric_stereo: bool,
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

/// Pre-roll of AAC with Parametric Stereo (HE-AAC v2), in seconds. A
/// fresh decoder applies no stereo until a frame carries a PS header
/// (FFmpeg copies the mono core to both channels before it, and the core's
/// PS processing changes the left channel too), and encoders repeat that
/// header only every so often. Measured on `afconvert -d aacp` encodes (32,
/// 44.1 and 48 kHz; tones, swept and gated tones, noise, mono content; 24 to
/// 48 kbit/s; fresh starts every 0.25 s): a fresh decoder is off by up to
/// the full signal for as long as 1.86 s, then within the sequential
/// decode's SBR noise. A block that started inside that window would differ
/// from legacy (one decode from the start) by the signal's level, so the
/// pre-roll is about twice the longest wait.
pub const PS_PREROLL_SECONDS: u64 = 4;

/// Decode pre-roll per codec, in samples at the stream rate. `profile` is
/// FFmpeg's (`AV_PROFILE_*`, from the container or the probe's decode).
fn preroll_for(codec: ff::AVCodecID, profile: c_int, sample_rate: u32) -> u64 {
    match codec {
        ff::AV_CODEC_ID_AAC | ff::AV_CODEC_ID_AAC_LATM
            if profile == ff::AV_PROFILE_AAC_HE_V2 as c_int =>
        {
            sample_rate as u64 * PS_PREROLL_SECONDS
        }
        // Lossless and PCM: every packet decodes on its own.
        ff::AV_CODEC_ID_FLAC
        | ff::AV_CODEC_ID_ALAC
        | ff::AV_CODEC_ID_WAVPACK
        | ff::AV_CODEC_ID_TTA
        | ff::AV_CODEC_ID_APE
        | ff::AV_CODEC_ID_TRUEHD
        | ff::AV_CODEC_ID_MLP => 0,
        c if (ff::AV_CODEC_ID_PCM_S16LE..ff::AV_CODEC_ID_ADPCM_IMA_QT).contains(&c) => 0,
        // RFC 7845 section 4.6 asks for at least 80 ms, but CELT predicts
        // each band's energy from the previous frame, so a decoder started
        // mid-stream converges slowly: measured on tones (WebM, anchors close
        // to the target), 0.11 off after 80 ms, 1e-3 after 250 ms, 5e-6
        // after 420 ms, 3e-7 after 500 ms. A block boundary would step by
        // the difference, so pre-roll 500 ms.
        ff::AV_CODEC_ID_OPUS => (sample_rate as u64 * 500).div_ceil(1000),
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

/// Kept anchors are at least this many samples apart: a seek decodes at
/// most this much before its pre-roll (about 85 ms at 48 kHz), and an hour
/// of audio keeps about 42,000 anchors.
const ANCHOR_SPACING: i64 = 4096;

/// A packet a seek on a coarse-timebase stream can start at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Anchor {
    /// The packet's timestamp, unique in the stream (stream timebase).
    pts: i64,
    /// Program index of the packet's first sample.
    start: i64,
    /// Samples the packet decodes to.
    duration: i64,
    /// The next packet's timestamp (when later) and duration. A fresh
    /// decoder can hold back the anchor packet's samples, so its first
    /// frame may be the next packet's.
    next: Option<(i64, i64)>,
}

/// The demuxer and parser of a [`PacketIndex`] scan.
struct Scan {
    input: Input,
    stream: usize,
    /// The decoder's (probed) stream parameters: the frame size of raw
    /// AAC is only known after probing.
    par: *mut ff::AVCodecParameters,
    ctx: *mut ff::AVCodecContext,
    parser: *mut ff::AVCodecParserContext,
    pkt: *mut ff::AVPacket,
}

// Used from one thread at a time, like the decoder that owns it.
unsafe impl Send for Scan {}

impl Drop for Scan {
    fn drop(&mut self) {
        unsafe {
            if !self.parser.is_null() {
                ff::av_parser_close(self.parser);
            }
            ff::avcodec_free_context(&mut self.ctx);
            ff::av_packet_free(&mut self.pkt);
            ff::avcodec_parameters_free(&mut self.par);
        }
    }
}

/// One packet as the scan saw it.
struct ScannedPacket {
    pts: Option<i64>,
    /// Samples it decodes to, when known.
    duration: Option<i64>,
    /// It carries `AV_PKT_DATA_SKIP_SAMPLES` (priming or end trimming).
    skips: bool,
}

impl Scan {
    fn open(path: &Path, stream: usize, probed: *const ff::AVCodecParameters) -> Result<Scan> {
        let input = Input::open_raw(path)?;
        if stream >= input.streams()
            || input.media_type(stream) != ff::AVMEDIA_TYPE_AUDIO
            || input.codec_id(stream) != unsafe { (*probed).codec_id }
        {
            return Err(MediaError::unsupported("the scan sees other streams"));
        }
        unsafe {
            for i in 0..input.streams() {
                (*input.st(i)).discard = if i == stream {
                    ff::AVDISCARD_DEFAULT
                } else {
                    ff::AVDISCARD_ALL
                };
            }
            let out = Scan {
                stream,
                par: ff::avcodec_parameters_alloc(),
                ctx: ff::avcodec_alloc_context3(ptr::null()),
                parser: ff::av_parser_init((*probed).codec_id),
                pkt: ff::av_packet_alloc(),
                input,
            };
            if out.par.is_null() || out.ctx.is_null() || out.pkt.is_null() {
                return Err(MediaError::OutOfMemory(0));
            }
            check(
                ff::avcodec_parameters_copy(out.par, probed),
                "audio parameters",
            )?;
            check(
                ff::avcodec_parameters_to_context(out.ctx, probed),
                "audio parameters",
            )?;
            if !out.parser.is_null() {
                // Container packets are whole frames: the parser only reads
                // their headers (`PARSER_FLAG_COMPLETE_FRAMES`).
                (*out.parser).flags |= 1;
            }
            Ok(out)
        }
    }

    /// The next packet of the stream, `None` at the end.
    fn next(&mut self) -> Result<Option<ScannedPacket>> {
        unsafe {
            loop {
                let r = ff::av_read_frame(self.input.fmt, self.pkt);
                if r == ff::AVERROR_EOF
                    || r == ff::averror(libc::EIO) && ff::avio_feof((*self.input.fmt).pb) != 0
                {
                    return Ok(None);
                }
                check(r, "scan audio")?;
                if (*self.pkt).stream_index as usize != self.stream {
                    ff::av_packet_unref(self.pkt);
                    continue;
                }
                let mut parsed = 0;
                if !self.parser.is_null() {
                    let mut out: *mut u8 = ptr::null_mut();
                    let mut size: c_int = 0;
                    ff::av_parser_parse2(
                        self.parser,
                        self.ctx,
                        &mut out,
                        &mut size,
                        (*self.pkt).data,
                        (*self.pkt).size,
                        (*self.pkt).pts,
                        (*self.pkt).dts,
                        (*self.pkt).pos,
                    );
                    if size > 0 {
                        parsed = (*self.parser).duration.max(0) as i64;
                    }
                }
                // Codecs whose parser does not count (raw AAC) have a fixed
                // frame size; PCM counts by bytes.
                let fixed =
                    ff::av_get_audio_frame_duration2(self.par, (*self.pkt).size).max(0) as i64;
                let duration = if parsed > 0 {
                    Some(parsed)
                } else if fixed > 0 {
                    Some(fixed)
                } else {
                    None
                };
                let mut size = 0usize;
                let skips =
                    !ff::av_packet_get_side_data(self.pkt, ff::AV_PKT_DATA_SKIP_SAMPLES, &mut size)
                        .is_null();
                let pts = (*self.pkt).pts;
                ff::av_packet_unref(self.pkt);
                return Ok(Some(ScannedPacket {
                    pts: (pts != ff::AV_NOPTS_VALUE).then_some(pts),
                    duration,
                    skips,
                }));
            }
        }
    }
}

/// Program positions of a coarse-timebase stream's packets, from a
/// demux-only scan of the file, read as far as seeks need. The scan counts
/// samples exactly as a decode from the start does: the first packet's
/// samples less the priming, then every packet's duration. The anchors kept
/// are a function of the file alone (each is final once the packet after it
/// is read), so a seek's anchor, and with it what the seek decodes, does
/// not depend on what was scanned or decoded before.
struct PacketIndex {
    scan: Option<Scan>,
    anchors: Vec<Anchor>,
    /// Program index of the next packet's first sample.
    next_start: i64,
    /// The latest valid timestamp.
    last_pts: Option<i64>,
    /// The last packet, kept until the next one shows its timestamp is
    /// unique.
    candidate: Option<Anchor>,
}

impl PacketIndex {
    fn new(scan: Scan, priming: i64) -> PacketIndex {
        PacketIndex {
            scan: Some(scan),
            anchors: Vec::new(),
            next_start: -priming,
            last_pts: None,
            candidate: None,
        }
    }

    /// Program samples below this have every anchor they will ever have.
    fn settled(&self) -> i64 {
        match (&self.scan, &self.candidate) {
            (None, _) => i64::MAX,
            (Some(_), Some(c)) => c.start,
            (Some(_), None) => self.next_start,
        }
    }

    fn settle(&mut self, next_pts: Option<i64>, next_duration: Option<i64>) {
        let Some(mut c) = self.candidate.take() else {
            return;
        };
        // Two packets with one timestamp cannot be told apart.
        if next_pts == Some(c.pts) {
            return;
        }
        c.next = next_pts.zip(next_duration).filter(|&(pts, _)| pts > c.pts);
        if self
            .anchors
            .last()
            .is_none_or(|a| c.start - a.start >= ANCHOR_SPACING)
        {
            self.anchors.push(c);
        }
    }

    /// The last anchor at or before program sample `at`.
    fn anchor_for(&mut self, at: i64) -> Option<Anchor> {
        while self.settled() <= at {
            let Some(scan) = self.scan.as_mut() else {
                break;
            };
            let packet = match scan.next() {
                Ok(p) => p,
                Err(_) => {
                    // A read error ends the scan where it happened: the
                    // anchors before it stand, this seek and every later
                    // one resolve against them, and the decode reports the
                    // error itself if it reaches that spot.
                    self.scan = None;
                    self.candidate = None;
                    break;
                }
            };
            let Some(p) = packet else {
                self.settle(None, None);
                self.scan = None;
                break;
            };
            self.settle(p.pts, p.duration);
            let Some(duration) = p.duration else {
                // Positions past a packet of unknown length are unknown.
                self.scan = None;
                break;
            };
            let unique = p
                .pts
                .is_some_and(|t| self.last_pts.is_none_or(|last| t > last));
            if unique && !p.skips && self.next_start >= 0 && duration > 0 {
                self.candidate = Some(Anchor {
                    pts: p.pts.unwrap_or_default(),
                    start: self.next_start,
                    duration,
                    next: None,
                });
            }
            if p.pts.is_some() {
                self.last_pts = p.pts;
            }
            self.next_start += duration;
        }
        let i = self.anchors.partition_point(|a| a.start <= at);
        i.checked_sub(1).map(|i| self.anchors[i])
    }
}

/// Program index of the first sample of the first frame (`pts`, `n`
/// samples) a fresh decoder returns after starting at anchor `a`. Fresh
/// decoders drop their codec delay from the head of their first output
/// (`initial_padding`, Matroska's `CodecDelay`: Opus its pre-skip, AC-3
/// 256 samples, MP2 481, HE-AAC 4,224; LC-AAC and Vorbis nothing) and move
/// its timestamp by as much, rounded to the timebase: a frame of the anchor
/// packet holds the packet's last `n` samples. A decoder that holds the
/// anchor packet back returns the next packet's first.
///
/// `skip` is that codec delay in samples and `skip_ts` the same span in
/// the stream timebase. A delay as long as the anchor packet or longer
/// (HE-AAC's over 2,048-sample packets) runs into the packets after it:
/// the first frame then starts `skip` samples into the anchor's, and its
/// timestamp, moved by the delay, says so.
fn anchored_start(a: &Anchor, pts: i64, n: i64, skip: i64, skip_ts: i64) -> Option<i64> {
    if skip >= a.duration {
        // Packet timestamps are rounded to the timebase; a frame from a
        // packet before or after is a whole packet away.
        let duration_ts = a.next.map_or(0, |(p, _)| p - a.pts);
        let off = (pts - (a.pts + skip_ts)).abs();
        return (off <= 1 || 4 * off < duration_ts).then_some(a.start + skip);
    }
    let next_pts = a.next.map(|(p, _)| p);
    let of_anchor = pts >= a.pts && next_pts.is_none_or(|p| pts < p);
    if of_anchor && n <= a.duration {
        return Some(a.start + a.duration - n);
    }
    match a.next {
        Some((p, d)) if pts >= p && n <= d => Some(a.start + a.duration + d - n),
        _ => None,
    }
}

/// Where the running decode began; a seek that would begin at the same
/// place continues it instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    /// From the start of the stream, counting.
    Start,
    /// From an anchor packet (its timestamp), counting.
    Anchor(i64),
    /// Placed by timestamps.
    Placed,
}

/// Why a seek attempt could not be placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeekFault {
    /// The anchor packet was not found again.
    AnchorNotFound,
    /// The decoder's first frame after the anchor is not the anchor's.
    Misplaced,
    /// The first sample lands after the target: the attempt started too
    /// close to it (an earlier start places it).
    LandedAfter,
}

/// Anchored seeks failing in a row (for a reason of the stream) before the
/// stream counts from the start for good; see `AudioDecoder::anchor_failures`.
const ANCHOR_FAILURE_LIMIT: u32 = 3;

/// Seek attempts that landed after their target, each starting one anchor
/// (or one lead) earlier, before a seek decodes from the start.
const LANDED_AFTER_RETRIES: u32 = 2;

/// A streaming decoder for one audio stream.
pub struct AudioDecoder {
    input: Input,
    ctx: *mut ff::AVCodecContext,
    pkt: *mut ff::AVPacket,
    frame: *mut ff::AVFrame,
    info: AudioStreamInfo,
    /// What a fresh decoder drops from the head of its output
    /// (`initial_padding`): a seek must start at least this far before its
    /// target, or its first sample lands after it.
    codec_delay: i64,
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
    /// Where the running decode began.
    run: Run,
    /// Packets are dropped until the one with this timestamp (an anchored
    /// seek on a coarse timebase starts the decoder exactly there).
    feed_from: Option<i64>,
    /// The demuxer was rewound to find `feed_from`.
    rewound: bool,
    /// The anchor that places the first frame of an anchored seek.
    first: Option<Anchor>,
    /// Packet positions of a coarse-timebase stream (opened at the first
    /// seek that needs them; `None` before and for sample-exact streams).
    index: Option<PacketIndex>,
    /// No index can be opened for this stream.
    no_index: bool,
    /// Why the last seek attempt could not be placed (set with the
    /// `Unsupported` error that ends it).
    fault: Option<SeekFault>,
    /// Anchored seeks in a row that failed for a reason of the stream (the
    /// anchor packet was not found again, or the decoder's first frame did
    /// not fit it). Such failures come from the demuxer or the codec, so
    /// once [`ANCHOR_FAILURE_LIMIT`] happen in a row every later seek counts
    /// from the start, and reads in order continue that decode instead of
    /// retrying an anchor and restarting for every block. A success resets
    /// the count, so a stray anchor cannot turn random access into a decode
    /// from the start for good; a seek that landed after its target depends
    /// on the target, not the stream, and is never counted.
    anchor_failures: u32,
    /// Packets sent to the decoder (diagnostics).
    packets: u64,
    /// Timestamp and byte position of the stream's first packet, read when
    /// the file was opened: a rewind must land there.
    head: Option<(i64, i64)>,
    /// A seek to the start did not land on the first packet (the AVI
    /// demuxer stamps PCM after a seek to entry 0 one packet late), so
    /// rewinds reopen the file instead.
    reopen_to_rewind: bool,
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

/// FFmpeg's default layout for a channel count
/// (`av_channel_layout_default`): mono, stereo, 2.1, 4.0, 5.0, 5.1, 6.1,
/// 7.1, and so on; positions FFmpeg leaves unspecified come back as
/// [`Speaker::Other`].
pub fn default_speakers(channels: usize) -> Vec<Speaker> {
    let Ok(n) = c_int::try_from(channels) else {
        return vec![Speaker::Other(ff::AV_CHAN_UNKNOWN); channels];
    };
    unsafe {
        let mut layout: ff::AVChannelLayout = std::mem::zeroed();
        ff::av_channel_layout_default(&mut layout, n);
        let out = (0..channels)
            .map(|i| Speaker::from_av(ff::av_channel_layout_channel_from_index(&layout, i as u32)))
            .collect();
        ff::av_channel_layout_uninit(&mut layout);
        out
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
                    parametric_stereo: false,
                    sample_exact_timestamps: false,
                },
                codec_delay: 0,
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
                run: Run::Start,
                feed_from: None,
                rewound: false,
                first: None,
                index: None,
                no_index: false,
                fault: None,
                anchor_failures: 0,
                packets: 0,
                head: None,
                reopen_to_rewind: false,
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
            // The decoder reads the profile from the codec configuration
            // (explicit PS signalling); the probe's decode finds implicit PS.
            let profile = if (*ctx).profile == ff::AV_PROFILE_AAC_HE_V2 as c_int {
                (*ctx).profile
            } else {
                (*par).profile
            };
            out.info.preroll = preroll_for((*par).codec_id, profile, rate);
            out.info.parametric_stereo = profile == ff::AV_PROFILE_AAC_HE_V2 as c_int
                && matches!(
                    (*par).codec_id,
                    ff::AV_CODEC_ID_AAC | ff::AV_CODEC_ID_AAC_LATM
                );
            out.codec_delay = (*ctx).initial_padding.max(0) as i64;
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

    /// Packets sent to the codec since the decoder opened (diagnostics:
    /// what a seek or a read cost).
    pub fn packets_decoded(&self) -> u64 {
        self.packets
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
                    self.head = Some(((*self.pkt).pts, (*self.pkt).pos));
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

    /// Moves the demuxer to the stream's first packet.
    fn rewind(&mut self) -> Result<()> {
        if self.reopen_to_rewind {
            return self.reopen();
        }
        self.seek_to_head()?;
        // The demuxer must hand out the first packet again, with the same
        // timestamp: check, then seek once more to put it back.
        let Some(head) = self.head else {
            return Ok(());
        };
        let next = self.peek_packet();
        if next == Some(head) {
            self.seek_to_head()
        } else {
            self.reopen_to_rewind = true;
            self.reopen()
        }
    }

    fn seek_to_head(&mut self) -> Result<()> {
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
        }
        Ok(())
    }

    /// Timestamp and position of the stream's next packet (consumed).
    fn peek_packet(&mut self) -> Option<(i64, i64)> {
        unsafe {
            loop {
                if ff::av_read_frame(self.input.fmt, self.pkt) < 0 {
                    return None;
                }
                let mine = (*self.pkt).stream_index as usize == self.info.stream;
                let at = ((*self.pkt).pts, (*self.pkt).pos);
                ff::av_packet_unref(self.pkt);
                if mine {
                    return Some(at);
                }
            }
        }
    }

    /// Opens the file again: the demuxer starts at the first packet.
    fn reopen(&mut self) -> Result<()> {
        let input = Input::open(&self.info.path)?;
        unsafe {
            for i in 0..input.streams() {
                (*input.st(i)).discard = if i == self.info.stream {
                    ff::AVDISCARD_DEFAULT
                } else {
                    ff::AVDISCARD_ALL
                };
            }
        }
        self.input = input;
        Ok(())
    }

    /// Back to the start of the stream, positioned by counting.
    fn restart(&mut self) -> Result<()> {
        self.rewind()?;
        self.fresh_codec()?;
        self.pending.clear();
        self.expected = Some(-self.info.extra_priming);
        self.target = 0;
        self.position = 0;
        self.draining = false;
        self.eof = false;
        self.run = Run::Start;
        self.feed_from = None;
        self.first = None;
        Ok(())
    }

    /// Positions the decoder so the next [`Self::read`] starts at program
    /// sample `index`.
    pub fn seek(&mut self, index: u64) -> Result<()> {
        self.seek_with(index, false)
    }

    /// [`Self::seek`] whose samples depend only on the file and `index`:
    /// it starts a fresh decoder where it would start for `index` from any
    /// state, and continues the running decode only when that decode began
    /// at the same place (for lossless codecs both are the same). Block
    /// caches use it so a block decodes the same way whichever block was
    /// decoded before it.
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
        if !self.info.sample_exact_timestamps {
            return self.seek_counted(index);
        }
        let lead = self.lead();
        // A seek that lands after its target (a demuxer index coarser than
        // its packets) starts one lead earlier, then from the start.
        let mut back = lead;
        for attempt in 0..=LANDED_AFTER_RETRIES {
            if index <= back {
                break;
            }
            let from = index - back + self.info.extra_priming;
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
                    break;
                }
                self.fresh_codec()?;
            }
            self.pending.clear();
            self.expected = None;
            self.target = index;
            self.draining = false;
            self.eof = false;
            self.position = index;
            self.run = Run::Placed;
            self.feed_from = None;
            self.first = None;
            self.fault = None;
            match self.fill() {
                Ok(_) => return Ok(()),
                Err(MediaError::Unsupported(_))
                    if self.fault == Some(SeekFault::LandedAfter)
                        && attempt < LANDED_AFTER_RETRIES =>
                {
                    back += lead;
                }
                // Decode from the start.
                Err(MediaError::Unsupported(_)) => break,
                Err(e) => return Err(e),
            }
        }
        self.restart()?;
        self.target = index;
        self.skip_to_target()
    }

    /// How far before its target a seek starts decoding: the pre-roll, and
    /// at least the codec delay a fresh decoder drops, so the first sample
    /// it returns is at or before the target.
    fn lead(&self) -> i64 {
        (self.info.preroll as i64).max(self.codec_delay)
    }

    /// A seek on a coarse timebase: from the last anchor packet at least the
    /// lead before `index` (from the start without one), counting. When
    /// the running decode began at that same place and has not passed
    /// `index`, it continues: the samples are the same, decoded once.
    fn seek_counted(&mut self, index: i64) -> Result<()> {
        let mut from = index - self.lead();
        let mut landed_after = 0;
        loop {
            let anchor = if from > 0 && self.anchor_failures < ANCHOR_FAILURE_LIMIT {
                self.anchor_for(from)
            } else {
                None
            };
            let run = anchor.map_or(Run::Start, |a| Run::Anchor(a.pts));
            if self.run == run && self.position <= index && self.feed_from.is_none() {
                self.target = index;
                return self.skip_to_target();
            }
            let Some(a) = anchor else {
                break;
            };
            self.fault = None;
            match self.seek_to_anchor(a, index) {
                Ok(()) => {
                    self.anchor_failures = 0;
                    return Ok(());
                }
                Err(MediaError::Unsupported(_)) => match self.fault {
                    // Depends on the target: start one anchor earlier.
                    Some(SeekFault::LandedAfter) if landed_after < LANDED_AFTER_RETRIES => {
                        landed_after += 1;
                        from = a.start - 1;
                    }
                    Some(SeekFault::LandedAfter) => break,
                    // The anchor could not be found again or did not place
                    // the first frame: decode from the start, and count it.
                    _ => {
                        self.anchor_failures += 1;
                        break;
                    }
                },
                Err(e) => return Err(e),
            }
        }
        self.restart()?;
        self.target = index;
        self.skip_to_target()
    }

    /// The anchor for program sample `at`, scanning as far as needed.
    fn anchor_for(&mut self, at: i64) -> Option<Anchor> {
        if self.index.is_none() && !self.no_index {
            let probed = self.input.par(self.info.stream);
            match Scan::open(&self.info.path, self.info.stream, probed) {
                Ok(scan) => self.index = Some(PacketIndex::new(scan, self.info.priming)),
                Err(_) => self.no_index = true,
            }
        }
        match self.index.as_mut() {
            Some(index) => index.anchor_for(at),
            None => None,
        }
    }

    /// Starts a fresh decoder exactly at the anchor packet and discards to
    /// `index`.
    fn seek_to_anchor(&mut self, a: Anchor, index: i64) -> Result<()> {
        unsafe {
            let r = ff::avformat_seek_file(
                self.input.fmt,
                self.info.stream as c_int,
                i64::MIN,
                a.pts,
                a.pts,
                0,
            );
            self.rewound = false;
            if r < 0 {
                self.rewind()?;
                self.rewound = true;
            }
        }
        self.fresh_codec()?;
        self.pending.clear();
        self.expected = None;
        self.target = index;
        self.position = index;
        self.draining = false;
        self.eof = false;
        self.run = Run::Anchor(a.pts);
        self.feed_from = Some(a.pts);
        self.first = Some(a);
        let filled = self.fill();
        if filled.is_err() {
            self.feed_from = None;
            self.first = None;
            self.run = Run::Placed;
        }
        filled.map(|_| ())
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
                    if self.feed_from.is_some() {
                        self.fault = Some(SeekFault::AnchorNotFound);
                        return Err(MediaError::unsupported("the anchor packet was not found"));
                    }
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
                if let Some(from) = self.feed_from {
                    let pts = (*self.pkt).pts;
                    if pts == from {
                        // The anchor: the decoder starts here.
                        self.feed_from = None;
                    } else {
                        ff::av_packet_unref(self.pkt);
                        if pts != ff::AV_NOPTS_VALUE && pts > from {
                            // The demuxer landed after the anchor.
                            if self.rewound {
                                self.fault = Some(SeekFault::AnchorNotFound);
                                return Err(MediaError::unsupported(
                                    "the anchor packet was not found",
                                ));
                            }
                            self.rewind()?;
                            self.rewound = true;
                        }
                        continue;
                    }
                }
                self.packets += 1;
                let s = ff::avcodec_send_packet(self.ctx, self.pkt);
                ff::av_packet_unref(self.pkt);
                if s < 0 && s != ff::AVERROR_INVALIDDATA && s != ff::averror(libc::EAGAIN) {
                    check(s, "send audio")?;
                }
                return Ok(());
            }
        }
    }

    /// [`anchored_start`] with this decoder's codec delay: what a fresh
    /// decoder drops from the head of its output (`initial_padding`).
    fn place_anchored(&self, a: &Anchor, pts: i64, n: i64) -> Option<i64> {
        let skip = unsafe { (*self.ctx).initial_padding.max(0) as i64 };
        // The delay in the timebase, rounded to the nearest tick.
        let num = skip as i128 * self.tb.1 as i128;
        let den = self.info.sample_rate as i128 * self.tb.0 as i128;
        let skip_ts = ((2 * num + den) / (2 * den)) as i64;
        anchored_start(a, pts, n, skip, skip_ts)
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
        // The first frame of an anchored seek on a coarse timebase is
        // placed by the anchor the scan counted.
        let anchored = match (self.expected, self.first.take()) {
            (None, Some(a)) => match self.place_anchored(&a, pts, n as i64) {
                Some(start) => Some(start),
                None => {
                    self.fault = Some(SeekFault::Misplaced);
                    return Err(MediaError::unsupported(
                        "the first frame after an anchor is not the anchor's",
                    ));
                }
            },
            _ => None,
        };
        let start = match (self.expected, by_pts) {
            _ if let Some(a) = anchored => a,
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
            self.fault = Some(SeekFault::LandedAfter);
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

    /// A WebM/Opus file from Powermove's FFmpeg build (`PM_FFMPEG_DIR`).
    fn webm_opus(dir: &tempfile::TempDir, name: &str, seconds: u32) -> PathBuf {
        let out = dir.path().join(name);
        let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var("HOME").unwrap())
                    .join("Library/Caches/powermove-engine/ffmpeg/out")
            })
            .join("bin/ffmpeg");
        let source = format!(
            "aevalsrc=exprs='0.3*sin(2*PI*440*t)+0.2*sin(2*PI*3001*t)':s=48000:d={seconds}"
        );
        let status = std::process::Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg(source)
            .args(["-c:a", "libopus"])
            .arg(&out)
            .status()
            .expect("run ffmpeg");
        assert!(status.success());
        out
    }

    /// Anchored seeks that cannot be placed fall back to the decode from
    /// the start; after [`ANCHOR_FAILURE_LIMIT`] in a row the stream counts
    /// from the start for good: blocks read in order continue that decode
    /// (each packet decoded once, the samples exactly the sequential
    /// decode's) instead of retrying a new anchor and restarting for every
    /// block.
    #[test]
    fn failed_anchors_fall_back_after_a_few() {
        let dir = tempfile::Builder::new()
            .prefix("pm-media-audio-unit-")
            .tempdir()
            .unwrap();
        let file = webm_opus(&dir, "failed-anchors.webm", 30);
        let mut d = AudioDecoder::open(&file, None).unwrap();
        let mut pcm = vec![Vec::new()];
        while d.read(&mut pcm, 4096).unwrap() > 0 {}
        let once = d.packets_decoded();

        let mut s = AudioDecoder::open(&file, None).unwrap();
        assert!(!s.info.sample_exact_timestamps && s.info.preroll > 0);
        // Scan the whole file, then move every anchor off its packet (1 ms
        // timebase, 20 ms packets): no anchor can be found again.
        assert!(s.anchor_for(i64::MAX / 8).is_some());
        let index = s.index.as_mut().unwrap();
        assert!(index.anchors.len() > 100);
        for a in &mut index.anchors {
            a.pts += 1;
            a.next = a.next.map(|(p, d)| (p + 1, d));
        }
        let block = 48_000usize;
        let blocks = pcm[0].len().div_ceil(block);
        for k in 0..blocks {
            s.seek_anchored((k * block) as u64).unwrap();
            let mut out = vec![Vec::new()];
            s.read(&mut out, block).unwrap();
            let want = &pcm[0][k * block..(k * block + block).min(pcm[0].len())];
            assert_eq!(&out[0][..], want, "block {k}");
        }
        assert_eq!(s.anchor_failures, ANCHOR_FAILURE_LIMIT);
        // Blocks 1, 2 and 3 fail and decode from the start again (1, 2 and
        // 3 blocks of 50 packets of 20 ms decoded twice); every later block
        // continues the decode.
        let packets = s.packets_decoded();
        assert!(
            packets <= once + 6 * 50 + 8,
            "{blocks} blocks decoded {packets} packets (the stream has {once})"
        );
    }

    /// One anchor that cannot be found again costs one decode from the
    /// start; the seeks after it use their anchors again.
    #[test]
    fn a_stray_failed_anchor_does_not_stick() {
        let dir = tempfile::Builder::new()
            .prefix("pm-media-audio-unit-")
            .tempdir()
            .unwrap();
        let file = webm_opus(&dir, "stray-anchor.webm", 30);
        let mut d = AudioDecoder::open(&file, None).unwrap();
        let mut pcm = vec![Vec::new()];
        while d.read(&mut pcm, 4096).unwrap() > 0 {}

        let mut s = AudioDecoder::open(&file, None).unwrap();
        let target = 20 * 48_000i64;
        let a = s.anchor_for(target - s.lead()).unwrap();
        let index = s.index.as_mut().unwrap();
        let i = index.anchors.iter().position(|x| *x == a).unwrap();
        index.anchors[i].pts += 1;
        index.anchors[i].next = a.next.map(|(p, d)| (p + 1, d));
        s.seek_anchored(target as u64).unwrap();
        assert_eq!(s.anchor_failures, 1);
        let mut out = vec![Vec::new()];
        s.read(&mut out, 4096).unwrap();
        let t = target as usize;
        assert_eq!(&out[0][..], &pcm[0][t..t + 4096]);
        for at in [25 * 48_000u64, 5 * 48_000, 12 * 48_000 + 77] {
            let before = s.packets_decoded();
            s.seek_anchored(at).unwrap();
            let mut out = vec![Vec::new()];
            s.read(&mut out, 4096).unwrap();
            let cost = s.packets_decoded() - before;
            assert!(cost <= 40, "a seek to {at} decoded {cost} packets");
            assert_eq!(s.anchor_failures, 0);
        }
    }

    /// A seek whose first sample lands after its target starts one anchor
    /// earlier: no decode from the start, and nothing counted against the
    /// stream's anchors. With no lead, a target just past an anchor lands
    /// after it: Opus drops its 312-sample pre-skip from a fresh decoder's
    /// first frame.
    #[test]
    fn a_seek_that_lands_after_its_target_starts_an_anchor_earlier() {
        let dir = tempfile::Builder::new()
            .prefix("pm-media-audio-unit-")
            .tempdir()
            .unwrap();
        let file = webm_opus(&dir, "landed-after.webm", 30);
        let mut s = AudioDecoder::open(&file, None).unwrap();
        assert!(s.codec_delay > 100);
        let a = s.anchor_for(20 * 48_000).unwrap();
        let target = a.start + 100;
        let (preroll, delay) = (s.info.preroll, s.codec_delay);
        s.info.preroll = 0;
        s.codec_delay = 0;
        let before = s.packets_decoded();
        s.seek_anchored(target as u64).unwrap();
        let cost = s.packets_decoded() - before;
        // It runs from the anchor before `a`.
        let earlier = s.anchor_for(a.start - 1).unwrap();
        assert!(earlier.start < a.start);
        assert_eq!(s.run, Run::Anchor(earlier.pts));
        assert_eq!(s.anchor_failures, 0);
        assert!(cost <= 12, "the seek decoded {cost} packets");
        assert_eq!(s.position(), target as u64);
        s.info.preroll = preroll;
        s.codec_delay = delay;
    }

    #[test]
    fn default_speakers_follow_ffmpeg() {
        use Speaker::*;
        assert_eq!(default_speakers(1), [FrontCenter]);
        assert_eq!(default_speakers(2), [FrontLeft, FrontRight]);
        assert_eq!(default_speakers(3), [FrontLeft, FrontRight, LowFrequency]);
        assert_eq!(
            default_speakers(5),
            [FrontLeft, FrontRight, FrontCenter, BackLeft, BackRight]
        );
        assert_eq!(
            default_speakers(6),
            [
                FrontLeft,
                FrontRight,
                FrontCenter,
                LowFrequency,
                BackLeft,
                BackRight
            ]
        );
        assert!(default_speakers(0).is_empty());
    }
}
