//! WebM through FFmpeg (plan section 17, "Muxers" and "Formats at cutover"): VP9
//! or VP8 from libvpx, alpha as libvpx's second stream (Matroska `AlphaMode` and
//! `BlockAdditional`), Opus from libopus at 48 kHz. Opaque exports remux the
//! encoded packets into the current exporter's EBML layout.
//!
//! Frame-exact: every frame of the range is encoded, frame n at presentation
//! time n / rate (Matroska stores milliseconds; the stream also declares its
//! frame duration). The video is the engine's own conversion of the working
//! frames (`convert`: BT.709 matrix, limited range, left-sited 4:2:0, blue-noise
//! dither), tagged with the export's colour description; libvpx runs with
//! today's realtime settings. Keyframes are forced every rounded two seconds;
//! rate control follows the quality bitrate.
//!
//! Verification demuxes the whole file (frame count, every frame's timestamp,
//! codec, size, tags, range, alpha mode), decodes the first, middle, and last
//! frames with libvpx (which decodes the alpha stream) to compare block means
//! with the encoder input, and decodes the audio to count its samples.

use std::ffi::{CStr, CString, c_int};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, Instant};

use pm_media::sys::ffmpeg as ff;

use crate::Delivered;
use crate::Error;
use crate::atomic::AtomicOutput;
use crate::convert::{EncoderInput, Plane, PlaneData};
use crate::pipeline::Sink;
use crate::settings::{
    AlphaOutput, AudioCodec, Codec, Container, ExportSpec, InputFormat, Overwrite,
};
use crate::verify::{self, AudioReport, BlockSample, PixelCheck, VerifyReport};

/// Matroska's timestamp unit.
const MS: ff::AVRational = ff::AVRational { num: 1, den: 1000 };
/// POSIX EAGAIN on Darwin and Linux (`AVERROR(EAGAIN)`).
#[cfg(target_os = "macos")]
const EAGAIN: c_int = 35;
#[cfg(not(target_os = "macos"))]
const EAGAIN: c_int = 11;

fn err(what: &str, e: c_int) -> Error {
    Error::Encode(format!("{what}: {}", pm_media::ff::err_str(e)))
}

fn check(what: &str, e: c_int) -> Result<c_int, Error> {
    if e < 0 { Err(err(what, e)) } else { Ok(e) }
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("no NUL in FFmpeg option strings")
}

/// An FFmpeg option dictionary, freed on drop.
struct Dict(*mut ff::AVDictionary);

impl Dict {
    fn new(pairs: &[(&str, String)]) -> Dict {
        let mut d = Dict(ptr::null_mut());
        for (k, v) in pairs {
            let (k, v) = (cstr(k), cstr(v));
            unsafe { ff::av_dict_set(&mut d.0, k.as_ptr(), v.as_ptr(), 0) };
        }
        d
    }
}

impl Drop for Dict {
    fn drop(&mut self) {
        unsafe { ff::av_dict_free(&mut self.0) };
    }
}

/// An encoder context, freed on drop.
struct Encoder {
    ctx: *mut ff::AVCodecContext,
    stream: c_int,
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { ff::avcodec_free_context(&mut self.ctx) };
    }
}

fn rational(num: i64, den: i64) -> ff::AVRational {
    ff::AVRational {
        num: num as c_int,
        den: den as c_int,
    }
}

/// Opt-in parity capture of the exact native boundaries fed to the encoders.
/// The caller owns and removes the directory on either success or failure.
struct InputCapture {
    i420: File,
    f32_pcm: File,
}

impl InputCapture {
    fn from_env() -> Result<Option<Self>, Error> {
        if std::env::var("POWERMOVE_BACKGROUND_TEST").as_deref() != Ok("1") {
            return Ok(None);
        }
        let Some(dir) = std::env::var_os("PM_EXPORT_INPUT_CAPTURE_DIR") else {
            return Ok(None);
        };
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Io(format!("create WebM input capture: {e}")))?;
        let open = |name| {
            File::create(dir.join(name))
                .map_err(|e| Error::Io(format!("open WebM input capture {name}: {e}")))
        };
        Ok(Some(Self {
            i420: open("native.i420")?,
            f32_pcm: open("native.f32")?,
        }))
    }

    fn write_i420(&mut self, input: &EncoderInput) -> Result<(), Error> {
        for plane in &input.planes {
            let PlaneData::U8(bytes) = &plane.data else {
                return Err(Error::InvalidFrame(
                    "WebM input capture expected 8-bit YUV".into(),
                ));
            };
            self.i420
                .write_all(bytes)
                .map_err(|e| Error::Io(format!("write WebM I420 capture: {e}")))?;
        }
        Ok(())
    }

    fn write_audio(&mut self, samples: &[f32]) -> Result<(), Error> {
        let mut bytes = Vec::with_capacity(std::mem::size_of_val(samples));
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.f32_pcm
            .write_all(&bytes)
            .map_err(|e| Error::Io(format!("write WebM PCM capture: {e}")))
    }
}

/// The WebM sink: libvpx video, libopus audio, FFmpeg's muxer, then verification
/// and the atomic move.
pub struct WebmSink {
    // Drop order: the muxer closes its file before `output` removes it.
    fmt: *mut ff::AVFormatContext,
    video: Encoder,
    audio: Option<Encoder>,
    frame: *mut ff::AVFrame,
    packet: *mut ff::AVPacket,
    audio_frame: *mut ff::AVFrame,
    /// Interleaved samples not yet a whole Opus frame.
    pending_audio: Vec<f32>,
    audio_pts: i64,
    audio_pushed: u64,
    /// Opus's lookahead in 48 kHz samples. The legacy WebCodecs muxer leaves
    /// its packets at nonnegative timestamps and does not write CodecDelay.
    audio_lookahead: i64,
    /// WebCodecs tracks output timestamps outside libvpx; Chromium always
    /// encodes with vpx timestamp 0 and the frame duration for rate control.
    video_packet_index: u64,
    header_written: bool,
    trailer_written: bool,
    /// Taken by the commit; dropped (removing the temporary file) otherwise.
    output: Option<AtomicOutput>,
    spec: ExportSpec,
    sampled: Vec<u64>,
    blocks: Vec<BlockSample>,
    capture: Option<InputCapture>,
}

// The FFmpeg contexts are used from one thread at a time.
unsafe impl Send for WebmSink {}

/// Chromium's software-encoder thread tiers (media/base/video_encoder.cc).
fn threads(width: u32, height: u32) -> i64 {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get()) as i64;
    let area = u64::from(width) * u64::from(height);
    let want = if area >= 3840 * 2160 {
        16
    } else if area >= 1920 * 1080 {
        8
    } else if area >= 1280 * 720 {
        4
    } else if area >= 640 * 480 {
        2
    } else {
        1
    };
    want.min(cores)
}

impl WebmSink {
    pub fn new(spec: &ExportSpec) -> Result<Self, Error> {
        spec.validate()?;
        if spec.container != Container::Webm {
            return Err(Error::InvalidSpec(format!(
                "{:?} is not written by the WebM muxer",
                spec.container
            )));
        }
        let output = match spec.overwrite {
            Overwrite::InPlace => AtomicOutput::in_place(&spec.destination, "webm", spec.mode)?,
            o => AtomicOutput::new(&spec.destination, o, spec.mode)?,
        };
        let mut this = WebmSink {
            fmt: ptr::null_mut(),
            video: Encoder {
                ctx: ptr::null_mut(),
                stream: 0,
            },
            audio: None,
            frame: ptr::null_mut(),
            packet: ptr::null_mut(),
            audio_frame: ptr::null_mut(),
            pending_audio: vec![],
            audio_pts: 0,
            audio_pushed: 0,
            audio_lookahead: 0,
            video_packet_index: 0,
            header_written: false,
            trailer_written: false,
            output: Some(output),
            spec: spec.clone(),
            sampled: verify::sampled_frames(spec.frames),
            blocks: vec![],
            capture: None,
        };
        this.open()?;
        this.capture = InputCapture::from_env()?;
        Ok(this)
    }

    fn temp_path(&self) -> std::path::PathBuf {
        self.output
            .as_ref()
            .expect("the output lives until the commit")
            .temp_path()
            .to_path_buf()
    }

    fn open(&mut self) -> Result<(), Error> {
        // FFmpeg's log level (fatal only unless `PM_FFMPEG_LOG` says otherwise).
        pm_media::ff::init();
        let spec = &self.spec;
        let path = cstr(&self.temp_path().to_string_lossy());
        let webm = cstr("webm");
        unsafe {
            check(
                "allocate the WebM muxer",
                ff::avformat_alloc_output_context2(
                    &mut self.fmt,
                    ptr::null(),
                    webm.as_ptr(),
                    path.as_ptr(),
                ),
            )?;
            // Video.
            let (name, codec_id) = match spec.codec {
                Codec::Vp9 { .. } => ("libvpx-vp9", ff::AV_CODEC_ID_VP9),
                Codec::Vp8 { .. } => ("libvpx", ff::AV_CODEC_ID_VP8),
                _ => {
                    return Err(Error::InvalidSpec(format!(
                        "{} is not a WebM codec",
                        spec.codec.name()
                    )));
                }
            };
            let codec = ff::avcodec_find_encoder_by_name(cstr(name).as_ptr());
            if codec.is_null() {
                return Err(Error::Encode(format!(
                    "this FFmpeg build has no {name} encoder"
                )));
            }
            let ctx = ff::avcodec_alloc_context3(codec);
            self.video.ctx = ctx;
            let c = &mut *ctx;
            let alpha = spec.alpha != AlphaOutput::None;
            c.codec_id = codec_id;
            c.width = spec.width as c_int;
            c.height = spec.height as c_int;
            c.pix_fmt = if alpha {
                ff::AV_PIX_FMT_YUVA420P
            } else {
                ff::AV_PIX_FMT_YUV420P
            };
            // Chromium gives libvpx the rounded WebCodecs timestamps in
            // microseconds. Its rate control uses this timebase too.
            c.time_base = rational(1, 1_000_000);
            c.framerate = rational(spec.rate.num(), spec.rate.den());
            c.bit_rate = spec.codec.bitrate_bps().unwrap_or(16_000_000) as i64;
            // VideoEncoder has no configured keyframe interval. It passes an
            // explicit force-keyframe flag every rounded two seconds instead.
            c.gop_size = 128;
            c.qmin = 2;
            c.qmax = 58;
            c.rc_buffer_size = c.bit_rate as c_int;
            c.rc_initial_buffer_occupancy = (c.bit_rate * 3 / 5) as c_int;
            // The export's colour description (H.273 code points), limited
            // range: what `convert` wrote.
            let (primaries, transfer, matrix) = spec.tags.cicp();
            c.color_primaries = primaries.into();
            c.color_trc = transfer.into();
            c.colorspace = matrix.into();
            c.color_range = ff::AVCOL_RANGE_MPEG;
            c.chroma_sample_location = ff::AVCHROMA_LOC_LEFT;
            c.thread_count = threads(spec.width, spec.height) as c_int;
            if (*(*self.fmt).oformat).flags & ff::AVFMT_GLOBALHEADER as c_int != 0 {
                c.flags |= ff::AV_CODEC_FLAG_GLOBAL_HEADER as c_int;
            }
            let mut opts = vec![
                ("deadline", "realtime".to_string()),
                (
                    "cpu-used",
                    if matches!(spec.codec, Codec::Vp9 { .. }) {
                        "7"
                    } else {
                        "-6"
                    }
                    .to_string(),
                ),
                ("lag-in-frames", "0".to_string()),
                ("auto-alt-ref", "0".to_string()),
                ("undershoot-pct", "50".to_string()),
                ("overshoot-pct", "50".to_string()),
                ("drop-threshold", "0".to_string()),
                ("static-thresh", "1".to_string()),
                ("max-intra-rate", "600".to_string()),
            ];
            if matches!(spec.codec, Codec::Vp9 { .. }) {
                opts.push(("row-mt", "1".into()));
                let tile_columns = spec.width.div_ceil(256);
                let log2_tiles = (31 - tile_columns.leading_zeros()).min(6);
                opts.push(("tile-columns", log2_tiles.to_string()));
            }
            let mut dict = Dict::new(&opts);
            check(
                &format!("open {name}"),
                ff::avcodec_open2(ctx, codec, &mut dict.0),
            )?;
            let st = ff::avformat_new_stream(self.fmt, ptr::null());
            if st.is_null() {
                return Err(Error::Mux("could not add the video stream".into()));
            }
            check(
                "video stream parameters",
                ff::avcodec_parameters_from_context((*st).codecpar, ctx),
            )?;
            (*st).time_base = c.time_base;
            (*st).avg_frame_rate = c.framerate;
            (*st).r_frame_rate = c.framerate;
            self.video.stream = (*st).index;
            if alpha {
                // The muxer writes `AlphaMode` from this tag.
                let (k, v) = (cstr("alpha_mode"), cstr("1"));
                ff::av_dict_set(&mut (*st).metadata, k.as_ptr(), v.as_ptr(), 0);
            }
            // Audio.
            if let Some(a) = spec.audio {
                if a.codec != AudioCodec::Opus {
                    return Err(Error::InvalidSpec("WebM audio is Opus".into()));
                }
                let codec = ff::avcodec_find_encoder_by_name(cstr("libopus").as_ptr());
                if codec.is_null() {
                    return Err(Error::Encode("this FFmpeg build has no libopus".into()));
                }
                let actx = ff::avcodec_alloc_context3(codec);
                let ac = &mut *actx;
                ac.sample_rate = a.sample_rate as c_int;
                ac.sample_fmt = ff::AV_SAMPLE_FMT_FLT;
                // The legacy AudioEncoder config sets 160 kbps for the whole
                // stream. Chromium 152 defaults to 20 ms, VBR, application
                // audio, signal auto, and complexity 9 on macOS. FFmpeg's
                // libopus wrapper otherwise defaults to complexity 10.
                ac.bit_rate = 160_000;
                ac.compression_level = 9;
                ac.time_base = rational(1, i64::from(a.sample_rate));
                ff::av_channel_layout_default(&mut ac.ch_layout, c_int::from(a.channels));
                if (*(*self.fmt).oformat).flags & ff::AVFMT_GLOBALHEADER as c_int != 0 {
                    ac.flags |= ff::AV_CODEC_FLAG_GLOBAL_HEADER as c_int;
                }
                // Owned from here: freed on every exit path.
                self.audio = Some(Encoder {
                    ctx: actx,
                    stream: 0,
                });
                let mut dict = Dict::new(&[
                    ("application", "audio".into()),
                    ("frame_duration", "20".into()),
                    ("vbr", "on".into()),
                    ("packet_loss", "0".into()),
                    ("fec", "0".into()),
                    ("dtx", "0".into()),
                ]);
                check("open libopus", ff::avcodec_open2(actx, codec, &mut dict.0))?;
                self.audio_lookahead = ac.initial_padding as i64;
                let st = ff::avformat_new_stream(self.fmt, ptr::null());
                if st.is_null() {
                    return Err(Error::Mux("could not add the audio stream".into()));
                }
                check(
                    "audio stream parameters",
                    ff::avcodec_parameters_from_context((*st).codecpar, actx),
                )?;
                // Today's hand-written WebM track writes OpusHead's pre-skip,
                // but no CodecDelay. Keeping FFmpeg's initial_padding here
                // would add CodecDelay and shift decoded audio by 312 samples.
                (*(*st).codecpar).initial_padding = 0;
                (*st).time_base = ac.time_base;
                if let Some(a) = self.audio.as_mut() {
                    a.stream = (*st).index;
                }
                self.audio_frame = ff::av_frame_alloc();
            }
            check(
                "open the WebM file",
                ff::avio_open(
                    &mut (*self.fmt).pb,
                    path.as_ptr(),
                    ff::AVIO_FLAG_WRITE as c_int,
                ),
            )?;
            check(
                "write the WebM header",
                ff::avformat_write_header(self.fmt, ptr::null_mut()),
            )?;
            self.header_written = true;
            self.frame = ff::av_frame_alloc();
            self.packet = ff::av_packet_alloc();
            let f = &mut *self.frame;
            f.format = c.pix_fmt;
            f.width = c.width;
            f.height = c.height;
            f.color_primaries = ff::AVCOL_PRI_UNSPECIFIED;
            f.color_trc = ff::AVCOL_TRC_UNSPECIFIED;
            f.colorspace = ff::AVCOL_SPC_BT709;
            f.color_range = ff::AVCOL_RANGE_MPEG;
            f.chroma_location = ff::AVCHROMA_LOC_LEFT;
            check("allocate a frame", ff::av_frame_get_buffer(self.frame, 0))?;
        }
        Ok(())
    }

    /// Sends a frame (or flushes with null) and writes every packet out.
    fn drain(&mut self, audio: bool, frame: *mut ff::AVFrame) -> Result<(), Error> {
        let enc = if audio {
            self.audio.as_ref().expect("audio encoder")
        } else {
            &self.video
        };
        let (ctx, stream) = (enc.ctx, enc.stream);
        unsafe {
            check("encode", ff::avcodec_send_frame(ctx, frame))?;
            loop {
                let r = ff::avcodec_receive_packet(ctx, self.packet);
                if r == -EAGAIN || r == ff::AVERROR_EOF {
                    return Ok(());
                }
                check("encode", r)?;
                let st = *(*self.fmt).streams.add(stream as usize);
                if audio {
                    // WebCodecs exposes the first Opus chunk at timestamp 0.
                    // libopus reports its lookahead as a negative first PTS.
                    (*self.packet).pts += self.audio_lookahead;
                    (*self.packet).dts += self.audio_lookahead;
                    // The legacy muxer writes SimpleBlocks only. In particular
                    // it has no end DiscardPadding; FFmpeg derives that from
                    // AV_PKT_DATA_SKIP_SAMPLES unless we remove this side data.
                    ff::av_packet_free_side_data(self.packet);
                } else {
                    let rate = self.spec.rate.to_f64();
                    let pts =
                        ((self.video_packet_index as f64 / rate) * 1_000_000.0).round() as i64;
                    (*self.packet).pts = pts;
                    (*self.packet).dts = pts;
                    self.video_packet_index += 1;
                }
                ff::av_packet_rescale_ts(self.packet, (*ctx).time_base, (*st).time_base);
                (*self.packet).stream_index = stream;
                let w = ff::av_interleaved_write_frame(self.fmt, self.packet);
                if w < 0 {
                    return Err(Error::Mux(format!(
                        "write a packet: {}",
                        pm_media::ff::err_str(w)
                    )));
                }
            }
        }
    }

    fn encode_audio_frames(&mut self, flush: bool) -> Result<(), Error> {
        let Some(enc) = self.audio.as_ref() else {
            return Ok(());
        };
        let ctx = enc.ctx;
        let channels = self.spec.audio.map_or(2, |a| a.channels as usize);
        let frame_size = unsafe { (*ctx).frame_size }.max(1) as usize;
        loop {
            let available = self.pending_audio.len() / channels;
            let n = if available >= frame_size {
                frame_size
            } else if flush && available > 0 {
                available
            } else {
                break;
            };
            unsafe {
                let f = &mut *self.audio_frame;
                ff::av_frame_unref(self.audio_frame);
                f.format = ff::AV_SAMPLE_FMT_FLT;
                f.nb_samples = n as c_int;
                f.sample_rate = (*ctx).sample_rate;
                check(
                    "audio channel layout",
                    ff::av_channel_layout_copy(&mut f.ch_layout, &(*ctx).ch_layout),
                )?;
                check(
                    "allocate an audio frame",
                    ff::av_frame_get_buffer(self.audio_frame, 0),
                )?;
                let dst = std::slice::from_raw_parts_mut(f.data[0] as *mut f32, n * channels);
                dst.copy_from_slice(&self.pending_audio[..n * channels]);
                f.pts = self.audio_pts;
            }
            self.pending_audio.drain(..n * channels);
            self.audio_pts += n as i64;
            self.drain(true, self.audio_frame)?;
        }
        if flush {
            self.drain(true, ptr::null_mut())?;
        }
        Ok(())
    }

    fn write_planes(&mut self, input: &EncoderInput, index: u64) -> Result<(), Error> {
        unsafe {
            check(
                "make the frame writable",
                ff::av_frame_make_writable(self.frame),
            )?;
            let f = &mut *self.frame;
            for (k, plane) in input.planes.iter().enumerate() {
                let PlaneData::U8(data) = &plane.data else {
                    return Err(Error::InvalidFrame("libvpx takes 8-bit planes".into()));
                };
                let base = f.data[k];
                let stride = f.linesize[k] as usize;
                if base.is_null() || stride < plane.row_samples {
                    return Err(Error::Encode(format!("frame plane {k} is too small")));
                }
                for y in 0..plane.rows {
                    let dst =
                        std::slice::from_raw_parts_mut(base.add(y * stride), plane.row_samples);
                    dst.copy_from_slice(&data[y * plane.row_samples..(y + 1) * plane.row_samples]);
                }
            }
            let rate = self.spec.rate.to_f64();
            f.pts = 0;
            f.duration = (1_000_000.0 / rate).round() as i64;
            let key_interval = (2.0 * rate).round().max(1.0) as u64;
            f.pict_type = if index.is_multiple_of(key_interval) {
                ff::AV_PICTURE_TYPE_I
            } else {
                ff::AV_PICTURE_TYPE_NONE
            };
        }
        self.drain(false, self.frame)
    }

    fn close_file(&mut self) {
        unsafe {
            if !self.fmt.is_null() {
                if !(*self.fmt).pb.is_null() {
                    ff::avio_closep(&mut (*self.fmt).pb);
                }
                ff::avformat_free_context(self.fmt);
                self.fmt = ptr::null_mut();
            }
        }
    }
}

impl Drop for WebmSink {
    fn drop(&mut self) {
        unsafe {
            ff::av_frame_free(&mut self.frame);
            ff::av_frame_free(&mut self.audio_frame);
            ff::av_packet_free(&mut self.packet);
        }
        self.close_file();
    }
}

impl Sink for WebmSink {
    type Output = Delivered;

    fn video(&mut self, index: u64, input: &EncoderInput) -> Result<(), Error> {
        let want = self.spec.input_format();
        if input.format != want {
            return Err(Error::InvalidFrame(format!(
                "WebM takes '{}' encoder input",
                String::from_utf8_lossy(&want.fourcc())
            )));
        }
        if self.sampled.contains(&index) {
            self.blocks.extend(verify::block_means(input, index));
        }
        if let Some(capture) = self.capture.as_mut() {
            capture.write_i420(input)?;
        }
        self.write_planes(input, index)?;
        // Keep audio interleaved with video.
        self.encode_audio_frames(false)
    }

    fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        if self.audio.is_none() {
            return Err(Error::InvalidFrame("this export has no audio track".into()));
        }
        if let Some(capture) = self.capture.as_mut() {
            capture.write_audio(interleaved)?;
        }
        self.pending_audio.extend_from_slice(interleaved);
        let channels = self.spec.audio.map_or(2, |a| a.channels as usize);
        self.audio_pushed += (interleaved.len() / channels) as u64;
        Ok(())
    }

    fn finish(mut self) -> Result<Delivered, Error> {
        self.drain(false, ptr::null_mut())?;
        self.encode_audio_frames(true)?;
        unsafe {
            check("write the WebM trailer", ff::av_write_trailer(self.fmt))?;
        }
        self.trailer_written = true;
        self.close_file();
        let temp = self.temp_path();
        let mut report = verify_webm(&temp, &self.spec, &self.blocks, self.audio_pushed)?;
        report.bytes = std::fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
        let path = self
            .output
            .take()
            .expect("the output lives until the commit")
            .commit()?;
        Ok(Delivered { path, report })
    }
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// An open input, freed on drop.
struct Demuxer(*mut ff::AVFormatContext);

impl Drop for Demuxer {
    fn drop(&mut self) {
        unsafe { ff::avformat_close_input(&mut self.0) };
    }
}

struct Decoder(*mut ff::AVCodecContext);

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { ff::avcodec_free_context(&mut self.0) };
    }
}

fn open_decoder(st: *mut ff::AVStream, name: Option<&str>) -> Result<Decoder, Error> {
    unsafe {
        let par = (*st).codecpar;
        let codec = match name {
            Some(n) => ff::avcodec_find_decoder_by_name(cstr(n).as_ptr()),
            None => ff::avcodec_find_decoder((*par).codec_id),
        };
        if codec.is_null() {
            return Err(Error::Verify("no decoder for the stream".into()));
        }
        let ctx = Decoder(ff::avcodec_alloc_context3(codec));
        check(
            "decoder parameters",
            ff::avcodec_parameters_to_context(ctx.0, par),
        )?;
        (*ctx.0).pkt_timebase = (*st).time_base;
        (*ctx.0).thread_count = 4;
        check(
            "open the decoder",
            ff::avcodec_open2(ctx.0, codec, ptr::null_mut()),
        )?;
        Ok(ctx)
    }
}

fn frame_input(f: &ff::AVFrame, alpha: bool) -> Result<EncoderInput, Error> {
    let (w, h, strides) = frame_layout(f, alpha)?;
    let dims = [(w, h), (w / 2, h / 2), (w / 2, h / 2), (w, h)];
    let planes = dims
        .iter()
        .enumerate()
        .take(if alpha { 4 } else { 3 })
        .map(|(k, &(pw, ph))| copy_frame_plane(f, k, strides[k], 0, 0, pw, ph))
        .collect();
    Ok(EncoderInput {
        format: if alpha {
            InputFormat::I420a
        } else {
            InputFormat::I420
        },
        width: w as u32,
        height: h as u32,
        planes,
    })
}

fn frame_layout(f: &ff::AVFrame, alpha: bool) -> Result<(usize, usize, [usize; 4]), Error> {
    if f.width <= 0 || f.height <= 0 {
        return Err(Error::Verify(format!(
            "invalid decoded frame dimensions {}x{}",
            f.width, f.height
        )));
    }
    let (w, h) = (f.width as usize, f.height as usize);
    let want = if alpha {
        ff::AV_PIX_FMT_YUVA420P
    } else {
        ff::AV_PIX_FMT_YUV420P
    };
    if f.format != want {
        return Err(Error::Verify(format!(
            "decoded pixel format {}, expected {}",
            pix_name(f.format),
            pix_name(want)
        )));
    }
    let widths = [w, w / 2, w / 2, w];
    let heights = [h, h / 2, h / 2, h];
    let mut strides = [0; 4];
    for k in 0..if alpha { 4 } else { 3 } {
        let stride = usize::try_from(f.linesize[k]).unwrap_or(0);
        if (widths[k] != 0 && heights[k] != 0 && f.data[k].is_null())
            || (heights[k] != 0 && stride < widths[k])
        {
            return Err(Error::Verify(format!(
                "invalid decoded plane {k} (stride {}, width {})",
                f.linesize[k], widths[k]
            )));
        }
        strides[k] = stride;
    }
    Ok((w, h, strides))
}

fn copy_frame_plane(
    f: &ff::AVFrame,
    k: usize,
    stride: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Plane {
    let mut bytes = Vec::with_capacity(width * height);
    if width != 0 {
        for row in y..y + height {
            bytes.extend_from_slice(unsafe {
                std::slice::from_raw_parts(f.data[k].add(row * stride + x), width)
            });
        }
    }
    Plane {
        data: PlaneData::U8(bytes),
        row_samples: width,
        rows: height,
    }
}

#[derive(Default)]
struct SampledPixelTimes {
    crop: Duration,
    means: Duration,
}

fn frame_blocks(
    f: &ff::AVFrame,
    alpha: bool,
    frame: u64,
    expected_size: (u32, u32),
    mut times: Option<&mut SampledPixelTimes>,
) -> Result<Vec<BlockSample>, Error> {
    let started = times.as_ref().map(|_| Instant::now());
    let means_before = times.as_ref().map_or(Duration::ZERO, |t| t.means);
    let (w, h, strides) = frame_layout(f, alpha)?;
    if (w as u32, h as u32) != expected_size {
        return Err(Error::Verify(format!(
            "decoded frame dimensions {}x{}, expected {}x{}",
            w, h, expected_size.0, expected_size.1
        )));
    }
    // The existing full-plane conversion handles dimensions for which chroma
    // crops cannot keep the original 4:2:0 addressing and block geometry.
    if w < verify::BLOCK as usize
        || h < verify::BLOCK as usize
        || !w.is_multiple_of(2)
        || !h.is_multiple_of(2)
    {
        let input = frame_input(f, alpha)?;
        let means_started = times.as_ref().map(|_| Instant::now());
        let blocks = verify::block_means(&input, frame);
        if let (Some(times), Some(means_started)) = (times.as_deref_mut(), means_started) {
            times.means += means_started.elapsed();
        }
        if let (Some(times), Some(started)) = (times, started) {
            times.crop += started.elapsed().saturating_sub(times.means - means_before);
        }
        return Ok(blocks);
    }
    let mut blocks = Vec::with_capacity(9);
    for region in verify::block_regions(w as u32, h as u32) {
        let x = region.x as usize;
        let y = region.y as usize;
        let width = region.width as usize;
        let height = region.height as usize;
        let planes = (0..if alpha { 4 } else { 3 })
            .map(|k| {
                let divisor = if k == 1 || k == 2 { 2 } else { 1 };
                copy_frame_plane(
                    f,
                    k,
                    strides[k],
                    x / divisor,
                    y / divisor,
                    width / divisor,
                    height / divisor,
                )
            })
            .collect();
        let crop = EncoderInput {
            format: if alpha {
                InputFormat::I420a
            } else {
                InputFormat::I420
            },
            width: region.width,
            height: region.height,
            planes,
        };
        let means_started = times.as_ref().map(|_| Instant::now());
        let mut block = verify::block_means(&crop, frame)
            .pop()
            .expect("a cropped block has one region");
        if let (Some(times), Some(means_started)) = (times.as_deref_mut(), means_started) {
            times.means += means_started.elapsed();
        }
        block.x = region.x;
        block.y = region.y;
        blocks.push(block);
    }
    if let (Some(times), Some(started)) = (times, started) {
        times.crop += started.elapsed().saturating_sub(times.means - means_before);
    }
    Ok(blocks)
}

fn pix_name(f: c_int) -> String {
    unsafe {
        let p = ff::av_get_pix_fmt_name(f);
        if p.is_null() {
            format!("{f}")
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// Probes and decodes a written WebM against what the export asked for.
pub fn verify_webm(
    path: &Path,
    spec: &ExportSpec,
    blocks: &[BlockSample],
    audio_pushed: u64,
) -> Result<VerifyReport, Error> {
    let profiling = std::env::var("PM_EXPORT_PROFILE").as_deref() == Ok("1");
    let verify_started = profiling.then(Instant::now);
    pm_media::ff::init();
    let mut problems: Vec<String> = vec![];
    let alpha = spec.alpha != AlphaOutput::None;
    let p = cstr(&path.to_string_lossy());
    let open_started = profiling.then(Instant::now);
    let input = unsafe {
        let mut fmt: *mut ff::AVFormatContext = ptr::null_mut();
        check(
            "open the written WebM",
            ff::avformat_open_input(&mut fmt, p.as_ptr(), ptr::null(), ptr::null_mut()),
        )
        .map_err(|e| Error::Verify(e.to_string()))?;
        let d = Demuxer(fmt);
        check(
            "read the written WebM",
            ff::avformat_find_stream_info(d.0, ptr::null_mut()),
        )
        .map_err(|e| Error::Verify(e.to_string()))?;
        d
    };
    let open_ms = open_started.map_or(0.0, |started| started.elapsed().as_secs_f64() * 1e3);
    let scan_started = profiling.then(Instant::now);
    let fmt = input.0;
    let streams: Vec<*mut ff::AVStream> = unsafe {
        (0..(*fmt).nb_streams as usize)
            .map(|i| *(*fmt).streams.add(i))
            .collect()
    };
    let kind = |st: *mut ff::AVStream| unsafe { (*(*st).codecpar).codec_type };
    let Some(&vst) = streams.iter().find(|&&s| kind(s) == ff::AVMEDIA_TYPE_VIDEO) else {
        return Err(Error::Verify("the file has no video stream".into()));
    };
    let ast = streams
        .iter()
        .copied()
        .find(|&s| kind(s) == ff::AVMEDIA_TYPE_AUDIO);
    let vpar = unsafe { &*(*vst).codecpar };
    let (want_id, codec) = match spec.codec {
        Codec::Vp9 { .. } => (ff::AV_CODEC_ID_VP9, "vp09"),
        _ => (ff::AV_CODEC_ID_VP8, "vp08"),
    };
    if vpar.codec_id != want_id {
        problems.push(format!("codec id {}, expected {codec}", vpar.codec_id));
    }
    if (vpar.width as u32, vpar.height as u32) != (spec.width, spec.height) {
        problems.push(format!(
            "frame size {}×{}, expected {}×{}",
            vpar.width, vpar.height, spec.width, spec.height
        ));
    }
    // The export's colour description, limited range (Matroska `Colour`).
    let cicp = (
        vpar.color_primaries as u16,
        vpar.color_trc as u16,
        vpar.color_space as u16,
    );
    let expected_cicp = spec.tags.cicp();
    if cicp != expected_cicp {
        problems.push(format!(
            "colour {}-{}-{}, expected {}-{}-{}",
            cicp.0, cicp.1, cicp.2, expected_cicp.0, expected_cicp.1, expected_cicp.2,
        ));
    }
    let full_range = vpar.color_range == ff::AVCOL_RANGE_JPEG;
    let expected_range = ff::AVCOL_RANGE_MPEG;
    if vpar.color_range != expected_range {
        problems.push(format!(
            "range {}, expected {expected_range}",
            vpar.color_range
        ));
    }
    let alpha_tag = unsafe {
        let k = cstr("alpha_mode");
        let e = ff::av_dict_get((*vst).metadata, k.as_ptr(), ptr::null(), 0);
        !e.is_null() && CStr::from_ptr((*e).value).to_bytes() == b"1"
    };
    if alpha_tag != alpha {
        problems.push(format!(
            "AlphaMode {}, expected {}",
            alpha_tag as u8, alpha as u8
        ));
    }
    // Every packet: the frame count and each frame's timestamp.
    let codec_tb = rational(spec.rate.den(), spec.rate.num());
    let vtb = unsafe { (*vst).time_base };
    let vindex = unsafe { (*vst).index };
    let aindex = ast.map(|a| unsafe { (*a).index });
    let mut frames = 0u64;
    let mut late: Vec<(u64, i64, i64)> = vec![];
    let mut packet = unsafe { ff::av_packet_alloc() };
    let audio_dec = match ast {
        Some(a) => Some(open_decoder(a, None)?),
        None => None,
    };
    let mut audio_frame = unsafe { ff::av_frame_alloc() };
    let mut audio_samples = 0u64;
    let mut decode_audio = |pkt: *mut ff::AVPacket| -> Result<(), Error> {
        let Some(dec) = audio_dec.as_ref() else {
            return Ok(());
        };
        unsafe {
            check("decode audio", ff::avcodec_send_packet(dec.0, pkt))
                .map_err(|e| Error::Verify(e.to_string()))?;
            loop {
                let r = ff::avcodec_receive_frame(dec.0, audio_frame);
                if r == -EAGAIN || r == ff::AVERROR_EOF {
                    return Ok(());
                }
                check("decode audio", r).map_err(|e| Error::Verify(e.to_string()))?;
                audio_samples += (*audio_frame).nb_samples as u64;
            }
        }
    };
    unsafe {
        while ff::av_read_frame(fmt, packet) >= 0 {
            let idx = (*packet).stream_index;
            if idx == vindex {
                let want = ff::av_rescale_q(frames as i64, codec_tb, vtb);
                if (*packet).pts != want && late.len() < 4 {
                    late.push((frames, (*packet).pts, want));
                }
                frames += 1;
            } else if Some(idx) == aindex {
                decode_audio(packet)?;
            }
            ff::av_packet_unref(packet);
        }
        decode_audio(ptr::null_mut())?;
    }
    unsafe {
        ff::av_packet_free(&mut packet);
        ff::av_frame_free(&mut audio_frame);
    }
    if frames != spec.frames {
        problems.push(format!("{frames} video frames, expected {}", spec.frames));
    }
    if !late.is_empty() {
        problems.push(format!(
            "frames off their timestamps (frame, pts, expected): {late:?}"
        ));
    }
    // Audio.
    let audio = match (spec.audio, ast) {
        (None, None) => None,
        (None, Some(_)) => {
            problems.push("unexpected audio stream".into());
            None
        }
        (Some(_), None) => {
            problems.push("audio stream missing".into());
            None
        }
        (Some(a), Some(st)) => {
            let par = unsafe { &*(*st).codecpar };
            if par.codec_id != ff::AV_CODEC_ID_OPUS {
                problems.push("audio is not Opus".into());
            }
            if par.sample_rate as u32 != a.sample_rate
                || par.ch_layout.nb_channels != i32::from(a.channels)
            {
                problems.push(format!(
                    "audio {} Hz × {}, expected {} Hz × {}",
                    par.sample_rate, par.ch_layout.nb_channels, a.sample_rate, a.channels
                ));
            }
            // Chromium flushes enough silent input to cover Opus lookahead,
            // then the legacy muxer leaves the final frame's padding intact.
            // OpusHead still removes its pre-skip on decode.
            let pre_skip = if par.extradata_size >= 12 && !par.extradata.is_null() {
                unsafe {
                    u16::from_le_bytes([*par.extradata.add(10), *par.extradata.add(11)]) as u64
                }
            } else {
                0
            };
            if pre_skip != 312 || par.initial_padding != 0 {
                problems.push(format!(
                    "Opus pre-skip {pre_skip}, CodecDelay {}; expected 312 and 0",
                    par.initial_padding
                ));
            }
            let frame_samples = u64::from(a.sample_rate) / 50;
            let encoded_samples = audio_pushed
                .saturating_add(pre_skip)
                .div_ceil(frame_samples)
                .saturating_mul(frame_samples);
            let expected_samples = encoded_samples.saturating_sub(pre_skip);
            if audio_samples != expected_samples {
                problems.push(format!(
                    "{audio_samples} audio samples decoded, expected {expected_samples} for {audio_pushed} written"
                ));
            }
            Some(AudioReport {
                codec: "opus".into(),
                sample_rate: f64::from(par.sample_rate),
                channels: par.ch_layout.nb_channels as u32,
                bits_per_channel: 0,
                float: true,
                timescale: 1000,
                samples: audio_samples,
            })
        }
    };
    let scan_ms = scan_started.map_or(0.0, |started| started.elapsed().as_secs_f64() * 1e3);
    // Sampled pixels: native VP9 for opaque I420; libvpx retains alpha decoding.
    let mut sampled_pixels = SampledPixelTimes::default();
    let decode_started = profiling.then(Instant::now);
    let decoded = decode_sampled(
        fmt,
        vst,
        spec,
        alpha,
        profiling.then_some(&mut sampled_pixels),
    )?;
    let sampled_total = decode_started.map_or(Duration::ZERO, |started| started.elapsed());
    let decode_ms = sampled_total
        .saturating_sub(sampled_pixels.crop + sampled_pixels.means)
        .as_secs_f64()
        * 1e3;
    let pixel_check_started = profiling.then(Instant::now);
    let pixels: Vec<PixelCheck> =
        verify::check_pixel_blocks(spec.codec, blocks, decoded, &mut problems);
    let pixel_check_ms =
        pixel_check_started.map_or(0.0, |started| started.elapsed().as_secs_f64() * 1e3);
    if let Some(started) = verify_started {
        eprintln!(
            "[export timing] webm_verify_open_ms={open_ms:.3} webm_verify_scan_audio_ms={scan_ms:.3} webm_verify_sample_decode_ms={decode_ms:.3} webm_verify_crop_ms={:.3} webm_verify_means_ms={:.3} webm_verify_pixel_check_ms={pixel_check_ms:.3} webm_verify_total_ms={:.3}",
            sampled_pixels.crop.as_secs_f64() * 1e3,
            sampled_pixels.means.as_secs_f64() * 1e3,
            started.elapsed().as_secs_f64() * 1e3,
        );
    }
    if !problems.is_empty() {
        return Err(Error::Verify(problems.join("; ")));
    }
    let duration = unsafe { ff::av_rescale_q(spec.frames as i64, codec_tb, MS) } as u64;
    Ok(VerifyReport {
        codec: codec.into(),
        profile: spec.codec.name(),
        pixel_format: verify::expected_pixel_format(spec.codec, alpha).into(),
        depth: 8,
        cicp,
        full_range,
        alpha: alpha_tag,
        frames,
        movie_timescale: 1000,
        media_timescale: 1000,
        duration,
        audio,
        timecode: None,
        pixels,
        hardware_encoder: Some(false),
        bytes: 0,
    })
}

/// Decodes the sampled frames. Nearby samples share the decoder cursor; distant
/// samples seek to a keyframe so verification stays bounded for long exports.
fn decode_sampled(
    fmt: *mut ff::AVFormatContext,
    vst: *mut ff::AVStream,
    spec: &ExportSpec,
    alpha: bool,
    mut times: Option<&mut SampledPixelTimes>,
) -> Result<Vec<BlockSample>, Error> {
    let key_interval = (2.0 * spec.rate.to_f64()).round().max(1.0) as u64;
    // The opaque native VP9 decoder reproduces libvpx's complete planes and
    // sampled frame IDs and is faster in the sampled-verifier benchmark.
    // Alpha and VP8 retain their existing libvpx decoders.
    let decoder = match spec.codec {
        Codec::Vp9 { .. } if !alpha => Some("vp9"),
        _ => None,
    };
    let sampled = decode_sampled_with_gap_map(fmt, vst, spec, key_interval, decoder, |f, n| {
        frame_blocks(f, alpha, n, (spec.width, spec.height), times.as_deref_mut())
    })?;
    Ok(sampled.into_iter().flat_map(|(_, blocks)| blocks).collect())
}

fn decode_sampled_with_gap_map<T>(
    fmt: *mut ff::AVFormatContext,
    vst: *mut ff::AVStream,
    spec: &ExportSpec,
    max_sequential_gap: u64,
    decoder_name: Option<&str>,
    mut map: impl FnMut(&ff::AVFrame, u64) -> Result<T, Error>,
) -> Result<Vec<(u64, T)>, Error> {
    let (libvpx, native) = match spec.codec {
        Codec::Vp9 { .. } => ("libvpx-vp9", "vp9"),
        _ => ("libvpx", "vp8"),
    };
    // None selects the libvpx reference. Native vp8/vp9 are allowed only
    // for opaque video: libvpx is needed for WebM's alpha stream.
    let name = decoder_name.unwrap_or(libvpx);
    if name != libvpx && (spec.alpha != AlphaOutput::None || name != native) {
        return Err(Error::Verify(
            "native verifier trial requires matching opaque VP8/VP9 I420".into(),
        ));
    }
    let dec = open_decoder(vst, Some(name))?;
    let codec_tb = rational(spec.rate.den(), spec.rate.num());
    let (vtb, vindex) = unsafe { ((*vst).time_base, (*vst).index) };
    let mut packet = unsafe { ff::av_packet_alloc() };
    let mut frame = unsafe { ff::av_frame_alloc() };
    let mut out = vec![];
    let result = (|| -> Result<(), Error> {
        let mut previous = None;
        let mut eof = false;
        for n in verify::sampled_frames(spec.frames) {
            let target = unsafe { ff::av_rescale_q(n as i64, codec_tb, vtb) };
            if previous.is_none_or(|p: u64| n - p > max_sequential_gap) {
                unsafe {
                    check(
                        "seek",
                        ff::av_seek_frame(fmt, vindex, target, ff::AVSEEK_FLAG_BACKWARD as c_int),
                    )
                    .map_err(|e| Error::Verify(e.to_string()))?;
                    ff::avcodec_flush_buffers(dec.0);
                }
                eof = false;
            }
            let mut found = None;
            while found.is_none() {
                unsafe {
                    // A packet can yield more than one frame. Drain pending
                    // output before feeding the next packet, including after
                    // the previous sampled frame ended this loop early.
                    let r = ff::avcodec_receive_frame(dec.0, frame);
                    if r == 0 {
                        if (*frame).best_effort_timestamp == target {
                            found = Some(map(&*frame, n)?);
                        }
                        continue;
                    }
                    if r == ff::AVERROR_EOF || eof && r == -EAGAIN {
                        return Err(Error::Verify(format!("frame {n} is missing")));
                    }
                    if r != -EAGAIN {
                        check("decode", r).map_err(|e| Error::Verify(e.to_string()))?;
                    }
                    let sent = loop {
                        let read = ff::av_read_frame(fmt, packet);
                        if read < 0 {
                            if read != ff::AVERROR_EOF {
                                return Err(Error::Verify(format!(
                                    "read frame {n}: {}",
                                    pm_media::ff::err_str(read)
                                )));
                            }
                            eof = true;
                            break ff::avcodec_send_packet(dec.0, ptr::null_mut());
                        }
                        if (*packet).stream_index != vindex {
                            ff::av_packet_unref(packet);
                            continue;
                        }
                        let sent = ff::avcodec_send_packet(dec.0, packet);
                        ff::av_packet_unref(packet);
                        break sent;
                    };
                    if sent < 0 {
                        return Err(Error::Verify(format!(
                            "decode frame {n}: {}",
                            pm_media::ff::err_str(sent)
                        )));
                    }
                }
            }
            out.push((n, found.expect("set above")));
            previous = Some(n);
        }
        Ok(())
    })();
    unsafe {
        ff::av_packet_free(&mut packet);
        ff::av_frame_free(&mut frame);
    }
    result.map(|()| out)
}

#[cfg(test)]
mod sampled_decode_tests {
    use super::*;
    use crate::convert::WorkingFrame;
    use crate::settings::{ColorTags, Dither, WorkingEncoding};
    use pm_time::Rate;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "pm-webm-samples-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn with_video_stream<T>(
        path: &Path,
        run: impl FnOnce(*mut ff::AVFormatContext, *mut ff::AVStream) -> T,
    ) -> T {
        let p = cstr(&path.to_string_lossy());
        let input = unsafe {
            let mut fmt = ptr::null_mut();
            check(
                "open test WebM",
                ff::avformat_open_input(&mut fmt, p.as_ptr(), ptr::null(), ptr::null_mut()),
            )
            .unwrap();
            let input = Demuxer(fmt);
            check(
                "read test WebM",
                ff::avformat_find_stream_info(input.0, ptr::null_mut()),
            )
            .unwrap();
            input
        };
        let vst = unsafe {
            (0..(*input.0).nb_streams as usize)
                .map(|i| *(*input.0).streams.add(i))
                .find(|&st| (*(*st).codecpar).codec_type == ff::AVMEDIA_TYPE_VIDEO)
                .unwrap()
        };
        run(input.0, vst)
    }

    fn decoded_samples_with<T>(
        path: &Path,
        spec: &ExportSpec,
        max_sequential_gap: u64,
        decoder_name: Option<&str>,
        map: impl FnMut(&ff::AVFrame, u64) -> Result<T, Error>,
    ) -> Vec<(u64, T)> {
        with_video_stream(path, |fmt, vst| {
            decode_sampled_with_gap_map(fmt, vst, spec, max_sequential_gap, decoder_name, map)
                .unwrap()
        })
    }

    fn decoded_samples_named(
        path: &Path,
        spec: &ExportSpec,
        max_sequential_gap: u64,
        decoder_name: Option<&str>,
    ) -> Vec<(u64, EncoderInput)> {
        decoded_samples_with(path, spec, max_sequential_gap, decoder_name, |f, _| {
            frame_input(f, spec.alpha != AlphaOutput::None)
        })
    }

    fn decoded_samples(
        path: &Path,
        spec: &ExportSpec,
        max_sequential_gap: u64,
    ) -> Vec<(u64, EncoderInput)> {
        decoded_samples_named(path, spec, max_sequential_gap, None)
    }

    #[test]
    fn cursor_reuse_matches_seek_each_sample_for_vp8_vp9_and_alpha() {
        // At 30 fps the forced keyframe interval is 60 frames. The 60-frame
        // cases share a GOP, while the 100-frame cases cross the keyframe at 60.
        let scratch = Scratch::new();
        for (name, codec, alpha, frames) in [
            (
                "vp8",
                Codec::Vp8 {
                    bitrate_bps: 2_000_000,
                },
                false,
                60,
            ),
            (
                "vp9-alpha",
                Codec::Vp9 {
                    bitrate_bps: 2_000_000,
                },
                true,
                60,
            ),
            (
                "vp8-alpha",
                Codec::Vp8 {
                    bitrate_bps: 2_000_000,
                },
                true,
                100,
            ),
            (
                "vp9",
                Codec::Vp9 {
                    bitrate_bps: 2_000_000,
                },
                false,
                100,
            ),
        ] {
            let spec = ExportSpec {
                width: 64,
                height: 48,
                rate: Rate::FPS_30,
                frames,
                codec,
                container: Container::Webm,
                tags: if alpha {
                    ColorTags::Rec709Broadcast
                } else {
                    ColorTags::Rec709QuickTime
                },
                alpha: if alpha {
                    AlphaOutput::Straight
                } else {
                    AlphaOutput::None
                },
                working: WorkingEncoding::Standard,
                dither: Dither::Default,
                audio: None,
                timecode: None,
                destination: scratch.0.join(format!("{name}.webm")),
                overwrite: Overwrite::Fail,
                mode: None,
            };
            let mut sink =
                crate::Pipeline::new(spec.clone(), WebmSink::new(&spec).unwrap()).unwrap();
            for n in 0..frames {
                let mut rgba = Vec::with_capacity((spec.width * spec.height * 4) as usize);
                for y in 0..spec.height {
                    for x in 0..spec.width {
                        let a = if alpha {
                            ((x + y + n as u32) % 25) as f32 / 24.0
                        } else {
                            1.0
                        };
                        rgba.extend([
                            ((x + n as u32) % 32) as f32 / 31.0 * a,
                            ((y + n as u32) % 32) as f32 / 31.0 * a,
                            (x ^ y) as f32 / 63.0 * a,
                            a,
                        ]);
                    }
                }
                sink.push_frame(&WorkingFrame {
                    width: spec.width,
                    height: spec.height,
                    rgba: &rgba,
                })
                .unwrap();
            }
            let delivered = sink.finish().unwrap();
            let gap = (2.0 * spec.rate.to_f64()).round().max(1.0) as u64;
            let reused = decoded_samples(&delivered.path, &spec, gap);
            let sought = decoded_samples(&delivered.path, &spec, 0);
            assert_eq!(reused, sought, "{name}");
            if alpha {
                assert!(
                    reused.iter().all(|(_, input)| {
                        input.format == InputFormat::I420a && input.planes.len() == 4
                    }),
                    "alpha remains on libvpx's four-plane decoder: {name}"
                );
            } else {
                // Both explicit native decoders must reproduce every byte of
                // libvpx's Y, U and V planes, not merely pass block tolerances.
                let native_name = match codec {
                    Codec::Vp9 { .. } => "vp9",
                    Codec::Vp8 { .. } => "vp8",
                    _ => unreachable!(),
                };
                for decode_gap in [gap, 0] {
                    let native = decoded_samples_named(
                        &delivered.path,
                        &spec,
                        decode_gap,
                        Some(native_name),
                    );
                    assert_eq!(
                        native.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
                        verify::sampled_frames(frames),
                        "{name} native frame IDs with gap {decode_gap}"
                    );
                    assert_eq!(
                        native, reused,
                        "{name} native full YUV planes with gap {decode_gap}"
                    );
                }
            }
            let expected: Vec<_> = reused
                .iter()
                .flat_map(|(n, input)| verify::block_means(input, *n))
                .collect();
            assert_eq!(expected.len(), verify::sampled_frames(frames).len() * 9);
            for decode_gap in [gap, 0] {
                let cropped: Vec<_> =
                    decoded_samples_with(&delivered.path, &spec, decode_gap, None, |f, n| {
                        frame_blocks(f, alpha, n, (spec.width, spec.height), None)
                    })
                    .into_iter()
                    .flat_map(|(_, blocks)| blocks)
                    .collect();
                assert_eq!(cropped, expected, "{name} with gap {decode_gap}");
            }
            assert_eq!(
                reused.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
                verify::sampled_frames(frames),
                "{name}"
            );
        }
    }

    /// Manual comparison of the exact verifier decode path on a saved opaque
    /// 1920×1080, 60-frame, 30000/1001 VP9 export. The file is read only.
    #[test]
    #[ignore = "set PM_WEBM_VERIFY_BENCH_INPUT to the saved large VP9 WebM"]
    fn opaque_vp9_decoder_timing_on_saved_large_export() {
        let path = PathBuf::from(
            std::env::var_os("PM_WEBM_VERIFY_BENCH_INPUT")
                .expect("PM_WEBM_VERIFY_BENCH_INPUT must name a saved opaque VP9 WebM"),
        );
        let spec = ExportSpec {
            width: 1920,
            height: 1080,
            rate: Rate::FPS_29_97,
            frames: 60,
            codec: Codec::Vp9 {
                bitrate_bps: 16_000_000,
            },
            container: Container::Webm,
            tags: ColorTags::Rec709QuickTime,
            alpha: AlphaOutput::None,
            working: WorkingEncoding::Standard,
            dither: Dither::Default,
            audio: None,
            timecode: None,
            destination: path.clone(),
            overwrite: Overwrite::Fail,
            mode: None,
        };
        let gap = (2.0 * spec.rate.to_f64()).round().max(1.0) as u64;
        let libvpx = decoded_samples_named(&path, &spec, gap, Some("libvpx-vp9"));
        let native = decoded_samples_named(&path, &spec, gap, Some("vp9"));
        assert_eq!(
            libvpx.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            verify::sampled_frames(spec.frames)
        );
        assert_eq!(native, libvpx, "native and libvpx full YUV planes");
        let expected: Vec<_> = libvpx
            .iter()
            .flat_map(|(n, input)| verify::block_means(input, *n))
            .collect();
        for round in 0..6 {
            let names = if round % 2 == 0 {
                ["libvpx-vp9", "vp9"]
            } else {
                ["vp9", "libvpx-vp9"]
            };
            for name in names {
                let (elapsed, pixels, blocks) = with_video_stream(&path, |fmt, vst| {
                    let mut pixels = SampledPixelTimes::default();
                    let started = Instant::now();
                    let sampled = decode_sampled_with_gap_map(
                        fmt,
                        vst,
                        &spec,
                        gap,
                        Some(name),
                        |frame, n| {
                            frame_blocks(
                                frame,
                                false,
                                n,
                                (spec.width, spec.height),
                                Some(&mut pixels),
                            )
                        },
                    )
                    .unwrap();
                    (started.elapsed(), pixels, sampled)
                });
                let got: Vec<_> = blocks.into_iter().flat_map(|(_, v)| v).collect();
                assert_eq!(got, expected, "{name} verifier blocks on pass {round}");
                let decode = elapsed.saturating_sub(pixels.crop + pixels.means);
                eprintln!(
                    "webm verifier decoder={name} pass={round} decode_ms={:.3} total_ms={:.3}",
                    decode.as_secs_f64() * 1e3,
                    elapsed.as_secs_f64() * 1e3,
                );
            }
        }
    }
}
