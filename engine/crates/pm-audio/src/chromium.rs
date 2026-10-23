//! Today's decoder for the formats Chromium decodes with FFmpeg (AAC, Opus,
//! Vorbis): Electron's own `libffmpeg.dylib`, the library today's
//! `decodeAudioData` runs (Chromium 152's FFmpeg fork, its libopus), so the
//! samples are today's by construction.
//!
//! The library is already mapped into every Electron process (the Electron
//! Framework links it), including the engine's utility process. It is found
//! among the loaded images (or at `POWERMOVE_CHROMIUM_FFMPEG`), opened with
//! `RTLD_LOCAL`, and every function is resolved with `dlsym` from that
//! handle, so nothing binds to it by name and our LGPL FFmpeg's symbols
//! never clash with it. Its ABI majors equal ours (libavcodec and
//! libavformat 63, libavutil 61, checked at open), so our bindings' struct
//! layouts describe its objects.
//!
//! Chromium's rules (`media/filters/audio_file_reader.cc`, the FFmpeg audio
//! decoder): the first audio stream; packets of other streams skipped; float
//! output requested (`request_sample_fmt = FLT`); priming and end padding
//! dropped as the stream's skip side data gives them; decoded floats
//! clipped to [−1, 1] (NaN to 0); the decode runs from the file's start.
//! A block equals the sequential decode: the decoder moves forward from
//! where it is and starts again from the beginning only to go back, keeping
//! the blocks it passed.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use pm_media::sys::ffmpeg as ff;

use crate::error::{AudioError, Result};
use crate::source::BLOCK;

fn err(what: &str) -> AudioError {
    AudioError::Media(pm_media::MediaError::decode(format!(
        "today's decoder: {what}"
    )))
}

type AllocFormat = unsafe extern "C" fn() -> *mut ff::AVFormatContext;
type Malloc = unsafe extern "C" fn(usize) -> *mut c_void;
type ReadFn = unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int;
type SeekFn = unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64;
type AllocIo = unsafe extern "C" fn(
    *mut u8,
    c_int,
    c_int,
    *mut c_void,
    Option<ReadFn>,
    *const c_void,
    Option<SeekFn>,
) -> *mut ff::AVIOContext;
type OpenInput = unsafe extern "C" fn(
    *mut *mut ff::AVFormatContext,
    *const c_char,
    *const c_void,
    *mut *mut c_void,
) -> c_int;
type FindInfo = unsafe extern "C" fn(*mut ff::AVFormatContext, *mut *mut c_void) -> c_int;
type FindDecoder = unsafe extern "C" fn(ff::AVCodecID) -> *const ff::AVCodec;
type AllocCodec = unsafe extern "C" fn(*const ff::AVCodec) -> *mut ff::AVCodecContext;
type ParamsTo =
    unsafe extern "C" fn(*mut ff::AVCodecContext, *const ff::AVCodecParameters) -> c_int;
type Open2 =
    unsafe extern "C" fn(*mut ff::AVCodecContext, *const ff::AVCodec, *mut *mut c_void) -> c_int;
type AllocPacket = unsafe extern "C" fn() -> *mut ff::AVPacket;
type AllocFrame = unsafe extern "C" fn() -> *mut ff::AVFrame;
type ReadFrame = unsafe extern "C" fn(*mut ff::AVFormatContext, *mut ff::AVPacket) -> c_int;
type SendPacket = unsafe extern "C" fn(*mut ff::AVCodecContext, *const ff::AVPacket) -> c_int;
type ReceiveFrame = unsafe extern "C" fn(*mut ff::AVCodecContext, *mut ff::AVFrame) -> c_int;
type UnrefPacket = unsafe extern "C" fn(*mut ff::AVPacket);
type CloseInput = unsafe extern "C" fn(*mut *mut ff::AVFormatContext);
type FreeCodec = unsafe extern "C" fn(*mut *mut ff::AVCodecContext);
type FreeFrame = unsafe extern "C" fn(*mut *mut ff::AVFrame);
type FreePacket = unsafe extern "C" fn(*mut *mut ff::AVPacket);
type FreeIo = unsafe extern "C" fn(*mut *mut ff::AVIOContext);
type Free = unsafe extern "C" fn(*mut c_void);
type Version = unsafe extern "C" fn() -> u32;

struct Lib {
    path: PathBuf,
    alloc_format: AllocFormat,
    malloc: Malloc,
    alloc_io: AllocIo,
    open_input: OpenInput,
    find_info: FindInfo,
    find_decoder: FindDecoder,
    alloc_codec: AllocCodec,
    params_to: ParamsTo,
    open2: Open2,
    alloc_packet: AllocPacket,
    alloc_frame: AllocFrame,
    read_frame: ReadFrame,
    send_packet: SendPacket,
    receive_frame: ReceiveFrame,
    unref_packet: UnrefPacket,
    close_input: CloseInput,
    free_codec: FreeCodec,
    free_frame: FreeFrame,
    free_packet: FreePacket,
    free_io: FreeIo,
    free: Free,
}

// SAFETY: function pointers into a library that stays loaded.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

/// `libffmpeg.dylib` of the Electron running this process, or the file
/// `POWERMOVE_CHROMIUM_FFMPEG` names.
fn locate() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("POWERMOVE_CHROMIUM_FFMPEG") {
        return Some(PathBuf::from(p));
    }
    unsafe extern "C" {
        fn _dyld_image_count() -> u32;
        fn _dyld_get_image_name(index: u32) -> *const c_char;
    }
    // SAFETY: dyld's image list; names are NUL-terminated paths it owns.
    unsafe {
        for i in 0.._dyld_image_count() {
            let name = _dyld_get_image_name(i);
            if name.is_null() {
                continue;
            }
            let path = CStr::from_ptr(name).to_string_lossy();
            if path.ends_with("/Libraries/libffmpeg.dylib") {
                return Some(PathBuf::from(path.into_owned()));
            }
        }
    }
    None
}

fn load() -> Option<Lib> {
    let path = locate()?;
    let c = CString::new(path.to_string_lossy().as_bytes()).ok()?;
    // SAFETY: dlopen/dlsym with a valid path; the handle is never closed.
    unsafe {
        let handle = libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            return None;
        }
        macro_rules! sym {
            ($name:literal) => {{
                let p = libc::dlsym(handle, concat!($name, "\0").as_ptr().cast());
                if p.is_null() {
                    return None;
                }
                #[allow(clippy::missing_transmute_annotations)]
                std::mem::transmute::<*mut c_void, _>(p)
            }};
        }
        let codec_version: Version = sym!("avcodec_version");
        let format_version: Version = sym!("avformat_version");
        let util_version: Version = sym!("avutil_version");
        // The same ABI majors as our bindings, or the layouts differ.
        if codec_version() >> 16 != ff::LIBAVCODEC_VERSION_MAJOR
            || format_version() >> 16 != ff::LIBAVFORMAT_VERSION_MAJOR
            || util_version() >> 16 != ff::LIBAVUTIL_VERSION_MAJOR
        {
            return None;
        }
        Some(Lib {
            path,
            alloc_format: sym!("avformat_alloc_context"),
            malloc: sym!("av_malloc"),
            alloc_io: sym!("avio_alloc_context"),
            open_input: sym!("avformat_open_input"),
            find_info: sym!("avformat_find_stream_info"),
            find_decoder: sym!("avcodec_find_decoder"),
            alloc_codec: sym!("avcodec_alloc_context3"),
            params_to: sym!("avcodec_parameters_to_context"),
            open2: sym!("avcodec_open2"),
            alloc_packet: sym!("av_packet_alloc"),
            alloc_frame: sym!("av_frame_alloc"),
            read_frame: sym!("av_read_frame"),
            send_packet: sym!("avcodec_send_packet"),
            receive_frame: sym!("avcodec_receive_frame"),
            unref_packet: sym!("av_packet_unref"),
            close_input: sym!("avformat_close_input"),
            free_codec: sym!("avcodec_free_context"),
            free_frame: sym!("av_frame_free"),
            free_packet: sym!("av_packet_free"),
            free_io: sym!("avio_context_free"),
            free: sym!("av_free"),
        })
    }
}

fn lib() -> Option<&'static Lib> {
    static LIB: OnceLock<Option<Lib>> = OnceLock::new();
    LIB.get_or_init(load).as_ref()
}

/// Whether today's decoder is loaded in this process, and from where.
pub fn library() -> Option<&'static Path> {
    lib().map(|l| l.path.as_path())
}

/// Whether today decodes `codec` (FFmpeg's name) with FFmpeg: everything
/// but what Chromium hands to Symphonia (MP3, FLAC) or decodes as PCM.
pub fn decodes(codec: &str) -> bool {
    matches!(codec, "aac" | "opus" | "vorbis")
}

/// The file's bytes, read through a custom I/O context as Chromium's
/// `InMemoryUrlProtocol` serves them.
struct Memory {
    data: Vec<u8>,
    pos: usize,
}

unsafe extern "C" fn read_mem(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    // SAFETY: `opaque` is the decoder's `Memory`, alive while it is open.
    let m = unsafe { &mut *opaque.cast::<Memory>() };
    let left = m.data.len().saturating_sub(m.pos);
    if left == 0 {
        return ff::AVERROR_EOF;
    }
    let n = left.min(size.max(0) as usize);
    // SAFETY: FFmpeg's buffer holds `size` bytes.
    unsafe { std::ptr::copy_nonoverlapping(m.data.as_ptr().add(m.pos), buf, n) };
    m.pos += n;
    n as c_int
}

unsafe extern "C" fn seek_mem(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    // SAFETY: as in `read_mem`.
    let m = unsafe { &mut *opaque.cast::<Memory>() };
    let whence = whence & !(ff::AVSEEK_FORCE as c_int);
    let to = match whence {
        w if w == ff::AVSEEK_SIZE as c_int => return m.data.len() as i64,
        0 => offset,
        1 => m.pos as i64 + offset,
        2 => m.data.len() as i64 + offset,
        _ => return -1,
    };
    if to < 0 {
        return -1;
    }
    m.pos = (to as usize).min(m.data.len());
    m.pos as i64
}

/// One open decode of a file through today's library.
struct Session {
    lib: &'static Lib,
    format: *mut ff::AVFormatContext,
    io: *mut ff::AVIOContext,
    codec: *mut ff::AVCodecContext,
    packet: *mut ff::AVPacket,
    frame: *mut ff::AVFrame,
    stream: c_int,
    _memory: Box<Memory>,
    flushed: bool,
    done: bool,
}

// SAFETY: a session is used by one thread at a time (behind the source's
// mutex) and owns every FFmpeg object it points to.
unsafe impl Send for Session {}

impl Session {
    fn open(lib: &'static Lib, data: Vec<u8>) -> Result<Session> {
        let mut memory = Box::new(Memory { data, pos: 0 });
        // SAFETY: FFmpeg calls through today's library; every pointer is
        // checked and freed in `Drop`.
        unsafe {
            let mut s = Session {
                lib,
                format: (lib.alloc_format)(),
                io: std::ptr::null_mut(),
                codec: std::ptr::null_mut(),
                packet: (lib.alloc_packet)(),
                frame: (lib.alloc_frame)(),
                stream: -1,
                _memory: Box::new(Memory {
                    data: vec![],
                    pos: 0,
                }),
                flushed: false,
                done: false,
            };
            let buf = (lib.malloc)(32768).cast::<u8>();
            s.io = (lib.alloc_io)(
                buf,
                32768,
                0,
                (&mut *memory as *mut Memory).cast(),
                Some(read_mem),
                std::ptr::null(),
                Some(seek_mem),
            );
            s._memory = memory;
            if s.format.is_null() || s.io.is_null() || s.packet.is_null() || s.frame.is_null() {
                return Err(err("out of memory"));
            }
            (*s.format).pb = s.io;
            (*s.format).flags |= ff::AVFMT_FLAG_CUSTOM_IO as c_int;
            if (lib.open_input)(
                &mut s.format,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            ) < 0
            {
                s.format = std::ptr::null_mut();
                return Err(err("cannot open the file"));
            }
            if (lib.find_info)(s.format, std::ptr::null_mut()) < 0 {
                return Err(err("cannot read the stream info"));
            }
            // The first audio stream, as `AudioFileReader::OpenDemuxer`.
            let streams =
                std::slice::from_raw_parts((*s.format).streams, (*s.format).nb_streams as usize);
            let Some(index) = streams
                .iter()
                .position(|st| (*(**st).codecpar).codec_type == ff::AVMEDIA_TYPE_AUDIO)
            else {
                return Err(err("no audio stream"));
            };
            s.stream = index as c_int;
            let par = (*streams[index]).codecpar;
            let decoder = (lib.find_decoder)((*par).codec_id);
            if decoder.is_null() {
                return Err(err("no decoder for the stream"));
            }
            s.codec = (lib.alloc_codec)(decoder);
            if s.codec.is_null() || (lib.params_to)(s.codec, par) < 0 {
                return Err(err("cannot configure the decoder"));
            }
            (*s.codec).request_sample_fmt = ff::AV_SAMPLE_FMT_FLT;
            if (lib.open2)(s.codec, decoder, std::ptr::null_mut()) < 0 {
                return Err(err("cannot open the decoder"));
            }
            Ok(s)
        }
    }

    /// Appends the next decoded frame to `out` (planar, clipped). `false`
    /// at the end of the stream.
    fn next(&mut self, out: &mut [Vec<f32>]) -> Result<bool> {
        let lib = self.lib;
        // SAFETY: the session's objects are open; frame data are read
        // within `nb_samples` × channels of their declared format.
        unsafe {
            loop {
                if self.done {
                    return Ok(false);
                }
                let r = (lib.receive_frame)(self.codec, self.frame);
                if r >= 0 {
                    let f = &*self.frame;
                    let n = f.nb_samples.max(0) as usize;
                    let ch = f.ch_layout.nb_channels.max(1) as usize;
                    let clip = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) };
                    for (c, dst) in out.iter_mut().enumerate() {
                        let k = c.min(ch - 1);
                        match f.format {
                            x if x == ff::AV_SAMPLE_FMT_FLT => {
                                let p = *f.extended_data.cast::<*const f32>();
                                dst.extend((0..n).map(|i| clip(*p.add(i * ch + k))));
                            }
                            x if x == ff::AV_SAMPLE_FMT_FLTP => {
                                let p = *f.extended_data.cast::<*const f32>().add(k);
                                dst.extend((0..n).map(|i| clip(*p.add(i))));
                            }
                            x if x == ff::AV_SAMPLE_FMT_S16 => {
                                let p = *f.extended_data.cast::<*const i16>();
                                dst.extend((0..n).map(|i| s16(*p.add(i * ch + k))));
                            }
                            x if x == ff::AV_SAMPLE_FMT_S16P => {
                                let p = *f.extended_data.cast::<*const i16>().add(k);
                                dst.extend((0..n).map(|i| s16(*p.add(i))));
                            }
                            _ => return Err(err("an unexpected sample format")),
                        }
                    }
                    if n > 0 {
                        return Ok(true);
                    }
                    continue;
                }
                if r == ff::AVERROR_EOF {
                    self.done = true;
                    return Ok(false);
                }
                if self.flushed {
                    self.done = true;
                    return Ok(false);
                }
                // Needs input: the next packet of the stream, or the flush.
                loop {
                    let r = (lib.read_frame)(self.format, self.packet);
                    if r < 0 {
                        (lib.send_packet)(self.codec, std::ptr::null());
                        self.flushed = true;
                        break;
                    }
                    if (*self.packet).stream_index != self.stream {
                        (lib.unref_packet)(self.packet);
                        continue;
                    }
                    // A damaged packet is dropped, as the reader continues.
                    let _ = (lib.send_packet)(self.codec, self.packet);
                    (lib.unref_packet)(self.packet);
                    break;
                }
            }
        }
    }
}

/// Chromium's 16-bit to float conversion (`FixedSampleTypeTraits`).
fn s16(v: i16) -> f32 {
    if v < 0 {
        f32::from(v) * (1.0 / 32768.0)
    } else {
        f32::from(v) * (1.0 / 32767.0)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let lib = self.lib;
        // SAFETY: each object is freed once by the library that made it.
        unsafe {
            (lib.free_frame)(&mut self.frame);
            (lib.free_packet)(&mut self.packet);
            (lib.free_codec)(&mut self.codec);
            if !self.format.is_null() {
                (lib.close_input)(&mut self.format);
            }
            if !self.io.is_null() {
                (lib.free)((*self.io).buffer.cast());
                (lib.free_io)(&mut self.io);
            }
        }
    }
}

/// Blocks passed on the way forward, kept for a request just behind.
const KEPT: usize = 8;

/// A source decoded by today's library.
pub struct ChromiumDecoder {
    path: PathBuf,
    channels: usize,
    session: Option<Session>,
    /// Program sample of the next decoded frame.
    position: u64,
    pending: Vec<Vec<f32>>,
    offset: usize,
    /// Whole blocks decoded while moving forward, by index.
    kept: BTreeMap<u64, Vec<Vec<f32>>>,
    /// The block being read from `kept`, and how far.
    serving: Option<(Vec<Vec<f32>>, usize)>,
}

impl ChromiumDecoder {
    /// `None` when today's library is not loaded in this process.
    pub fn open(path: &Path, channels: usize) -> Result<Option<ChromiumDecoder>> {
        if lib().is_none() {
            return Ok(None);
        }
        let mut d = ChromiumDecoder {
            path: path.to_owned(),
            channels,
            session: None,
            position: 0,
            pending: vec![Vec::new(); channels],
            offset: 0,
            kept: BTreeMap::new(),
            serving: None,
        };
        d.restart()?;
        Ok(Some(d))
    }

    /// The decoded length in frames: today's buffer length, which sets the
    /// clip's audible end (`asset.audioBuffer.duration`). A whole decode,
    /// once per open source; the blocks it passes are kept.
    pub fn count_frames(&mut self) -> Result<u64> {
        self.restart()?;
        let mut total = 0u64;
        loop {
            let start = self.position;
            let mut block = vec![Vec::with_capacity(BLOCK); self.channels];
            let got = self.read_pending(&mut block, BLOCK)? as u64;
            total += got;
            if got == BLOCK as u64 {
                self.keep(start / BLOCK as u64, block);
            } else {
                break;
            }
        }
        self.restart()?;
        Ok(total)
    }

    fn restart(&mut self) -> Result<()> {
        let lib = lib().ok_or_else(|| err("not loaded"))?;
        let data = std::fs::read(&self.path).map_err(|e| err(&format!("read: {e}")))?;
        self.session = None;
        self.session = Some(Session::open(lib, data)?);
        self.position = 0;
        for c in &mut self.pending {
            c.clear();
        }
        self.offset = 0;
        Ok(())
    }

    fn available(&self) -> usize {
        self.pending.first().map_or(0, |c| c.len()) - self.offset
    }

    fn fill(&mut self) -> Result<bool> {
        for c in self.pending.iter_mut() {
            c.drain(..self.offset.min(c.len()));
        }
        self.offset = 0;
        let session = self.session.as_mut().ok_or_else(|| err("not open"))?;
        session.next(&mut self.pending)
    }

    /// Positions the decode at program sample `index`: the samples equal
    /// the sequential decode from the file's start.
    pub fn seek_anchored(&mut self, index: u64) -> Result<()> {
        self.serving = None;
        if index.is_multiple_of(BLOCK as u64)
            && let Some(block) = self.kept.get(&(index / BLOCK as u64))
        {
            self.serving = Some((block.clone(), 0));
            return Ok(());
        }
        if index < self.position {
            self.restart()?;
        }
        while self.position < index {
            if self.available() == 0 && !self.fill()? {
                return Ok(());
            }
            // Keep whole blocks passed on the way.
            if self.position.is_multiple_of(BLOCK as u64) && index - self.position >= BLOCK as u64 {
                let mut block = vec![Vec::with_capacity(BLOCK); self.channels];
                let got = self.read_pending(&mut block, BLOCK)?;
                if got == BLOCK {
                    self.keep((self.position - BLOCK as u64) / BLOCK as u64, block);
                }
                continue;
            }
            let take = (index - self.position).min(self.available() as u64) as usize;
            self.offset += take;
            self.position += take as u64;
        }
        Ok(())
    }

    fn keep(&mut self, index: u64, block: Vec<Vec<f32>>) {
        self.kept.insert(index, block);
        while self.kept.len() > KEPT {
            self.kept.pop_first();
        }
    }

    fn read_pending(&mut self, out: &mut [Vec<f32>], frames: usize) -> Result<usize> {
        let mut done = 0;
        while done < frames {
            if self.available() == 0 && !self.fill()? {
                break;
            }
            let take = (frames - done).min(self.available());
            for (dst, src) in out.iter_mut().zip(&self.pending) {
                dst.extend_from_slice(&src[self.offset..self.offset + take]);
            }
            self.offset += take;
            self.position += take as u64;
            done += take;
        }
        Ok(done)
    }

    /// Up to `frames` samples per channel appended to `out`.
    pub fn read(&mut self, out: &mut [Vec<f32>], frames: usize) -> Result<usize> {
        if let Some((block, at)) = &mut self.serving {
            let n = frames.min(block[0].len() - *at);
            for (dst, src) in out.iter_mut().zip(block.iter()) {
                dst.extend_from_slice(&src[*at..*at + n]);
            }
            *at += n;
            return Ok(n);
        }
        let start = self.position;
        let first = out.first().map_or(0, |c| c.len());
        let got = self.read_pending(out, frames)?;
        if got == BLOCK && start.is_multiple_of(BLOCK as u64) {
            let block: Vec<Vec<f32>> = out.iter().map(|c| c[first..].to_vec()).collect();
            self.keep(start / BLOCK as u64, block);
        }
        Ok(got)
    }
}
