//! A single immutable source-video frame in an IOSurface-backed CoreVideo buffer.
//!
//! Electron main owns this object until the imported shared texture reports
//! `allReferencesReleased`. The pointer returned to JavaScript is borrowed;
//! dropping or releasing this object invalidates it. A fresh buffer is made
//! for every frame, so a later decode never overwrites Chromium's readers.

use std::ffi::c_void;
use std::ptr::{self, NonNull};

use napi::bindgen_prelude::*;
use napi_derive::napi;
use objc2_core_foundation::{CFDictionary, CFString, CFType};

const MAX_DIMENSION: u32 = 16_384;
const MAX_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferIOSurfacePropertiesKey: &'static CFString;
    static kCVPixelBufferMetalCompatibilityKey: &'static CFString;
    fn CVPixelBufferCreate(
        allocator: *const c_void,
        width: usize,
        height: usize,
        pixel_format: u32,
        attributes: *const c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn CVPixelBufferGetIOSurface(buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetPlaneCount(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetWidthOfPlane(buffer: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferGetHeightOfPlane(buffer: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferGetBytesPerRowOfPlane(buffer: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferGetBaseAddressOfPlane(buffer: *mut c_void, plane: usize) -> *mut c_void;
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
}

#[link(name = "IOSurface", kind = "framework")]
unsafe extern "C" {
    fn IOSurfaceGetAllocSize(surface: *mut c_void) -> usize;
}

fn invalid(why: impl std::fmt::Display) -> Error {
    Error::from_reason(why.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PackedFormat {
    Nv12,
    Nv16,
    P010Le,
}

impl PackedFormat {
    fn parse(format: &str) -> Result<Self> {
        match format {
            "nv12" => Ok(Self::Nv12),
            "nv16" => Ok(Self::Nv16),
            "p010le" => Ok(Self::P010Le),
            _ => Err(invalid(format!(
                "unsupported source-video format {format:?}"
            ))),
        }
    }

    fn fourcc(self, full_range: bool) -> u32 {
        let code = match (self, full_range) {
            (Self::Nv12, false) => *b"420v",
            (Self::Nv12, true) => *b"420f",
            (Self::Nv16, false) => *b"422v",
            (Self::Nv16, true) => *b"422f",
            (Self::P010Le, false) => *b"x420",
            (Self::P010Le, true) => *b"xf20",
        };
        u32::from_be_bytes(code)
    }
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    width: usize,
    rows: usize,
    row_bytes: usize,
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    y: Plane,
    uv: Plane,
    payload_bytes: usize,
}

impl Layout {
    fn new(width: u32, height: u32, format: PackedFormat) -> Result<Self> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(invalid(format!(
                "source-video dimensions must be 1..={MAX_DIMENSION}: {width}×{height}"
            )));
        }
        let w = width as usize;
        let h = height as usize;
        let chroma_width = w.div_ceil(2);
        let chroma_height = if format == PackedFormat::Nv16 {
            h
        } else {
            h.div_ceil(2)
        };
        let sample_bytes = if format == PackedFormat::P010Le { 2 } else { 1 };
        let y_row = w
            .checked_mul(sample_bytes)
            .ok_or_else(|| invalid("source-video luma row overflows"))?;
        let uv_row = chroma_width
            .checked_mul(2 * sample_bytes)
            .ok_or_else(|| invalid("source-video chroma row overflows"))?;
        let payload_bytes = y_row
            .checked_mul(h)
            .and_then(|bytes| uv_row.checked_mul(chroma_height)?.checked_add(bytes))
            .ok_or_else(|| invalid("source-video payload overflows"))?;
        if payload_bytes > MAX_PAYLOAD_BYTES {
            return Err(invalid(format!(
                "source-video payload {payload_bytes} exceeds {MAX_PAYLOAD_BYTES} bytes"
            )));
        }
        Ok(Self {
            y: Plane {
                width: w,
                rows: h,
                row_bytes: y_row,
            },
            uv: Plane {
                width: chroma_width,
                rows: chroma_height,
                row_bytes: uv_row,
            },
            payload_bytes,
        })
    }

    fn validate_payload(self, bytes: &[u8]) -> Result<()> {
        if bytes.len() != self.payload_bytes {
            return Err(invalid(format!(
                "source-video payload has {} bytes; expected {}",
                bytes.len(),
                self.payload_bytes
            )));
        }
        Ok(())
    }
}

/// The +1 reference returned by CVPixelBufferCreate. CoreVideo buffers are
/// reference counted and may move across threads, although methods here run
/// synchronously on Electron main's JavaScript thread.
struct RetainedPixelBuffer(NonNull<c_void>);
unsafe impl Send for RetainedPixelBuffer {}

impl Drop for RetainedPixelBuffer {
    fn drop(&mut self) {
        // SAFETY: this is the +1 reference returned by CVPixelBufferCreate.
        unsafe { CFRelease(self.0.as_ptr()) };
    }
}

struct LockedBuffer(NonNull<c_void>, bool);

impl LockedBuffer {
    fn new(buffer: NonNull<c_void>) -> Result<Self> {
        // SAFETY: `buffer` is a live CVPixelBuffer and zero means read/write.
        let status = unsafe { CVPixelBufferLockBaseAddress(buffer.as_ptr(), 0) };
        if status != 0 {
            return Err(invalid(format!("CVPixelBufferLockBaseAddress: {status}")));
        }
        Ok(Self(buffer, true))
    }

    fn unlock(mut self) -> Result<()> {
        // SAFETY: the successful lock above is paired exactly once here.
        let status = unsafe { CVPixelBufferUnlockBaseAddress(self.0.as_ptr(), 0) };
        self.1 = false;
        if status != 0 {
            return Err(invalid(format!("CVPixelBufferUnlockBaseAddress: {status}")));
        }
        Ok(())
    }
}

impl Drop for LockedBuffer {
    fn drop(&mut self) {
        if self.1 {
            // SAFETY: error paths still release the successful lock.
            unsafe { CVPixelBufferUnlockBaseAddress(self.0.as_ptr(), 0) };
        }
    }
}

fn copy_plane(
    buffer: NonNull<c_void>,
    plane_index: usize,
    layout: Plane,
    bytes: &[u8],
) -> Result<()> {
    // SAFETY: CoreVideo plane queries are valid while the live buffer is locked.
    let (width, rows, stride, base) = unsafe {
        (
            CVPixelBufferGetWidthOfPlane(buffer.as_ptr(), plane_index),
            CVPixelBufferGetHeightOfPlane(buffer.as_ptr(), plane_index),
            CVPixelBufferGetBytesPerRowOfPlane(buffer.as_ptr(), plane_index),
            CVPixelBufferGetBaseAddressOfPlane(buffer.as_ptr(), plane_index) as *mut u8,
        )
    };
    if width != layout.width || rows != layout.rows || stride < layout.row_bytes || base.is_null() {
        return Err(invalid(format!(
            "CoreVideo plane {plane_index} geometry {width}×{rows} stride {stride} cannot hold {}×{} bytes",
            layout.row_bytes, layout.rows
        )));
    }
    if bytes.len() != layout.row_bytes * layout.rows {
        return Err(invalid(format!(
            "source-video plane {plane_index} length mismatch"
        )));
    }
    for row in 0..layout.rows {
        // SAFETY: CoreVideo reports `stride >= row_bytes` and `rows` while
        // locked; `bytes` has exactly `row_bytes * rows` source bytes.
        unsafe {
            ptr::copy_nonoverlapping(
                bytes.as_ptr().add(row * layout.row_bytes),
                base.add(row * stride),
                layout.row_bytes,
            );
        }
    }
    Ok(())
}

fn create_buffer(
    width: u32,
    height: u32,
    format: PackedFormat,
    full_range: bool,
) -> Result<RetainedPixelBuffer> {
    let empty = CFDictionary::<CFType, CFType>::from_slices(&[], &[]);
    // SAFETY: CoreVideo exports these two immortal CFString keys.
    let keys: [&CFType; 2] = unsafe {
        [
            kCVPixelBufferIOSurfacePropertiesKey.as_ref(),
            kCVPixelBufferMetalCompatibilityKey.as_ref(),
        ]
    };
    let metal = unsafe { objc2_core_foundation::kCFBooleanTrue }
        .ok_or_else(|| invalid("CoreFoundation true constant unavailable"))?;
    let values: [&CFType; 2] = [empty.as_ref(), metal.as_ref()];
    let attributes = CFDictionary::<CFType, CFType>::from_slices(&keys, &values);
    let mut raw = ptr::null_mut();
    // SAFETY: attributes and out-param are live for the synchronous call.
    let status = unsafe {
        CVPixelBufferCreate(
            ptr::null(),
            width as usize,
            height as usize,
            format.fourcc(full_range),
            attributes.as_opaque() as *const _ as *const c_void,
            &mut raw,
        )
    };
    if status != 0 {
        if !raw.is_null() {
            // SAFETY: a failed CoreVideo call may still have initialized out.
            unsafe { CFRelease(raw) };
        }
        return Err(invalid(format!("CVPixelBufferCreate: {status}")));
    }
    let buffer = NonNull::new(raw).ok_or_else(|| invalid("CVPixelBufferCreate returned null"))?;
    // SAFETY: the retained pixel buffer keeps its IOSurface alive.
    if unsafe { CVPixelBufferGetIOSurface(buffer.as_ptr()) }.is_null() {
        // SAFETY: CoreVideo returned a +1 reference even without IOSurface.
        unsafe { CFRelease(buffer.as_ptr()) };
        return Err(invalid("CVPixelBufferCreate returned no IOSurface"));
    }
    Ok(RetainedPixelBuffer(buffer))
}

/// One source frame's immutable planar storage for Chromium's video import.
#[napi(js_name = "SourceVideoSurface")]
pub struct SourceVideoSurface {
    buffer: Option<RetainedPixelBuffer>,
}

#[napi]
impl SourceVideoSurface {
    #[napi(constructor)]
    pub fn new(
        width: u32,
        height: u32,
        format: String,
        bytes: Buffer,
        full_range: bool,
    ) -> Result<Self> {
        Self::filled(width, height, &format, &bytes, full_range)
    }

    /// Fills the surface straight from the engine's shared-memory segment
    /// (taken: unlinked), without first copying it into a JavaScript Buffer.
    #[napi(factory)]
    pub fn from_segment(
        width: u32,
        height: u32,
        format: String,
        name: String,
        length: f64,
        full_range: bool,
    ) -> Result<Self> {
        if !length.is_finite() || length < 1.0 || length > crate::shm::MAX_SEGMENT as f64 {
            return Err(invalid(format!("a {length}-byte source-video segment")));
        }
        let mut filled = None;
        crate::shm::take_with(&name, length as usize, |bytes| {
            filled = Some(Self::filled(width, height, &format, bytes, full_range));
            Ok(())
        })
        .map_err(|e| invalid(format!("source-video segment {name}: {e}")))?;
        filled.expect("take_with read the segment")
    }

    fn filled(
        width: u32,
        height: u32,
        format: &str,
        bytes: &[u8],
        full_range: bool,
    ) -> Result<Self> {
        let format = PackedFormat::parse(format)?;
        let layout = Layout::new(width, height, format)?;
        layout.validate_payload(bytes)?;
        let buffer = create_buffer(width, height, format, full_range)?;
        let locked = LockedBuffer::new(buffer.0)?;
        // SAFETY: the lock holds both planes' storage until unlock below.
        if unsafe { CVPixelBufferGetPlaneCount(buffer.0.as_ptr()) } != 2 {
            return Err(invalid(
                "CoreVideo source-video buffer has no two-plane layout",
            ));
        }
        let y_bytes = layout.y.row_bytes * layout.y.rows;
        copy_plane(buffer.0, 0, layout.y, &bytes[..y_bytes])?;
        copy_plane(buffer.0, 1, layout.uv, &bytes[y_bytes..])?;
        locked.unlock()?;
        Ok(Self {
            buffer: Some(buffer),
        })
    }

    /// A native-endian IOSurfaceRef pointer. Valid only until `release()` or
    /// finalization; main must retain the object through all Electron imports.
    #[napi(getter)]
    pub fn io_surface(&self) -> Result<Buffer> {
        let buffer = self
            .buffer
            .as_ref()
            .ok_or_else(|| invalid("source-video surface was released"))?;
        // SAFETY: the retained pixel buffer still owns its IOSurface.
        let surface = unsafe { CVPixelBufferGetIOSurface(buffer.0.as_ptr()) };
        let surface =
            NonNull::new(surface).ok_or_else(|| invalid("source-video IOSurface disappeared"))?;
        Ok(Buffer::from(
            (surface.as_ptr() as usize).to_ne_bytes().to_vec(),
        ))
    }

    /// Actual IOSurface allocation, including plane alignment and padding.
    /// Main uses this for its aggregate retained-surface memory cap.
    #[napi(getter)]
    pub fn alloc_bytes(&self) -> Result<f64> {
        let buffer = self
            .buffer
            .as_ref()
            .ok_or_else(|| invalid("source-video surface was released"))?;
        // SAFETY: the retained CVPixelBuffer keeps the IOSurface alive.
        let surface = unsafe { CVPixelBufferGetIOSurface(buffer.0.as_ptr()) };
        let surface =
            NonNull::new(surface).ok_or_else(|| invalid("source-video IOSurface disappeared"))?;
        // A 16384² P010 surface cannot approach f64's exact integer bound.
        Ok(unsafe { IOSurfaceGetAllocSize(surface.as_ptr()) } as f64)
    }

    /// Drops main's CVPixelBuffer reference once Chromium has released all
    /// imports. Repeated calls are harmless.
    #[napi]
    pub fn release(&mut self) {
        self.buffer.take();
    }
}

impl Drop for SourceVideoSurface {
    fn drop(&mut self) {
        self.buffer.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_layout_accepts_odd_dimensions_without_dropping_chroma() {
        let nv12 = Layout::new(3, 3, PackedFormat::Nv12).unwrap();
        assert_eq!((nv12.y.row_bytes, nv12.y.rows), (3, 3));
        assert_eq!(
            (nv12.uv.row_bytes, nv12.uv.rows, nv12.payload_bytes),
            (4, 2, 17)
        );
        let nv16 = Layout::new(3, 3, PackedFormat::Nv16).unwrap();
        assert_eq!(
            (nv16.uv.row_bytes, nv16.uv.rows, nv16.payload_bytes),
            (4, 3, 21)
        );
        let p010 = Layout::new(3, 3, PackedFormat::P010Le).unwrap();
        assert_eq!(
            (p010.y.row_bytes, p010.uv.row_bytes, p010.payload_bytes),
            (6, 8, 34)
        );
    }

    #[test]
    fn rejects_unknown_format_dimensions_and_payload_size() {
        assert!(PackedFormat::parse("rgba").is_err());
        assert!(Layout::new(0, 1, PackedFormat::Nv12).is_err());
        assert!(Layout::new(16_385, 1, PackedFormat::Nv12).is_err());
        assert!(Layout::new(16_384, 16_384, PackedFormat::Nv12).is_err());
        let layout = Layout::new(5, 3, PackedFormat::P010Le).unwrap();
        assert_eq!(layout.payload_bytes, 54);
        assert!(layout.validate_payload(&vec![0; 53]).is_err());
        assert!(layout.validate_payload(&vec![0; 54]).is_ok());
    }
}
