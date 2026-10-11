//! IOSurface-backed CVPixelBuffers: creation, plane access, color attachments.

#![allow(deprecated)]

use std::ptr::NonNull;

use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
use objc2_core_video::*;
use objc2_foundation::{NSDictionary, NSNumber, NSString};

use crate::cf::{self, Dict, Result, err, ns};

pub const Y416: u32 = kCVPixelFormatType_4444AYpCbCr16; // 'y416'
pub const X422: u32 = kCVPixelFormatType_422YpCbCr10BiPlanarVideoRange; // 'x422'
pub const X420: u32 = kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange; // 'x420'
pub const XF20: u32 = kCVPixelFormatType_420YpCbCr10BiPlanarFullRange; // 'xf20'
pub const V420: u32 = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange; // '420v'
pub const F420: u32 = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange; // '420f'
pub const BGRA: u32 = kCVPixelFormatType_32BGRA;
pub const RGBA_HALF: u32 = kCVPixelFormatType_64RGBAHalf; // 'RGhA'
pub const RGBA64: u32 = kCVPixelFormatType_64RGBALE; // 'l64r'
pub const B64A: u32 = kCVPixelFormatType_64ARGB; // 'b64a' (big-endian)

/// A retained CVPixelBuffer.
pub struct PixelBuffer(pub CFRetained<CVPixelBuffer>);

unsafe impl Send for PixelBuffer {}
unsafe impl Sync for PixelBuffer {}

/// Attributes requesting IOSurface backing and Metal compatibility.
pub fn surface_attrs(format: Option<u32>) -> objc2::rc::Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> {
    let empty = Dict::new().build();
    let mut d = Dict::new()
        .cfkey(unsafe { kCVPixelBufferIOSurfacePropertiesKey }, &empty)
        .cfkey(unsafe { kCVPixelBufferMetalCompatibilityKey }, &NSNumber::new_bool(true));
    if let Some(f) = format {
        d = d.cfkey(unsafe { kCVPixelBufferPixelFormatTypeKey }, &NSNumber::new_u32(f));
    }
    d.build()
}

impl PixelBuffer {
    pub fn new(width: usize, height: usize, format: u32) -> Result<Self> {
        let attrs = surface_attrs(None);
        let mut out: *mut CVPixelBuffer = std::ptr::null_mut();
        let r = unsafe {
            CVPixelBufferCreate(None, width, height, format, Some(cf::cf_dict(&attrs)), NonNull::from(&mut out))
        };
        if r != 0 || out.is_null() {
            return err(format!("CVPixelBufferCreate {}x{} '{}' failed: {r}", width, height, cf::fourcc_str(format)));
        }
        Ok(Self(unsafe { CFRetained::from_raw(NonNull::new_unchecked(out)) }))
    }

    /// Wrap a buffer handed to us by VideoToolbox or AVFoundation (retains it).
    ///
    /// # Safety
    /// `p` must be a valid CVPixelBuffer.
    pub unsafe fn retain(p: NonNull<CVPixelBuffer>) -> Self {
        Self(unsafe { CFRetained::retain(p) })
    }

    pub fn width(&self) -> usize {
        CVPixelBufferGetWidth(&self.0)
    }

    pub fn height(&self) -> usize {
        CVPixelBufferGetHeight(&self.0)
    }

    pub fn format(&self) -> u32 {
        CVPixelBufferGetPixelFormatType(&self.0)
    }

    pub fn plane_count(&self) -> usize {
        CVPixelBufferGetPlaneCount(&self.0)
    }

    pub fn is_iosurface_backed(&self) -> bool {
        CVPixelBufferGetIOSurface(Some(&self.0)).is_some()
    }

    /// Copy a plane out as u16 samples (row-major, tightly packed).
    /// Plane `None` means a non-planar buffer.
    pub fn read_u16(&self, plane: Option<usize>) -> Result<(Vec<u16>, usize, usize)> {
        self.with_lock(true, |pb| {
            let (base, bpr, w, h) = plane_geom(pb, plane);
            if base.is_null() {
                return err("null base address");
            }
            let row_u16 = bpr / 2;
            let mut out = Vec::with_capacity(w * h);
            // Samples per row: width × components.
            let comps = row_samples(pb, plane, w);
            for y in 0..h {
                let row = unsafe { std::slice::from_raw_parts((base as *const u16).add(y * row_u16), comps) };
                out.extend_from_slice(row);
            }
            Ok((out, comps, h))
        })
    }

    pub fn read_u8(&self, plane: Option<usize>) -> Result<(Vec<u8>, usize, usize)> {
        self.with_lock(true, |pb| {
            let (base, bpr, w, h) = plane_geom(pb, plane);
            let comps = row_samples(pb, plane, w);
            let mut out = Vec::with_capacity(comps * h);
            for y in 0..h {
                let row = unsafe { std::slice::from_raw_parts((base as *const u8).add(y * bpr), comps) };
                out.extend_from_slice(row);
            }
            Ok((out, comps, h))
        })
    }

    /// Write a plane from tightly packed u16 samples.
    pub fn write_u16(&self, plane: Option<usize>, data: &[u16]) -> Result<()> {
        self.with_lock(false, |pb| {
            let (base, bpr, w, h) = plane_geom(pb, plane);
            let comps = row_samples(pb, plane, w);
            if data.len() != comps * h {
                return err(format!("write_u16: expected {} samples, got {}", comps * h, data.len()));
            }
            for y in 0..h {
                let row = unsafe { std::slice::from_raw_parts_mut((base as *mut u8).add(y * bpr) as *mut u16, comps) };
                row.copy_from_slice(&data[y * comps..(y + 1) * comps]);
            }
            Ok(())
        })
    }

    pub fn write_u8(&self, plane: Option<usize>, data: &[u8]) -> Result<()> {
        self.with_lock(false, |pb| {
            let (base, bpr, w, h) = plane_geom(pb, plane);
            let comps = row_samples(pb, plane, w);
            if data.len() != comps * h {
                return err(format!("write_u8: expected {} samples, got {}", comps * h, data.len()));
            }
            for y in 0..h {
                let row = unsafe { std::slice::from_raw_parts_mut((base as *mut u8).add(y * bpr), comps) };
                row.copy_from_slice(&data[y * comps..(y + 1) * comps]);
            }
            Ok(())
        })
    }

    fn with_lock<T>(&self, read_only: bool, f: impl FnOnce(&CVPixelBuffer) -> Result<T>) -> Result<T> {
        let flags = if read_only { CVPixelBufferLockFlags::ReadOnly } else { CVPixelBufferLockFlags(0) };
        let r = unsafe { CVPixelBufferLockBaseAddress(&self.0, flags) };
        if r != 0 {
            return err(format!("CVPixelBufferLockBaseAddress failed: {r}"));
        }
        let out = f(&self.0);
        unsafe { CVPixelBufferUnlockBaseAddress(&self.0, flags) };
        out
    }

    /// Set a propagated attachment (color tags).
    pub fn set_attachment(&self, key: &CFString, value: &CFType) {
        unsafe { CVBufferSetAttachment(&self.0, key, value, CVAttachmentMode::ShouldPropagate) };
    }

    /// Set Rec.709 Y′CbCr tags (1-1-1).
    pub fn tag_709(&self) {
        unsafe {
            let p: &CFType = kCVImageBufferColorPrimaries_ITU_R_709_2.as_ref();
            self.set_attachment(kCVImageBufferColorPrimariesKey, p);
            self.set_attachment(kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_ITU_R_709_2.as_ref());
            self.set_attachment(kCVImageBufferYCbCrMatrixKey, kCVImageBufferYCbCrMatrix_ITU_R_709_2.as_ref());
        }
    }

    /// Propagated attachments, described as key → value strings.
    pub fn attachments(&self) -> Vec<(String, String)> {
        let Some(d) = CVBufferCopyAttachments(&self.0, CVAttachmentMode::ShouldPropagate) else {
            return vec![];
        };
        dict_pairs(&d)
    }

    pub fn as_cf(&self) -> &CVPixelBuffer {
        &self.0
    }
}

/// Describe all key/value pairs of a CFDictionary.
pub fn dict_pairs(d: &CFDictionary) -> Vec<(String, String)> {
    let nd: &NSDictionary<NSString, objc2::runtime::AnyObject> =
        unsafe { &*(d as *const CFDictionary as *const NSDictionary<NSString, objc2::runtime::AnyObject>) };
    let mut out = vec![];
    for k in nd.allKeys().iter() {
        if let Some(v) = nd.objectForKey(&k) {
            out.push((k.to_string(), cf::describe(&v).replace('\n', " ")));
        }
    }
    out.sort();
    out
}

fn plane_geom(pb: &CVPixelBuffer, plane: Option<usize>) -> (*mut std::ffi::c_void, usize, usize, usize) {
    match plane {
        Some(p) => (
            CVPixelBufferGetBaseAddressOfPlane(pb, p),
            CVPixelBufferGetBytesPerRowOfPlane(pb, p),
            CVPixelBufferGetWidthOfPlane(pb, p),
            CVPixelBufferGetHeightOfPlane(pb, p),
        ),
        None => (
            CVPixelBufferGetBaseAddress(pb),
            CVPixelBufferGetBytesPerRow(pb),
            CVPixelBufferGetWidth(pb),
            CVPixelBufferGetHeight(pb),
        ),
    }
}

/// Number of samples (u16 or u8 units) per row for the formats this spike uses.
fn row_samples(pb: &CVPixelBuffer, plane: Option<usize>, w: usize) -> usize {
    let f = CVPixelBufferGetPixelFormatType(pb);
    match (f, plane) {
        (Y416, _) | (RGBA_HALF, _) | (RGBA64, _) | (B64A, _) | (BGRA, _) => w * 4,
        (X422 | X420 | XF20 | V420 | F420, Some(0)) => w,
        (X422 | X420 | XF20 | V420 | F420, Some(1)) => w * 2,
        _ => w * 4,
    }
}

/// Helper used by `ns` consumers: the key string of a CF constant.
pub fn key_name(k: &CFString) -> String {
    ns(k).to_string()
}

/// Test-only uncompressed sink: write every plane tightly packed, little-endian, in
/// plane order (`y416`: AYCbCr u16; `x422`/`x420`: Y′ u16 then CbCr u16; `420v`: u8).
pub fn write_raw_sink(pb: &PixelBuffer, path: &std::path::Path) -> Result<u64> {
    let mut bytes: Vec<u8> = vec![];
    let planes: Vec<Option<usize>> = if pb.plane_count() == 0 { vec![None] } else { (0..pb.plane_count()).map(Some).collect() };
    for plane in planes {
        if pb.format() == V420 || pb.format() == F420 {
            bytes.extend(pb.read_u8(plane)?.0);
        } else {
            bytes.extend(pb.read_u16(plane)?.0.iter().flat_map(|v| v.to_le_bytes()));
        }
    }
    std::fs::write(path, &bytes).map_err(|e| cf::Error(format!("{}: {e}", path.display())))?;
    Ok(bytes.len() as u64)
}

/// Read a raw sink written by `write_raw_sink` back into a buffer of the same geometry.
pub fn read_raw_sink(path: &std::path::Path, width: usize, height: usize, format: u32) -> Result<PixelBuffer> {
    let bytes = std::fs::read(path).map_err(|e| cf::Error(format!("{}: {e}", path.display())))?;
    let pb = PixelBuffer::new(width, height, format)?;
    let planes: Vec<Option<usize>> = if pb.plane_count() == 0 { vec![None] } else { (0..pb.plane_count()).map(Some).collect() };
    let mut off = 0usize;
    for plane in planes {
        if format == V420 || format == F420 {
            let n = pb.read_u8(plane)?.0.len();
            pb.write_u8(plane, &bytes[off..off + n])?;
            off += n;
        } else {
            let n = pb.read_u16(plane)?.0.len();
            let v: Vec<u16> = bytes[off..off + 2 * n].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            pb.write_u16(plane, &v)?;
            off += 2 * n;
        }
    }
    if off != bytes.len() {
        return err(format!("{}: {} trailing bytes", path.display(), bytes.len() - off));
    }
    Ok(pb)
}
