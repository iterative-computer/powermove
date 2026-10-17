//! WebM through FFmpeg (plan section 17, "Muxers" and "Formats at cutover"): VP9
//! or VP8 from libvpx, alpha as libvpx's second stream (Matroska `AlphaMode` and
//! `BlockAdditional`), Opus from libopus at 48 kHz, muxed by FFmpeg's WebM muxer.
//!
//! Frame-exact: every frame of the range is encoded, frame n at presentation
//! time n / rate (Matroska stores milliseconds; the stream also declares its
//! frame duration), unlike today's "WebM · realtime capture", which this
//! replaces. Colour elements carry the output tags (section 9) with siting type 0
//! (`ChromaSitingHorz` left, vertical half). Keyframes come at least every two
//! seconds; rate control is today's quality tiers as an average bit rate.
//!
//! Verification demuxes the whole file (frame count, every frame's timestamp,
//! codec, size, tags, range, alpha mode), decodes the first, middle, and last
//! frames with libvpx (which decodes the alpha stream) to compare block means
//! with the encoder input, and decodes the audio to count its samples.

use std::ffi::{CStr, CString, c_int};
use std::path::Path;
use std::ptr;

use pm_media::sys::ffmpeg as ff;

use crate::Delivered;
use crate::Error;
use crate::atomic::AtomicOutput;
use crate::convert::{EncoderInput, Plane, PlaneData};
use crate::pipeline::Sink;
use crate::settings::{
    AlphaOutput, AudioCodec, Codec, ColorTags, Container, ExportSpec, InputFormat, Overwrite,
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
    header_written: bool,
    trailer_written: bool,
    /// Taken by the commit; dropped (removing the temporary file) otherwise.
    output: Option<AtomicOutput>,
    spec: ExportSpec,
    sampled: Vec<u64>,
    blocks: Vec<BlockSample>,
}

// The FFmpeg contexts are used from one thread at a time.
unsafe impl Send for WebmSink {}

fn color(tags: ColorTags) -> (ff::AVColorPrimaries, ff::AVColorTransferCharacteristic) {
    match tags {
        ColorTags::Rec709QuickTime | ColorTags::SrgbImage => {
            (ff::AVCOL_PRI_BT709, ff::AVCOL_TRC_IEC61966_2_1)
        }
        ColorTags::Rec709Broadcast | ColorTags::Rec709Image => {
            (ff::AVCOL_PRI_BT709, ff::AVCOL_TRC_BT709)
        }
    }
}

/// Encoder threads: libvpx scales to about one thread per tile column.
fn threads(width: u32) -> i64 {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get()) as i64;
    let tiles = (width as i64 / 256).max(1);
    tiles.min(cores).clamp(1, 16)
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
            header_written: false,
            trailer_written: false,
            output: Some(output),
            spec: spec.clone(),
            sampled: verify::sampled_frames(spec.frames),
            blocks: vec![],
        };
        this.open()?;
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
            c.time_base = rational(spec.rate.den(), spec.rate.num());
            c.framerate = rational(spec.rate.num(), spec.rate.den());
            c.bit_rate = spec.codec.bitrate_bps().unwrap_or(16_000_000) as i64;
            c.gop_size = (2.0 * spec.rate.to_f64()).round().max(1.0) as c_int;
            let (primaries, transfer) = color(spec.tags);
            c.color_primaries = primaries;
            c.color_trc = transfer;
            c.colorspace = ff::AVCOL_SPC_BT709;
            c.color_range = ff::AVCOL_RANGE_MPEG;
            c.chroma_sample_location = ff::AVCHROMA_LOC_LEFT;
            c.thread_count = threads(spec.width) as c_int;
            if (*(*self.fmt).oformat).flags & ff::AVFMT_GLOBALHEADER as c_int != 0 {
                c.flags |= ff::AV_CODEC_FLAG_GLOBAL_HEADER as c_int;
            }
            let mut opts = vec![
                ("deadline", "good".to_string()),
                ("cpu-used", "2".to_string()),
                // Alternate reference frames do not combine with libvpx's alpha.
                ("auto-alt-ref", if alpha { "0" } else { "1" }.to_string()),
            ];
            if matches!(spec.codec, Codec::Vp9 { .. }) {
                opts.push(("row-mt", "1".into()));
                let log2_tiles = ((spec.width as f64 / 256.0).log2().floor() as i64).clamp(0, 6);
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
                ac.bit_rate = 96_000 * i64::from(a.channels);
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
                let mut dict = Dict::new(&[("application", "audio".into())]);
                check("open libopus", ff::avcodec_open2(actx, codec, &mut dict.0))?;
                let st = ff::avformat_new_stream(self.fmt, ptr::null());
                if st.is_null() {
                    return Err(Error::Mux("could not add the audio stream".into()));
                }
                check(
                    "audio stream parameters",
                    ff::avcodec_parameters_from_context((*st).codecpar, actx),
                )?;
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
            f.color_primaries = primaries;
            f.color_trc = transfer;
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
            f.pts = index as i64;
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
        self.write_planes(input, index)?;
        // Keep audio interleaved with video.
        self.encode_audio_frames(false)
    }

    fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        if self.audio.is_none() {
            return Err(Error::InvalidFrame("this export has no audio track".into()));
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
    let dims = [(w, h), (w / 2, h / 2), (w / 2, h / 2), (w, h)];
    let mut planes = vec![];
    for (k, &(pw, ph)) in dims.iter().enumerate().take(if alpha { 4 } else { 3 }) {
        let stride = f.linesize[k] as usize;
        let mut v = Vec::with_capacity(pw * ph);
        for y in 0..ph {
            v.extend_from_slice(unsafe {
                std::slice::from_raw_parts(f.data[k].add(y * stride), pw)
            });
        }
        planes.push(Plane {
            data: PlaneData::U8(v),
            row_samples: pw,
            rows: ph,
        });
    }
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
    pm_media::ff::init();
    let mut problems: Vec<String> = vec![];
    let alpha = spec.alpha != AlphaOutput::None;
    let p = cstr(&path.to_string_lossy());
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
    let (primaries, transfer) = color(spec.tags);
    let cicp = (
        vpar.color_primaries as u16,
        vpar.color_trc as u16,
        vpar.color_space as u16,
    );
    if cicp != spec.tags.cicp() || (vpar.color_primaries, vpar.color_trc) != (primaries, transfer) {
        problems.push(format!(
            "colour {}-{}-{}, expected {:?}",
            cicp.0,
            cicp.1,
            cicp.2,
            spec.tags.cicp()
        ));
    }
    let full_range = vpar.color_range == ff::AVCOL_RANGE_JPEG;
    if vpar.color_range != ff::AVCOL_RANGE_MPEG {
        problems.push(format!("range {}, expected limited", vpar.color_range));
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
            // Codec delay and end padding are trimmed by the decoder.
            if audio_samples != audio_pushed {
                problems.push(format!(
                    "{audio_samples} audio samples decoded, {audio_pushed} written"
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
    // Sampled pixels, decoded by libvpx (which also decodes the alpha stream).
    let decoded = decode_sampled(fmt, vst, spec, alpha)?;
    let pixels: Vec<PixelCheck> = verify::check_pixels(spec.codec, blocks, &decoded, &mut problems);
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

/// Decodes the sampled frames: seek to the keyframe before each, decode up to it.
fn decode_sampled(
    fmt: *mut ff::AVFormatContext,
    vst: *mut ff::AVStream,
    spec: &ExportSpec,
    alpha: bool,
) -> Result<Vec<(u64, EncoderInput)>, Error> {
    let name = match spec.codec {
        Codec::Vp9 { .. } => "libvpx-vp9",
        _ => "libvpx",
    };
    let dec = open_decoder(vst, Some(name))?;
    let codec_tb = rational(spec.rate.den(), spec.rate.num());
    let (vtb, vindex) = unsafe { ((*vst).time_base, (*vst).index) };
    let mut packet = unsafe { ff::av_packet_alloc() };
    let mut frame = unsafe { ff::av_frame_alloc() };
    let mut out = vec![];
    let result = (|| -> Result<(), Error> {
        for n in verify::sampled_frames(spec.frames) {
            let target = unsafe { ff::av_rescale_q(n as i64, codec_tb, vtb) };
            unsafe {
                check(
                    "seek",
                    ff::av_seek_frame(fmt, vindex, target, ff::AVSEEK_FLAG_BACKWARD as c_int),
                )
                .map_err(|e| Error::Verify(e.to_string()))?;
                ff::avcodec_flush_buffers(dec.0);
            }
            let mut found = None;
            let mut eof = false;
            while found.is_none() {
                unsafe {
                    let sent = if eof {
                        ff::avcodec_send_packet(dec.0, ptr::null_mut())
                    } else if ff::av_read_frame(fmt, packet) >= 0 {
                        if (*packet).stream_index != vindex {
                            ff::av_packet_unref(packet);
                            continue;
                        }
                        let r = ff::avcodec_send_packet(dec.0, packet);
                        ff::av_packet_unref(packet);
                        r
                    } else {
                        eof = true;
                        ff::avcodec_send_packet(dec.0, ptr::null_mut())
                    };
                    if sent < 0 && sent != ff::AVERROR_EOF {
                        return Err(Error::Verify(format!(
                            "decode frame {n}: {}",
                            pm_media::ff::err_str(sent)
                        )));
                    }
                    loop {
                        let r = ff::avcodec_receive_frame(dec.0, frame);
                        if r == -EAGAIN {
                            break;
                        }
                        if r == ff::AVERROR_EOF {
                            return Err(Error::Verify(format!("frame {n} is missing")));
                        }
                        check("decode", r).map_err(|e| Error::Verify(e.to_string()))?;
                        let pts = (*frame).best_effort_timestamp;
                        if pts == target {
                            found = Some(frame_input(&*frame, alpha)?);
                            break;
                        }
                    }
                }
            }
            out.push((n, found.expect("set above")));
        }
        Ok(())
    })();
    unsafe {
        ff::av_packet_free(&mut packet);
        ff::av_frame_free(&mut frame);
    }
    result.map(|()| out)
}
