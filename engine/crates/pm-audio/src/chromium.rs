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
//!
//! Random access as today has it: `decodeAudioData` decodes the whole file
//! once and playback reads the buffer. Here one sequential decode per file
//! ([`Whole`]) runs on its own thread from the first sample read, reading the file
//! through the I/O context (never holding its bytes), and every open of the
//! file reads the decoded samples it shares: a seek anywhere, forward or
//! back, copies samples once the decode has passed them. The samples equal
//! the sequential decode by construction (AAC's SBR noise index and Opus's
//! energy prediction carry state across the whole stream, so a packet seek
//! with pre-roll would not). Samples are spooled to an anonymous temporary
//! file, not retained in RAM. SourceCache owns the budgeted sample-block LRU.
//! Opening metadata is lazy; the spool disappears with the last reader.

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

type FileKey = (PathBuf, u64, Option<std::time::SystemTime>);

fn file_key(path: &Path) -> Option<FileKey> {
    let meta = std::fs::metadata(path).ok()?;
    Some((path.to_owned(), meta.len(), meta.modified().ok()))
}

/// Lengths of the files whose whole decode has ended.
type Lengths = std::sync::Mutex<BTreeMap<FileKey, u64>>;

fn lengths() -> &'static Lengths {
    static LENGTHS: OnceLock<Lengths> = OnceLock::new();
    LENGTHS.get_or_init(Default::default)
}

/// Today's decoded length of a file, when it has been counted (per file:
/// path, size, modification time).
pub fn known_length(path: &Path) -> Option<u64> {
    let key = file_key(path)?;
    lengths()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .copied()
}

/// Today's decoded length (`decodeAudioData` decodes the whole file): the
/// file's shared decode gives it when it ends. `blocking` waits for it (an
/// export's plan); otherwise `None` until it is known, so no mix build or
/// playback control waits on it.
pub fn count_length(path: &Path, channels: usize, blocking: bool) -> Option<u64> {
    if let Some(n) = known_length(path) {
        return Some(n);
    }
    if !blocking {
        return None;
    }
    ChromiumDecoder::open(path, channels)
        .ok()
        .flatten()
        .and_then(|mut d| d.count_frames().ok())
}

/// A notice for a mix whose voices use these codecs when today's decoder is
/// not in this process (the CLI, a test outside Electron): they decode with
/// our FFmpeg, about −93 dBFS from the app's samples.
pub fn fallback_notice<'a>(codecs: impl IntoIterator<Item = &'a str>) -> Option<String> {
    if lib().is_some() {
        return None;
    }
    let mut used: Vec<&str> = codecs.into_iter().filter(|c| decodes(c)).collect();
    used.sort_unstable();
    used.dedup();
    (!used.is_empty()).then(|| {
        format!(
            "audio: {} decoded with Powermove's FFmpeg, not the app's (Electron's libffmpeg is not              loaded in this process; set POWERMOVE_CHROMIUM_FFMPEG to its path): samples differ              from the app's export by about -93 dBFS",
            used.join(", ")
        )
    })
}

/// Today's decoded-audio budget (`PM.Memory`'s `audio`, 128 MiB): a file
/// whose `decodeAudioData` buffer is larger is refused ("too long to decode
/// safely") and plays nothing today.
pub const TODAYS_BUDGET: u64 = 128 << 20;

/// Whether today plays this stream: its decoded buffer (frames at the
/// context's rate × channels × 4 bytes) fits [`TODAYS_BUDGET`]. The context
/// runs at the device's rate, unknown here, so the estimate takes the lower
/// of the file's rate and 44.1 kHz: a file today plays is never taken for
/// one it refuses (a refused file near the line may still use the exact
/// sequential decoder). The sequential PCM is spooled to disk, so both
/// paths keep resident samples in SourceCache's bounded block LRU. Longer
/// files use Powermove's random-access decoder; the budget never refuses a
/// stream that the engine can play (LONG-1).
pub fn plays_today(info: &pm_media::audio::AudioStreamInfo) -> bool {
    let Some(frames) = info.length else {
        // No container length: exact sequential PCM goes to the disk spool.
        return true;
    };
    let rate = f64::from(info.sample_rate.max(1));
    let context_frames = frames as f64 * (44_100f64.min(rate) / rate);
    context_frames * info.channels().max(1) as f64 * 4.0 <= TODAYS_BUDGET as f64
}

/// Whether today decodes `codec` (FFmpeg's name) with FFmpeg: everything
/// but what Chromium hands to Symphonia (MP3, FLAC) or decodes as PCM.
pub fn decodes(codec: &str) -> bool {
    matches!(codec, "aac" | "opus" | "vorbis")
}

/// The file, read through a custom I/O context as Chromium's
/// `InMemoryUrlProtocol` serves its bytes (the demuxer sees the same bytes
/// and sizes; the file is not held in memory).
struct FileIo {
    file: std::fs::File,
    len: u64,
}

unsafe extern "C" fn read_file(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    use std::io::Read;
    // SAFETY: `opaque` is the session's `FileIo`, alive while it is open.
    let f = unsafe { &mut *opaque.cast::<FileIo>() };
    // SAFETY: FFmpeg's buffer holds `size` bytes.
    let out = unsafe { std::slice::from_raw_parts_mut(buf, size.max(0) as usize) };
    // As the in-memory reader: as much as asked for, short only at the end.
    let mut n = 0;
    while n < out.len() {
        match f.file.read(&mut out[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return ff::AVERROR_EOF,
        }
    }
    if n == 0 {
        return ff::AVERROR_EOF;
    }
    n as c_int
}

unsafe extern "C" fn seek_file(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    use std::io::{Seek, SeekFrom};
    // SAFETY: as in `read_file`.
    let f = unsafe { &mut *opaque.cast::<FileIo>() };
    let whence = whence & !(ff::AVSEEK_FORCE as c_int);
    let pos = f.file.stream_position().unwrap_or(0) as i64;
    let to = match whence {
        w if w == ff::AVSEEK_SIZE as c_int => return f.len as i64,
        0 => offset,
        1 => pos + offset,
        2 => f.len as i64 + offset,
        _ => return -1,
    };
    if to < 0 {
        return -1;
    }
    let to = (to as u64).min(f.len);
    match f.file.seek(SeekFrom::Start(to)) {
        Ok(p) => p as i64,
        Err(_) => -1,
    }
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
    _io: Box<FileIo>,
    flushed: bool,
    done: bool,
}

// SAFETY: a session is used by one thread at a time (behind the source's
// mutex) and owns every FFmpeg object it points to.
unsafe impl Send for Session {}

impl Session {
    fn open(lib: &'static Lib, path: &Path) -> Result<Session> {
        let file = std::fs::File::open(path).map_err(|e| err(&format!("open: {e}")))?;
        let len = file
            .metadata()
            .map_err(|e| err(&format!("open: {e}")))?
            .len();
        // SAFETY: FFmpeg calls through today's library; every pointer is
        // checked and freed in `Drop`. The I/O context points into the
        // boxed `FileIo`, which the session owns and never moves.
        unsafe {
            let mut s = Session {
                lib,
                format: (lib.alloc_format)(),
                io: std::ptr::null_mut(),
                codec: std::ptr::null_mut(),
                packet: (lib.alloc_packet)(),
                frame: (lib.alloc_frame)(),
                stream: -1,
                _io: Box::new(FileIo { file, len }),
                flushed: false,
                done: false,
            };
            let buf = (lib.malloc)(32768).cast::<u8>();
            if buf.is_null() {
                return Err(err("out of memory"));
            }
            s.io = (lib.alloc_io)(
                buf,
                32768,
                0,
                (&mut *s._io as *mut FileIo).cast(),
                Some(read_file),
                std::ptr::null(),
                Some(seek_file),
            );
            if s.io.is_null() {
                (lib.free)(buf.cast());
            }
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

/// Whole decodes running at once without a waiting reader (a project's
/// opening decodes its files a few at a time; a file a seek waits on starts
/// at once).
const BACKGROUND_DECODES: usize = 2;

/// The decode slots in use.
fn slots() -> &'static (std::sync::Mutex<usize>, std::sync::Condvar) {
    static SLOTS: OnceLock<(std::sync::Mutex<usize>, std::sync::Condvar)> = OnceLock::new();
    SLOTS.get_or_init(Default::default)
}

/// The decoded samples of a file so far.
struct Decoded {
    /// Sparse planar blocks on disk; the handle removes the anonymous file.
    file: std::fs::File,
    frames: u64,
    done: bool,
    error: Option<String>,
}

/// One file's sequential decode from its start, shared by every open of
/// the file while any is open.
struct Whole {
    key: FileKey,
    channels: usize,
    decoded: std::sync::Mutex<Decoded>,
    grew: std::sync::Condvar,
    /// A reader waits on it: it decodes without waiting for a slot.
    wanted: std::sync::atomic::AtomicBool,
}

type Wholes = std::sync::Mutex<BTreeMap<(FileKey, usize), std::sync::Weak<Whole>>>;

fn wholes() -> &'static Wholes {
    static WHOLES: OnceLock<Wholes> = OnceLock::new();
    WHOLES.get_or_init(Default::default)
}

/// Resident sample bytes outside SourceCache (zero: shared PCM is on disk).
pub fn resident_bytes() -> usize {
    let live: Vec<std::sync::Arc<Whole>> = wholes()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter_map(std::sync::Weak::upgrade)
        .collect();
    live.iter().map(|w| w.bytes()).sum()
}

/// Resident sample bytes of an open spool (zero); None after its last reader.
pub fn resident_bytes_of(path: &Path) -> Option<usize> {
    let key = file_key(path)?;
    let live: Vec<std::sync::Arc<Whole>> = wholes()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|((k, _), _)| *k == key)
        .filter_map(|(_, w)| w.upgrade())
        .collect();
    (!live.is_empty()).then(|| live.iter().map(|w| w.bytes()).sum())
}

impl Whole {
    /// The open decode of `path`, or a new one (its session opened here, so a
    /// file today's library cannot open fails now; the decode runs on its
    /// own thread).
    fn get(lib: &'static Lib, path: &Path, channels: usize) -> Result<std::sync::Arc<Whole>> {
        let key = file_key(path).ok_or_else(|| err("cannot read the file's metadata"))?;
        let mut map = wholes().lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, w| w.strong_count() > 0);
        if let Some(w) = map
            .get(&(key.clone(), channels))
            .and_then(std::sync::Weak::upgrade)
        {
            return Ok(w);
        }
        let session = Session::open(lib, path)?;
        let whole = std::sync::Arc::new(Whole {
            key: key.clone(),
            channels,
            decoded: std::sync::Mutex::new(Decoded {
                file: tempfile::tempfile().map_err(|e| err(&format!("PCM spool: {e}")))?,
                frames: 0,
                done: false,
                error: None,
            }),
            grew: std::sync::Condvar::new(),
            wanted: std::sync::atomic::AtomicBool::new(false),
        });
        map.insert((key, channels), std::sync::Arc::downgrade(&whole));
        drop(map);
        let weak = std::sync::Arc::downgrade(&whole);
        let spawned = std::thread::Builder::new()
            .name("pm-audio-decode-whole".into())
            .spawn(move || Whole::run(weak, session));
        if let Err(e) = spawned {
            let mut d = whole.lock();
            d.done = true;
            d.error = Some(format!("cannot start the decode: {e}"));
        }
        Ok(whole)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Decoded> {
        self.decoded.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn bytes(&self) -> usize {
        // PCM lives in the temporary file. Only SourceCache's blocks are resident.
        0
    }

    /// The decode: waits for a slot unless a reader wants it, then appends
    /// each decoded frame; stops when no open decoder holds the file.
    fn run(weak: std::sync::Weak<Whole>, mut session: Session) {
        let (gate, freed) = slots();
        {
            let mut used = gate.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                let Some(w) = weak.upgrade() else { return };
                if *used < BACKGROUND_DECODES || w.wanted.load(std::sync::atomic::Ordering::Acquire)
                {
                    break;
                }
                drop(w);
                used = freed
                    .wait_timeout(used, std::time::Duration::from_millis(20))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            *used += 1;
        }
        struct Slot;
        impl Drop for Slot {
            fn drop(&mut self) {
                let (gate, freed) = slots();
                *gate.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
                freed.notify_all();
            }
        }
        let _slot = Slot;
        let Some(channels) = weak.upgrade().map(|w| w.channels) else {
            return;
        };
        let mut frame: Vec<Vec<f32>> = (0..channels).map(|_| Vec::with_capacity(4096)).collect();
        loop {
            for c in &mut frame {
                c.clear();
            }
            let step = session.next(&mut frame);
            let Some(w) = weak.upgrade() else { return };
            let mut d = w.lock();
            match step {
                Ok(true) => {
                    if let Err(e) = d.append(&frame) {
                        d.done = true;
                        d.error = Some(e.to_string());
                        drop(d);
                        w.grew.notify_all();
                        return;
                    }
                }
                Ok(false) => {
                    d.done = true;
                    let frames = d.frames;
                    drop(d);
                    lengths()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(w.key.clone(), frames);
                    w.grew.notify_all();
                    return;
                }
                Err(e) => {
                    d.done = true;
                    d.error = Some(e.to_string());
                    drop(d);
                    w.grew.notify_all();
                    return;
                }
            }
            drop(d);
            w.grew.notify_all();
        }
    }

    /// Waits until the decode has passed frame `end` (or ended).
    fn wait_for(&self, end: u64) -> std::sync::MutexGuard<'_, Decoded> {
        let mut d = self.lock();
        if d.frames < end && !d.done {
            self.wanted
                .store(true, std::sync::atomic::Ordering::Release);
            slots().1.notify_all();
            while d.frames < end && !d.done {
                d = self.grew.wait(d).unwrap_or_else(|e| e.into_inner());
            }
        }
        d
    }
}

impl Decoded {
    fn append(&mut self, frame: &[Vec<f32>]) -> Result<()> {
        use std::os::unix::fs::FileExt;
        let n = frame.first().map_or(0, Vec::len);
        let mut at = 0;
        while at < n {
            let block = self.frames / BLOCK as u64;
            let off = self.frames as usize % BLOCK;
            let take = (BLOCK - off).min(n - at);
            for (channel, samples) in frame.iter().enumerate() {
                let offset =
                    ((block * frame.len() as u64 + channel as u64) * BLOCK as u64 + off as u64) * 4;
                // SAFETY: f32 samples are initialized, and the byte view has
                // exactly their size. This private spool is read on this host.
                let bytes = unsafe {
                    std::slice::from_raw_parts(samples[at..at + take].as_ptr().cast(), take * 4)
                };
                self.file
                    .write_all_at(bytes, offset)
                    .map_err(|e| err(&format!("PCM spool: {e}")))?;
            }
            at += take;
            self.frames += take as u64;
        }
        Ok(())
    }

    fn copy(&self, from: u64, frames: usize, out: &mut [Vec<f32>]) -> Result<usize> {
        use std::os::unix::fs::FileExt;
        let end = (from + frames as u64).min(self.frames);
        let mut at = from;
        while at < end {
            let block = at / BLOCK as u64;
            let off = at as usize % BLOCK;
            let take = ((end - at) as usize).min(BLOCK - off);
            let channels = out.len();
            for (channel, dst) in out.iter_mut().enumerate() {
                let first = dst.len();
                dst.resize(first + take, 0.0);
                let offset =
                    ((block * channels as u64 + channel as u64) * BLOCK as u64 + off as u64) * 4;
                // SAFETY: the initialized destination has take f32 elements;
                // every f32 bit pattern is valid, and the view does not escape.
                let bytes = unsafe {
                    std::slice::from_raw_parts_mut(dst[first..].as_mut_ptr().cast(), take * 4)
                };
                self.file
                    .read_exact_at(bytes, offset)
                    .map_err(|e| err(&format!("PCM spool: {e}")))?;
            }
            at += take as u64;
        }
        Ok(end.saturating_sub(from) as usize)
    }
}

/// A source decoded by today's library: a reader of the file's shared
/// whole decode.
pub struct ChromiumDecoder {
    whole: Option<std::sync::Arc<Whole>>,
    path: PathBuf,
    channels: usize,
    /// Program sample of the next read.
    position: u64,
}

impl ChromiumDecoder {
    /// `None` when today's library is not loaded in this process.
    pub fn open(path: &Path, channels: usize) -> Result<Option<ChromiumDecoder>> {
        let Some(_) = lib() else {
            return Ok(None);
        };
        Ok(Some(ChromiumDecoder {
            whole: None,
            path: path.to_owned(),
            channels,
            position: 0,
        }))
    }

    fn whole(&mut self) -> Result<&std::sync::Arc<Whole>> {
        if self.whole.is_none() {
            self.whole = Some(Whole::get(
                lib().ok_or_else(|| err("library unavailable"))?,
                &self.path,
                self.channels,
            )?);
        }
        Ok(self.whole.as_ref().expect("opened"))
    }

    /// The decoded length in frames: waits for the whole decode.
    pub fn count_frames(&mut self) -> Result<u64> {
        let d = self.whole()?.wait_for(u64::MAX);
        match &d.error {
            Some(e) => Err(err(e)),
            None => Ok(d.frames),
        }
    }

    /// Positions the next read at program sample `index`: the samples equal
    /// the sequential decode from the file's start.
    pub fn seek_anchored(&mut self, index: u64) -> Result<()> {
        self.position = index;
        Ok(())
    }

    /// Up to `frames` samples per channel appended to `out` (fewer only at
    /// the end of the stream); waits for the decode to pass them.
    pub fn read(&mut self, out: &mut [Vec<f32>], frames: usize) -> Result<usize> {
        let position = self.position;
        let d = self.whole()?.wait_for(position + frames as u64);
        if d.frames < position + frames as u64
            && let Some(e) = &d.error
        {
            return Err(err(e));
        }
        let got = d.copy(position, frames, out)?;
        drop(d);
        self.position += got as u64;
        Ok(got)
    }
}
