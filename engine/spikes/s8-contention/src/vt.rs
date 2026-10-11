//! Concurrent VideoToolbox compression sessions (export encoders).
//!
//! Each session runs on its own thread, encodes 4K frames from a small pool of
//! pre-filled IOSurface-backed pixel buffers (moving gradient plus noise, so the
//! encoder does real work every frame), keeps a bounded number of frames in flight,
//! and records throughput and per-frame encode latency.

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use serde::Serialize;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

type OSStatus = i32;
type CFTypeRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFAllocatorRef = *const c_void;
type VTSession = *mut c_void;
type CVPixelBufferRef = *mut c_void;
type CVPixelBufferPoolRef = *mut c_void;
type CMSampleBufferRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

const CMTIME_VALID: u32 = 1;
const INVALID_TIME: CMTime = CMTime { value: 0, timescale: 0, flags: 0, epoch: 0 };

type OutputCallback = extern "C" fn(*mut c_void, *mut c_void, OSStatus, u32, CMSampleBufferRef);

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    fn VTCompressionSessionCreate(
        allocator: CFAllocatorRef,
        width: i32,
        height: i32,
        codec_type: u32,
        encoder_specification: CFDictionaryRef,
        source_image_buffer_attributes: CFDictionaryRef,
        compressed_data_allocator: CFAllocatorRef,
        output_callback: Option<OutputCallback>,
        output_callback_ref_con: *mut c_void,
        session_out: *mut VTSession,
    ) -> OSStatus;
    fn VTSessionSetProperty(session: VTSession, key: CFStringRef, value: CFTypeRef) -> OSStatus;
    fn VTSessionCopyProperty(session: VTSession, key: CFStringRef, allocator: CFAllocatorRef, value_out: *mut c_void) -> OSStatus;
    fn VTCompressionSessionPrepareToEncodeFrames(session: VTSession) -> OSStatus;
    fn VTCompressionSessionEncodeFrame(
        session: VTSession,
        image_buffer: CVPixelBufferRef,
        pts: CMTime,
        duration: CMTime,
        frame_properties: CFDictionaryRef,
        source_frame_ref_con: *mut c_void,
        info_flags_out: *mut u32,
    ) -> OSStatus;
    fn VTCompressionSessionCompleteFrames(session: VTSession, complete_until: CMTime) -> OSStatus;
    fn VTCompressionSessionInvalidate(session: VTSession);
    fn VTCompressionSessionGetPixelBufferPool(session: VTSession) -> CVPixelBufferPoolRef;

    static kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: CFStringRef;
    static kVTCompressionPropertyKey_RealTime: CFStringRef;
    static kVTCompressionPropertyKey_ProfileLevel: CFStringRef;
    static kVTCompressionPropertyKey_AverageBitRate: CFStringRef;
    static kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder: CFStringRef;
    static kVTProfileLevel_HEVC_Main10_AutoLevel: CFStringRef;
    static kVTProfileLevel_H264_High_AutoLevel: CFStringRef;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferPoolCreatePixelBuffer(allocator: CFAllocatorRef, pool: CVPixelBufferPoolRef, out: *mut CVPixelBufferRef) -> i32;
    fn CVPixelBufferLockBaseAddress(pb: CVPixelBufferRef, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pb: CVPixelBufferRef, flags: u64) -> i32;
    fn CVPixelBufferGetPlaneCount(pb: CVPixelBufferRef) -> usize;
    fn CVPixelBufferGetBaseAddressOfPlane(pb: CVPixelBufferRef, plane: usize) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRowOfPlane(pb: CVPixelBufferRef, plane: usize) -> usize;
    fn CVPixelBufferGetHeightOfPlane(pb: CVPixelBufferRef, plane: usize) -> usize;
    fn CVPixelBufferGetWidthOfPlane(pb: CVPixelBufferRef, plane: usize) -> usize;

    static kCVPixelBufferPixelFormatTypeKey: CFStringRef;
    static kCVPixelBufferWidthKey: CFStringRef;
    static kCVPixelBufferHeightKey: CFStringRef;
    static kCVPixelBufferIOSurfacePropertiesKey: CFStringRef;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetTotalSampleSize(sb: CMSampleBufferRef) -> usize;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc10,
    ProRes422Hq,
}

impl Codec {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "h264" => Self::H264,
            "hevc10" => Self::Hevc10,
            "prores422hq" | "prores" => Self::ProRes422Hq,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc10 => "hevc10",
            Self::ProRes422Hq => "prores422hq",
        }
    }

    fn codec_type(self) -> u32 {
        match self {
            Self::H264 => fourcc(b"avc1"),
            Self::Hevc10 => fourcc(b"hvc1"),
            Self::ProRes422Hq => fourcc(b"apch"),
        }
    }

    /// Native encoder input formats: 8-bit 4:2:0, 10-bit 4:2:0, 10-bit 4:2:2 (biplanar).
    fn pixel_format(self) -> u32 {
        match self {
            Self::H264 => fourcc(b"420v"),
            Self::Hevc10 => fourcc(b"x420"),
            Self::ProRes422Hq => fourcc(b"x422"),
        }
    }

    fn ten_bit(self) -> bool {
        !matches!(self, Self::H264)
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub codec: Codec,
    pub width: i32,
    pub height: i32,
    pub sessions: usize,
    /// Frames in flight per session.
    pub max_pending: i64,
    /// Optional per-session frame-rate cap (frames per second); `None` = as fast as possible.
    pub fps_cap: Option<f64>,
    pub qos: Option<crate::sys::Qos>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStats {
    pub codec: &'static str,
    pub index: usize,
    pub created: bool,
    pub error: Option<String>,
    pub hardware: Option<bool>,
    pub frames: u64,
    pub dropped: u64,
    pub errors: u64,
    pub bytes: u64,
    pub seconds: f64,
    pub fps: f64,
    pub encode_latency_ms: crate::stats::Dist,
}

struct CallbackState {
    pending: AtomicI64,
    frames: AtomicU64,
    dropped: AtomicU64,
    errors: AtomicU64,
    bytes: AtomicU64,
    latency_ms: Mutex<Vec<f64>>,
}

extern "C" fn on_output(refcon: *mut c_void, source: *mut c_void, status: OSStatus, _flags: u32, sb: CMSampleBufferRef) {
    // SAFETY: refcon is the Arc<CallbackState> pointer kept alive by the session thread
    // until after CompleteFrames and Invalidate.
    let st = unsafe { &*(refcon as *const CallbackState) };
    let submitted_ns = source as usize as u64;
    if status != 0 {
        st.errors.fetch_add(1, Ordering::Relaxed);
    } else if sb.is_null() {
        st.dropped.fetch_add(1, Ordering::Relaxed);
    } else {
        // SAFETY: valid sample buffer for the duration of the callback.
        let size = unsafe { CMSampleBufferGetTotalSampleSize(sb) };
        st.bytes.fetch_add(size as u64, Ordering::Relaxed);
        st.frames.fetch_add(1, Ordering::Relaxed);
        let lat = (crate::clock::now_ns().saturating_sub(submitted_ns)) as f64 / 1e6;
        st.latency_ms.lock().unwrap().push(lat);
    }
    st.pending.fetch_sub(1, Ordering::AcqRel);
}

fn cfstr(s: CFStringRef) -> CFString {
    // SAFETY: framework constant strings are valid for the process lifetime.
    unsafe { CFString::wrap_under_get_rule(s) }
}

fn fill_pixel_buffer(pb: CVPixelBufferRef, frame: usize, ten_bit: bool) {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64 ^ (frame as u64).wrapping_mul(0x2545_F491_4F6C_DD1D);
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    // SAFETY: lock/unlock bracket every access; plane pointers and strides come from CV.
    unsafe {
        assert_eq!(CVPixelBufferLockBaseAddress(pb, 0), 0);
        for plane in 0..CVPixelBufferGetPlaneCount(pb) {
            let base = CVPixelBufferGetBaseAddressOfPlane(pb, plane) as *mut u8;
            let stride = CVPixelBufferGetBytesPerRowOfPlane(pb, plane);
            let h = CVPixelBufferGetHeightOfPlane(pb, plane);
            let w = CVPixelBufferGetWidthOfPlane(pb, plane);
            let comps = if plane == 0 { 1 } else { 2 };
            for y in 0..h {
                let row = base.add(y * stride);
                for x in 0..w * comps {
                    let grad = ((x + y + frame * 37) % 1024) as u32;
                    let noise = (rnd() & 63) as u32;
                    let v10 = (64 + (grad * 800 / 1024) + noise).min(940);
                    if ten_bit {
                        *(row.add(x * 2) as *mut u16) = (v10 << 6) as u16;
                    } else {
                        *row.add(x) = (v10 >> 2) as u8;
                    }
                }
            }
        }
        CVPixelBufferUnlockBaseAddress(pb, 0);
    }
}

fn run_session(cfg: &Config, index: usize, stop: &AtomicBool) -> SessionStats {
    if let Some(q) = cfg.qos {
        let _ = crate::sys::set_thread_qos(q);
    }
    let mut stats = SessionStats {
        codec: cfg.codec.name(),
        index,
        created: false,
        error: None,
        hardware: None,
        frames: 0,
        dropped: 0,
        errors: 0,
        bytes: 0,
        seconds: 0.0,
        fps: 0.0,
        encode_latency_ms: Default::default(),
    };
    let state = Arc::new(CallbackState {
        pending: AtomicI64::new(0),
        frames: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
        latency_ms: Mutex::new(Vec::new()),
    });
    // SAFETY: CF objects are created and released here; the callback state outlives the
    // session (Invalidate is called before `state` drops).
    unsafe {
        let spec = CFDictionary::from_CFType_pairs(&[(
            cfstr(kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder).as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        let empty: CFDictionary<CFType, CFType> = CFDictionary::from_CFType_pairs(&[]);
        let attrs = CFDictionary::from_CFType_pairs(&[
            (cfstr(kCVPixelBufferPixelFormatTypeKey).as_CFType(), CFNumber::from(cfg.codec.pixel_format() as i32).as_CFType()),
            (cfstr(kCVPixelBufferWidthKey).as_CFType(), CFNumber::from(cfg.width).as_CFType()),
            (cfstr(kCVPixelBufferHeightKey).as_CFType(), CFNumber::from(cfg.height).as_CFType()),
            (cfstr(kCVPixelBufferIOSurfacePropertiesKey).as_CFType(), empty.as_CFType()),
        ]);
        let mut session: VTSession = std::ptr::null_mut();
        let rc = VTCompressionSessionCreate(
            std::ptr::null(),
            cfg.width,
            cfg.height,
            cfg.codec.codec_type(),
            spec.as_concrete_TypeRef() as CFDictionaryRef,
            attrs.as_concrete_TypeRef() as CFDictionaryRef,
            std::ptr::null(),
            Some(on_output),
            Arc::as_ptr(&state) as *mut c_void,
            &mut session,
        );
        if rc != 0 || session.is_null() {
            stats.error = Some(format!("VTCompressionSessionCreate = {rc}"));
            return stats;
        }
        stats.created = true;
        VTSessionSetProperty(session, kVTCompressionPropertyKey_RealTime, CFBoolean::false_value().as_CFTypeRef());
        match cfg.codec {
            Codec::Hevc10 => {
                VTSessionSetProperty(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_HEVC_Main10_AutoLevel as CFTypeRef);
                VTSessionSetProperty(session, kVTCompressionPropertyKey_AverageBitRate, CFNumber::from(100_000_000i32).as_CFTypeRef());
            }
            Codec::H264 => {
                VTSessionSetProperty(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel as CFTypeRef);
                VTSessionSetProperty(session, kVTCompressionPropertyKey_AverageBitRate, CFNumber::from(60_000_000i32).as_CFTypeRef());
            }
            Codec::ProRes422Hq => {}
        }
        let mut hw: CFTypeRef = std::ptr::null();
        if VTSessionCopyProperty(
            session,
            kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
            std::ptr::null(),
            &mut hw as *mut _ as *mut c_void,
        ) == 0
            && !hw.is_null()
        {
            stats.hardware = Some(CFBoolean::wrap_under_create_rule(hw as _).into());
        }
        let rc = VTCompressionSessionPrepareToEncodeFrames(session);
        if rc != 0 {
            stats.error = Some(format!("PrepareToEncodeFrames = {rc}"));
        }
        let pool = VTCompressionSessionGetPixelBufferPool(session);
        let mut buffers = Vec::new();
        if !pool.is_null() {
            for i in 0..8 {
                let mut pb: CVPixelBufferRef = std::ptr::null_mut();
                if CVPixelBufferPoolCreatePixelBuffer(std::ptr::null(), pool, &mut pb) == 0 && !pb.is_null() {
                    fill_pixel_buffer(pb, i * 3 + index, cfg.codec.ten_bit());
                    buffers.push(pb);
                }
            }
        }
        if buffers.is_empty() {
            stats.error = Some("no pixel buffers from the session pool".into());
            VTCompressionSessionInvalidate(session);
            CFRelease(session as CFTypeRef);
            return stats;
        }
        let start = crate::clock::now_ns();
        let mut n: i64 = 0;
        while !stop.load(Ordering::Relaxed) {
            if state.pending.load(Ordering::Acquire) >= cfg.max_pending {
                crate::clock::sleep_ns(200_000);
                continue;
            }
            if let Some(cap) = cfg.fps_cap {
                let due = start + (n as f64 * 1e9 / cap) as u64;
                crate::clock::sleep_until_ns(due);
            }
            let pb = buffers[n as usize % buffers.len()];
            state.pending.fetch_add(1, Ordering::AcqRel);
            let pts = CMTime { value: n * 1001, timescale: 60000, flags: CMTIME_VALID, epoch: 0 };
            let dur = CMTime { value: 1001, timescale: 60000, flags: CMTIME_VALID, epoch: 0 };
            let rc = VTCompressionSessionEncodeFrame(
                session,
                pb,
                pts,
                dur,
                std::ptr::null(),
                crate::clock::now_ns() as usize as *mut c_void,
                std::ptr::null_mut(),
            );
            if rc != 0 {
                state.pending.fetch_sub(1, Ordering::AcqRel);
                state.errors.fetch_add(1, Ordering::Relaxed);
                crate::clock::sleep_ns(1_000_000);
            }
            n += 1;
        }
        VTCompressionSessionCompleteFrames(session, INVALID_TIME);
        let secs = (crate::clock::now_ns() - start) as f64 / 1e9;
        VTCompressionSessionInvalidate(session);
        CFRelease(session as CFTypeRef);
        for pb in buffers {
            CFRelease(pb as CFTypeRef);
        }
        stats.frames = state.frames.load(Ordering::Relaxed);
        stats.dropped = state.dropped.load(Ordering::Relaxed);
        stats.errors = state.errors.load(Ordering::Relaxed);
        stats.bytes = state.bytes.load(Ordering::Relaxed);
        stats.seconds = crate::stats::r(secs, 3);
        stats.fps = crate::stats::r(stats.frames as f64 / secs, 2);
        stats.encode_latency_ms = crate::stats::dist(&state.latency_ms.lock().unwrap()).rounded(2);
    }
    stats
}

/// Start `cfg.sessions` sessions; they run until `stop` is set. Join the handle for stats.
pub fn spawn(cfg: Config, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<Vec<SessionStats>> {
    std::thread::spawn(move || {
        let handles: Vec<_> = (0..cfg.sessions)
            .map(|i| {
                let cfg = cfg.clone();
                let stop = stop.clone();
                std::thread::spawn(move || run_session(&cfg, i, &stop))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_values() {
        assert_eq!(fourcc(b"avc1"), 0x6176_6331);
        assert_eq!(Codec::Hevc10.pixel_format(), 0x7834_3230);
        assert_eq!(Codec::parse("prores"), Some(Codec::ProRes422Hq));
    }

    /// Encodes real frames on the hardware encoder for a moment (small size to stay fast).
    #[test]
    fn h264_session_encodes_frames() {
        let stop = Arc::new(AtomicBool::new(false));
        let h = spawn(
            Config { codec: Codec::H264, width: 640, height: 360, sessions: 1, max_pending: 16, fps_cap: None, qos: None },
            stop.clone(),
        );
        std::thread::sleep(std::time::Duration::from_millis(500));
        stop.store(true, Ordering::Relaxed);
        let s = &h.join().unwrap()[0];
        assert!(s.created, "{:?}", s.error);
        assert!(s.frames > 10, "{s:?}");
        assert!(s.bytes > 0);
        assert_eq!(s.errors, 0);
    }
}
