//! IOSurface-backed CVPixelBuffers: a pool for encoder input, plane copies, color
//! attachments.

#![allow(deprecated)]

use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFRetained, CFString, CFType};
use objc2_core_video::*;
use objc2_foundation::{NSDictionary, NSNumber, NSString};

use super::cf::{self, Dict};
use crate::Error;
use crate::convert::{EncoderInput, Plane, PlaneData, plane_layout};
use crate::settings::{AlphaOutput, ColorTags, InputFormat};

/// A retained CVPixelBuffer.
pub struct PixelBuffer(pub CFRetained<CVPixelBuffer>);

// CVPixelBuffers are reference-counted CF objects that may cross threads.
unsafe impl Send for PixelBuffer {}

fn surface_attrs(
    format: Option<u32>,
    size: Option<(u32, u32)>,
) -> Retained<NSDictionary<NSString, AnyObject>> {
    let empty = Dict::new().build();
    let mut d = Dict::new()
        .cfkey(unsafe { kCVPixelBufferIOSurfacePropertiesKey }, &empty)
        .cfkey(
            unsafe { kCVPixelBufferMetalCompatibilityKey },
            &NSNumber::new_bool(true),
        );
    if let Some(f) = format {
        d = d.cfkey(
            unsafe { kCVPixelBufferPixelFormatTypeKey },
            &NSNumber::new_u32(f),
        );
    }
    if let Some((w, h)) = size {
        d = d
            .cfkey(unsafe { kCVPixelBufferWidthKey }, &NSNumber::new_u32(w))
            .cfkey(unsafe { kCVPixelBufferHeightKey }, &NSNumber::new_u32(h));
    }
    d.build()
}

/// Pixel buffer attributes requesting `format` with IOSurface backing (decoder output).
pub fn output_attributes(format: u32) -> Retained<NSDictionary<NSString, AnyObject>> {
    surface_attrs(Some(format), None)
}

/// A pool of encoder-input buffers of one format and size.
pub struct Pool {
    pool: CFRetained<CVPixelBufferPool>,
}

unsafe impl Send for Pool {}

impl Pool {
    pub fn new(format: InputFormat, width: u32, height: u32) -> Result<Self, Error> {
        let attrs = surface_attrs(Some(format.code()), Some((width, height)));
        let mut out: *mut CVPixelBufferPool = std::ptr::null_mut();
        let r = unsafe {
            CVPixelBufferPoolCreate(
                None,
                None,
                Some(cf::cf_dict(&attrs)),
                NonNull::from(&mut out),
            )
        };
        if r != 0 || out.is_null() {
            return Err(Error::Encode(format!(
                "CVPixelBufferPoolCreate '{}' {width}×{height}: {r}",
                cf::fourcc_str(format.code())
            )));
        }
        Ok(Self {
            pool: unsafe { CFRetained::from_raw(NonNull::new_unchecked(out)) },
        })
    }

    pub fn buffer(&self) -> Result<PixelBuffer, Error> {
        let mut out: *mut CVPixelBuffer = std::ptr::null_mut();
        let r = unsafe {
            CVPixelBufferPoolCreatePixelBuffer(None, &self.pool, NonNull::from(&mut out))
        };
        if r != 0 || out.is_null() {
            return Err(Error::Encode(format!(
                "CVPixelBufferPoolCreatePixelBuffer: {r}"
            )));
        }
        Ok(PixelBuffer(unsafe {
            CFRetained::from_raw(NonNull::new_unchecked(out))
        }))
    }
}

impl PixelBuffer {
    /// Wraps (retains) a buffer handed out by VideoToolbox or AVFoundation.
    ///
    /// # Safety
    /// `p` must point to a valid CVPixelBuffer.
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

    pub fn as_cf(&self) -> &CVPixelBuffer {
        &self.0
    }

    fn with_lock<T>(
        &self,
        read_only: bool,
        f: impl FnOnce(&CVPixelBuffer) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let flags = if read_only {
            CVPixelBufferLockFlags::ReadOnly
        } else {
            CVPixelBufferLockFlags(0)
        };
        let r = unsafe { CVPixelBufferLockBaseAddress(&self.0, flags) };
        if r != 0 {
            return Err(Error::Encode(format!("CVPixelBufferLockBaseAddress: {r}")));
        }
        let out = f(&self.0);
        unsafe { CVPixelBufferUnlockBaseAddress(&self.0, flags) };
        out
    }

    /// (base, bytes per row, rows) of a plane; `planar` false means the whole buffer.
    fn geometry(pb: &CVPixelBuffer, planar: bool, plane: usize) -> (*mut u8, usize, usize) {
        if planar {
            (
                CVPixelBufferGetBaseAddressOfPlane(pb, plane) as *mut u8,
                CVPixelBufferGetBytesPerRowOfPlane(pb, plane),
                CVPixelBufferGetHeightOfPlane(pb, plane),
            )
        } else {
            (
                CVPixelBufferGetBaseAddress(pb) as *mut u8,
                CVPixelBufferGetBytesPerRow(pb),
                CVPixelBufferGetHeight(pb),
            )
        }
    }

    /// Copies an encoder-input frame into this buffer (same format and size).
    pub fn write(&self, input: &EncoderInput) -> Result<(), Error> {
        if self.format() != input.format.code()
            || self.width() != input.width as usize
            || self.height() != input.height as usize
        {
            return Err(Error::InvalidFrame(
                "encoder input does not match the pool".into(),
            ));
        }
        let planar = input.planes.len() > 1;
        self.with_lock(false, |pb| {
            for (i, plane) in input.planes.iter().enumerate() {
                let (base, bpr, rows) = Self::geometry(pb, planar, i);
                let row_bytes = plane.row_bytes();
                if base.is_null() || bpr < row_bytes || rows != plane.rows {
                    return Err(Error::Encode(format!(
                        "plane {i}: {bpr} bytes per row, {rows} rows for {row_bytes} × {}",
                        plane.rows
                    )));
                }
                for y in 0..rows {
                    let dst =
                        unsafe { std::slice::from_raw_parts_mut(base.add(y * bpr), row_bytes) };
                    match &plane.data {
                        PlaneData::U16(v) => {
                            for (d, s) in dst
                                .chunks_exact_mut(2)
                                .zip(&v[y * plane.row_samples..(y + 1) * plane.row_samples])
                            {
                                d.copy_from_slice(&s.to_le_bytes());
                            }
                        }
                        PlaneData::U8(v) => dst.copy_from_slice(
                            &v[y * plane.row_samples..(y + 1) * plane.row_samples],
                        ),
                    }
                }
            }
            Ok(())
        })
    }

    /// Reads this buffer as encoder-input planes (`y416`, `v216`, or `420v`).
    pub fn read(&self, format: InputFormat) -> Result<EncoderInput, Error> {
        if self.format() != format.code() {
            return Err(Error::Verify(format!(
                "decoder delivered '{}', asked for '{}'",
                cf::fourcc_str(self.format()),
                cf::fourcc_str(format.code())
            )));
        }
        let (w, h) = (self.width() as u32, self.height() as u32);
        let layout = plane_layout(format, w, h);
        let planar = layout.len() > 1;
        self.with_lock(true, |pb| {
            let mut planes = vec![];
            for (i, &(samples, rows, wide)) in layout.iter().enumerate() {
                let (base, bpr, got_rows) = Self::geometry(pb, planar, i);
                let row_bytes = if wide { samples * 2 } else { samples };
                if base.is_null() || bpr < row_bytes || got_rows != rows {
                    return Err(Error::Verify(format!(
                        "decoded plane {i} has an unexpected layout"
                    )));
                }
                let data = if wide {
                    let mut v = Vec::with_capacity(samples * rows);
                    for y in 0..rows {
                        let row =
                            unsafe { std::slice::from_raw_parts(base.add(y * bpr), row_bytes) };
                        v.extend(
                            row.chunks_exact(2)
                                .map(|c| u16::from_le_bytes([c[0], c[1]])),
                        );
                    }
                    PlaneData::U16(v)
                } else {
                    let mut v = Vec::with_capacity(samples * rows);
                    for y in 0..rows {
                        v.extend_from_slice(unsafe {
                            std::slice::from_raw_parts(base.add(y * bpr), row_bytes)
                        });
                    }
                    PlaneData::U8(v)
                };
                planes.push(Plane {
                    data,
                    row_samples: samples,
                    rows,
                });
            }
            Ok(EncoderInput {
                format,
                width: w,
                height: h,
                planes,
            })
        })
    }

    fn attach(&self, key: &CFString, value: &CFType) {
        unsafe { CVBufferSetAttachment(&self.0, key, value, CVAttachmentMode::ShouldPropagate) };
    }

    /// Sets the color and alpha attachments the encoder sees.
    pub fn tag(&self, tags: ColorTags, alpha: AlphaOutput) {
        let (primaries, transfer, matrix) = color_constants(tags);
        unsafe {
            self.attach(kCVImageBufferColorPrimariesKey, primaries.as_ref());
            self.attach(kCVImageBufferTransferFunctionKey, transfer.as_ref());
            self.attach(kCVImageBufferYCbCrMatrixKey, matrix.as_ref());
            match alpha {
                AlphaOutput::None => {}
                AlphaOutput::Straight => self.attach(
                    kCVImageBufferAlphaChannelModeKey,
                    kCVImageBufferAlphaChannelMode_StraightAlpha.as_ref(),
                ),
                AlphaOutput::Premultiplied => self.attach(
                    kCVImageBufferAlphaChannelModeKey,
                    kCVImageBufferAlphaChannelMode_PremultipliedAlpha.as_ref(),
                ),
            }
        }
    }
}

/// CoreVideo constants for a tag set: (primaries, transfer, matrix).
pub fn color_constants(
    tags: ColorTags,
) -> (&'static CFString, &'static CFString, &'static CFString) {
    unsafe {
        match tags {
            ColorTags::Rec709Broadcast | ColorTags::Rec709Image => (
                kCVImageBufferColorPrimaries_ITU_R_709_2,
                kCVImageBufferTransferFunction_ITU_R_709_2,
                kCVImageBufferYCbCrMatrix_ITU_R_709_2,
            ),
            ColorTags::Rec709QuickTime | ColorTags::SrgbImage => (
                kCVImageBufferColorPrimaries_ITU_R_709_2,
                kCVImageBufferTransferFunction_sRGB,
                kCVImageBufferYCbCrMatrix_ITU_R_709_2,
            ),
        }
    }
}
