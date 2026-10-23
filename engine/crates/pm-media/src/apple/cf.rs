//! Minimal CoreFoundation, CoreMedia, CoreVideo, VideoToolbox, ImageIO, and
//! CoreGraphics FFI.
//!
//! Declared by hand so ownership is exact: every `Cf` owns one retain and
//! releases it on drop.
#![allow(non_upper_case_globals, non_snake_case, clippy::missing_safety_doc)]

use std::ffi::{c_char, c_void};

pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFAllocatorRef = *const c_void;
pub type OSStatus = i32;
pub type CVReturn = i32;
pub type CVPixelBufferRef = *mut c_void;
pub type CMFormatDescriptionRef = *mut c_void;
pub type CMSampleBufferRef = *mut c_void;
pub type CMBlockBufferRef = *mut c_void;
pub type VTDecompressionSessionRef = *mut c_void;
pub type CVMetalTextureCacheRef = *mut c_void;
pub type CVMetalTextureRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

impl CMTime {
    pub const FLAG_VALID: u32 = 1;
    pub fn new(value: i64, timescale: i32) -> Self {
        CMTime {
            value,
            timescale,
            flags: Self::FLAG_VALID,
            epoch: 0,
        }
    }
    pub fn invalid() -> Self {
        CMTime {
            value: 0,
            timescale: 0,
            flags: 0,
            epoch: 0,
        }
    }
    pub fn is_valid(&self) -> bool {
        self.flags & Self::FLAG_VALID != 0
    }
}

#[repr(C)]
pub struct CMSampleTimingInfo {
    pub duration: CMTime,
    pub pts: CMTime,
    pub dts: CMTime,
}

#[repr(C)]
pub struct CFDictionaryCallBacks {
    _opaque: [usize; 6],
}

pub type VTDecompressionOutputCallback = unsafe extern "C" fn(
    refcon: *mut c_void,
    source_frame_refcon: *mut c_void,
    status: OSStatus,
    info_flags: u32,
    image_buffer: CVPixelBufferRef,
    pts: CMTime,
    duration: CMTime,
);

#[repr(C)]
pub struct VTDecompressionOutputCallbackRecord {
    pub callback: VTDecompressionOutputCallback,
    pub refcon: *mut c_void,
}

pub const kCFNumberSInt32Type: isize = 3;
pub const kCFNumberSInt64Type: isize = 4;
pub const kCFStringEncodingUTF8: u32 = 0x0800_0100;
pub const kCMBlockBufferAssureMemoryNowFlag: u32 = 1;
pub const kCVPixelBufferLock_ReadOnly: u64 = 1;
pub const kCVAttachmentMode_ShouldPropagate: u32 = 1;
pub const kVTDecodeInfo_FrameDropped: u32 = 1 << 1;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub static kCFAllocatorDefault: CFAllocatorRef;
    pub static kCFTypeDictionaryKeyCallBacks: CFDictionaryCallBacks;
    pub static kCFTypeDictionaryValueCallBacks: CFDictionaryCallBacks;
    pub static kCFBooleanTrue: CFTypeRef;
    pub static kCFBooleanFalse: CFTypeRef;
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFGetTypeID(cf: CFTypeRef) -> usize;
    pub fn CFStringGetTypeID() -> usize;
    pub fn CFNumberGetTypeID() -> usize;
    pub fn CFBooleanGetTypeID() -> usize;
    pub fn CFDictionaryCreate(
        alloc: CFAllocatorRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: isize,
        key_callbacks: *const CFDictionaryCallBacks,
        value_callbacks: *const CFDictionaryCallBacks,
    ) -> CFDictionaryRef;
    pub fn CFDictionaryGetValue(dict: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    pub fn CFNumberCreate(alloc: CFAllocatorRef, kind: isize, value: *const c_void) -> CFTypeRef;
    pub fn CFNumberGetValue(number: CFTypeRef, kind: isize, value: *mut c_void) -> u8;
    pub fn CFBooleanGetValue(b: CFTypeRef) -> u8;
    pub fn CFDataCreate(alloc: CFAllocatorRef, bytes: *const u8, len: isize) -> CFTypeRef;
    pub fn CFStringCreateWithBytes(
        alloc: CFAllocatorRef,
        bytes: *const u8,
        len: isize,
        encoding: u32,
        external: u8,
    ) -> CFStringRef;
    pub fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: isize, encoding: u32) -> u8;
    pub fn CFCopyDescription(cf: CFTypeRef) -> CFStringRef;
    pub fn CFDataGetTypeID() -> usize;
    pub fn CFDictionaryGetTypeID() -> usize;
    pub fn CFDataGetLength(d: CFTypeRef) -> isize;
    pub fn CFDataGetBytePtr(d: CFTypeRef) -> *const u8;
    pub fn CFURLCreateFromFileSystemRepresentation(
        alloc: CFAllocatorRef,
        buffer: *const u8,
        len: isize,
        is_directory: u8,
    ) -> CFTypeRef;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    pub static kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms: CFStringRef;
    pub static kCMFormatDescriptionExtension_FullRangeVideo: CFStringRef;
    pub fn CMFormatDescriptionGetExtensions(desc: CMFormatDescriptionRef) -> CFDictionaryRef;
    pub fn CMVideoFormatDescriptionCreate(
        alloc: CFAllocatorRef,
        codec_type: u32,
        width: i32,
        height: i32,
        extensions: CFDictionaryRef,
        out: *mut CMFormatDescriptionRef,
    ) -> OSStatus;
    pub fn CMVideoFormatDescriptionCreateFromHEVCParameterSets(
        alloc: CFAllocatorRef,
        count: usize,
        pointers: *const *const u8,
        sizes: *const usize,
        nal_unit_header_length: i32,
        extensions: CFDictionaryRef,
        out: *mut CMFormatDescriptionRef,
    ) -> OSStatus;
    pub fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        alloc: CFAllocatorRef,
        count: usize,
        pointers: *const *const u8,
        sizes: *const usize,
        nal_unit_header_length: i32,
        out: *mut CMFormatDescriptionRef,
    ) -> OSStatus;
    pub fn CMFormatDescriptionGetExtension(
        desc: CMFormatDescriptionRef,
        key: CFStringRef,
    ) -> CFTypeRef;
    pub fn CMBlockBufferCreateWithMemoryBlock(
        alloc: CFAllocatorRef,
        memory_block: *mut c_void,
        block_length: usize,
        block_allocator: CFAllocatorRef,
        custom_block_source: *const c_void,
        offset_to_data: usize,
        data_length: usize,
        flags: u32,
        out: *mut CMBlockBufferRef,
    ) -> OSStatus;
    pub fn CMBlockBufferReplaceDataBytes(
        source: *const c_void,
        dest: CMBlockBufferRef,
        offset: usize,
        len: usize,
    ) -> OSStatus;
    pub fn CMSampleBufferCreateReady(
        alloc: CFAllocatorRef,
        data_buffer: CMBlockBufferRef,
        format: CMFormatDescriptionRef,
        num_samples: isize,
        num_timing: isize,
        timing: *const CMSampleTimingInfo,
        num_sizes: isize,
        sizes: *const usize,
        out: *mut CMSampleBufferRef,
    ) -> OSStatus;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    pub static kCVPixelBufferPixelFormatTypeKey: CFStringRef;
    pub static kCVPixelBufferMetalCompatibilityKey: CFStringRef;
    pub static kCVPixelBufferIOSurfacePropertiesKey: CFStringRef;
    pub fn CVPixelBufferGetPixelFormatType(pb: CVPixelBufferRef) -> u32;
    pub fn CVPixelBufferGetWidth(pb: CVPixelBufferRef) -> usize;
    pub fn CVPixelBufferGetHeight(pb: CVPixelBufferRef) -> usize;
    pub fn CVPixelBufferIsPlanar(pb: CVPixelBufferRef) -> u8;
    pub fn CVPixelBufferGetPlaneCount(pb: CVPixelBufferRef) -> usize;
    pub fn CVPixelBufferGetWidthOfPlane(pb: CVPixelBufferRef, plane: usize) -> usize;
    pub fn CVPixelBufferGetHeightOfPlane(pb: CVPixelBufferRef, plane: usize) -> usize;
    pub fn CVPixelBufferGetBytesPerRowOfPlane(pb: CVPixelBufferRef, plane: usize) -> usize;
    pub fn CVPixelBufferGetBaseAddressOfPlane(pb: CVPixelBufferRef, plane: usize) -> *mut c_void;
    pub fn CVPixelBufferGetBytesPerRow(pb: CVPixelBufferRef) -> usize;
    pub fn CVPixelBufferGetBaseAddress(pb: CVPixelBufferRef) -> *mut c_void;
    pub fn CVPixelBufferLockBaseAddress(pb: CVPixelBufferRef, flags: u64) -> CVReturn;
    pub fn CVPixelBufferUnlockBaseAddress(pb: CVPixelBufferRef, flags: u64) -> CVReturn;
    pub fn CVPixelBufferGetIOSurface(pb: CVPixelBufferRef) -> *mut c_void;
    pub fn CVBufferCopyAttachments(buf: CVPixelBufferRef, mode: u32) -> CFDictionaryRef;
    pub fn CVMetalTextureCacheCreate(
        alloc: CFAllocatorRef,
        cache_attributes: CFDictionaryRef,
        metal_device: *mut c_void,
        texture_attributes: CFDictionaryRef,
        out: *mut CVMetalTextureCacheRef,
    ) -> CVReturn;
    pub fn CVMetalTextureCacheCreateTextureFromImage(
        alloc: CFAllocatorRef,
        cache: CVMetalTextureCacheRef,
        image: CVPixelBufferRef,
        texture_attributes: CFDictionaryRef,
        pixel_format: usize,
        width: usize,
        height: usize,
        plane: usize,
        out: *mut CVMetalTextureRef,
    ) -> CVReturn;
    pub fn CVMetalTextureGetTexture(texture: CVMetalTextureRef) -> *mut c_void;
    pub fn CVMetalTextureCacheFlush(cache: CVMetalTextureCacheRef, options: u64);
    pub fn CVBufferCopyAttachment(
        buf: CVPixelBufferRef,
        key: CFStringRef,
        mode: *mut u32,
    ) -> CFTypeRef;
    pub fn CVColorPrimariesGetIntegerCodePointForString(value: CFStringRef) -> i32;
    pub fn CVTransferFunctionGetIntegerCodePointForString(value: CFStringRef) -> i32;
    pub fn CVYCbCrMatrixGetIntegerCodePointForString(value: CFStringRef) -> i32;
    pub fn CVPixelBufferGetDataSize(pb: CVPixelBufferRef) -> usize;
    pub static kCVImageBufferYCbCrMatrixKey: CFStringRef;
    pub static kCVImageBufferColorPrimariesKey: CFStringRef;
    pub static kCVImageBufferTransferFunctionKey: CFStringRef;
    pub static kCVImageBufferChromaLocationTopFieldKey: CFStringRef;
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    pub static kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder: CFStringRef;
    pub static kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder: CFStringRef;
    pub static kVTDecompressionPropertyKey_UsingHardwareAcceleratedVideoDecoder: CFStringRef;
    pub fn VTDecompressionSessionCreate(
        alloc: CFAllocatorRef,
        format: CMFormatDescriptionRef,
        decoder_spec: CFDictionaryRef,
        destination_attributes: CFDictionaryRef,
        callback: *const VTDecompressionOutputCallbackRecord,
        out: *mut VTDecompressionSessionRef,
    ) -> OSStatus;
    pub fn VTDecompressionSessionDecodeFrame(
        session: VTDecompressionSessionRef,
        sample: CMSampleBufferRef,
        flags: u32,
        source_frame_refcon: *mut c_void,
        info_out: *mut u32,
    ) -> OSStatus;
    pub fn VTDecompressionSessionWaitForAsynchronousFrames(
        session: VTDecompressionSessionRef,
    ) -> OSStatus;
    pub fn VTDecompressionSessionFinishDelayedFrames(
        session: VTDecompressionSessionRef,
    ) -> OSStatus;
    pub fn VTDecompressionSessionInvalidate(session: VTDecompressionSessionRef);
    pub fn VTSessionCopyProperty(
        session: *mut c_void,
        key: CFStringRef,
        alloc: CFAllocatorRef,
        out: *mut c_void,
    ) -> OSStatus;
    pub fn VTIsHardwareDecodeSupported(codec_type: u32) -> u8;
    pub fn VTRegisterProfessionalVideoWorkflowVideoDecoders();
    pub fn VTRegisterSupplementalVideoDecoderIfAvailable(codec_type: u32);
}

/// One owned CoreFoundation reference.
pub struct Cf(CFTypeRef);

// CoreFoundation retain/release is thread safe; the objects used here are
// immutable after creation or internally synchronized (pixel buffers).
unsafe impl Send for Cf {}
unsafe impl Sync for Cf {}

impl Cf {
    /// Takes ownership of a +1 reference. Returns None for null.
    pub unsafe fn from_create(p: CFTypeRef) -> Option<Cf> {
        if p.is_null() { None } else { Some(Cf(p)) }
    }
    /// Retains a +0 reference.
    pub unsafe fn retain(p: CFTypeRef) -> Option<Cf> {
        if p.is_null() {
            None
        } else {
            Some(Cf(unsafe { CFRetain(p) }))
        }
    }
    pub fn as_ptr(&self) -> CFTypeRef {
        self.0
    }
    pub fn as_mut_ptr(&self) -> *mut c_void {
        self.0 as *mut c_void
    }
}

impl std::fmt::Debug for Cf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cf({})", unsafe { cf_describe(self.0) })
    }
}

impl Clone for Cf {
    fn clone(&self) -> Self {
        unsafe { Cf(CFRetain(self.0)) }
    }
}

impl Drop for Cf {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

pub fn cf_str(s: &str) -> Cf {
    unsafe {
        Cf::from_create(CFStringCreateWithBytes(
            kCFAllocatorDefault,
            s.as_ptr(),
            s.len() as isize,
            kCFStringEncodingUTF8,
            0,
        ))
        .expect("CFString")
    }
}

pub fn cf_i32(v: i32) -> Cf {
    unsafe {
        Cf::from_create(CFNumberCreate(
            kCFAllocatorDefault,
            kCFNumberSInt32Type,
            &v as *const i32 as _,
        ))
        .expect("CFNumber")
    }
}

pub fn cf_data(bytes: &[u8]) -> Cf {
    unsafe {
        Cf::from_create(CFDataCreate(
            kCFAllocatorDefault,
            bytes.as_ptr(),
            bytes.len() as isize,
        ))
        .expect("CFData")
    }
}

pub fn cf_bool(v: bool) -> CFTypeRef {
    unsafe { if v { kCFBooleanTrue } else { kCFBooleanFalse } }
}

/// Builds a CFDictionary; keys and values are retained by the dictionary.
pub fn cf_dict(pairs: &[(CFTypeRef, CFTypeRef)]) -> Cf {
    let keys: Vec<CFTypeRef> = pairs.iter().map(|p| p.0).collect();
    let values: Vec<CFTypeRef> = pairs.iter().map(|p| p.1).collect();
    unsafe {
        Cf::from_create(CFDictionaryCreate(
            kCFAllocatorDefault,
            keys.as_ptr(),
            values.as_ptr(),
            pairs.len() as isize,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        ))
        .expect("CFDictionary")
    }
}

/// Human-readable CF description, for reports.
///
/// # Safety
/// `cf` is null or a valid CF object.
pub unsafe fn cf_describe(cf: CFTypeRef) -> String {
    if cf.is_null() {
        return "null".into();
    }
    unsafe {
        let Some(desc) = Cf::from_create(CFCopyDescription(cf)) else {
            return String::new();
        };
        cf_string_value(desc.as_ptr()).unwrap_or_default()
    }
}

/// Short value text for CFString, CFBoolean, and CFNumber; the CF description
/// otherwise.
///
/// # Safety
/// `cf` is null or a valid CF object.
pub unsafe fn cf_value(cf: CFTypeRef) -> String {
    if cf.is_null() {
        return "absent".into();
    }
    unsafe {
        let t = CFGetTypeID(cf);
        if t == CFBooleanGetTypeID() {
            return (CFBooleanGetValue(cf) != 0).to_string();
        }
        if t == CFNumberGetTypeID() {
            let mut v = 0i64;
            CFNumberGetValue(cf, kCFNumberSInt64Type, &mut v as *mut i64 as *mut c_void);
            return v.to_string();
        }
    }
    unsafe { cf_string_value(cf).unwrap_or_else(|| cf_describe(cf)) }
}

/// # Safety
/// `s` is null or a valid CF object.
pub unsafe fn cf_string_value(s: CFTypeRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    unsafe {
        if CFGetTypeID(s) != CFStringGetTypeID() {
            return None;
        }
        let mut buf = vec![0 as c_char; 4096];
        if CFStringGetCString(
            s,
            buf.as_mut_ptr(),
            buf.len() as isize,
            kCFStringEncodingUTF8,
        ) == 0
        {
            return None;
        }
        Some(
            std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .into_owned(),
        )
    }
}

pub fn fourcc(v: u32) -> String {
    let b = v.to_be_bytes();
    if b.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
        String::from_utf8_lossy(&b).into_owned()
    } else {
        format!("0x{v:08x}")
    }
}

pub const fn fcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

/// Reads a CFNumber as i64.
///
/// # Safety
/// `cf` is null or a valid CF object.
pub unsafe fn cf_i64(cf: CFTypeRef) -> Option<i64> {
    if cf.is_null() || unsafe { CFGetTypeID(cf) != CFNumberGetTypeID() } {
        return None;
    }
    let mut v = 0i64;
    (unsafe { CFNumberGetValue(cf, kCFNumberSInt64Type, &mut v as *mut i64 as *mut c_void) } != 0)
        .then_some(v)
}

/// Reads a CFBoolean.
///
/// # Safety
/// `cf` is null or a valid CF object.
pub unsafe fn cf_bool_value(cf: CFTypeRef) -> Option<bool> {
    if cf.is_null() || unsafe { CFGetTypeID(cf) != CFBooleanGetTypeID() } {
        return None;
    }
    Some(unsafe { CFBooleanGetValue(cf) } != 0)
}

/// Looks up a string key in a CFDictionary (+0 result).
///
/// # Safety
/// `dict` is null or a valid CFDictionary.
pub unsafe fn cf_dict_get(dict: CFTypeRef, key: &str) -> CFTypeRef {
    if dict.is_null() || unsafe { CFGetTypeID(dict) != CFDictionaryGetTypeID() } {
        return std::ptr::null();
    }
    let k = cf_str(key);
    unsafe { CFDictionaryGetValue(dict, k.as_ptr()) }
}

/// The bytes of a CFData (+0 input).
///
/// # Safety
/// `cf` is null or a valid CF object.
pub unsafe fn cf_data_bytes(cf: CFTypeRef) -> Option<Vec<u8>> {
    if cf.is_null() || unsafe { CFGetTypeID(cf) != CFDataGetTypeID() } {
        return None;
    }
    let n = unsafe { CFDataGetLength(cf) } as usize;
    let p = unsafe { CFDataGetBytePtr(cf) };
    Some(if n == 0 || p.is_null() {
        vec![]
    } else {
        unsafe { std::slice::from_raw_parts(p, n) }.to_vec()
    })
}

/// A file URL for a path.
pub fn cf_file_url(path: &std::path::Path) -> Option<Cf> {
    use std::os::unix::ffi::OsStrExt;
    let b = path.as_os_str().as_bytes();
    unsafe {
        Cf::from_create(CFURLCreateFromFileSystemRepresentation(
            kCFAllocatorDefault,
            b.as_ptr(),
            b.len() as isize,
            0,
        ))
    }
}
