//! Declaring the alpha channel of a ProRes 4444 export in its format
//! description.
//!
//! VideoToolbox's ProRes encoder describes a frame whose alpha is entirely
//! opaque as 24-bit (no alpha), and AVAssetWriter takes the movie's sample
//! description from the first sample. An alpha export whose first frame is
//! opaque (a title that fades in, a matte that opens later) would therefore be
//! written as a movie without alpha, whatever its later frames hold. Measured
//! on macOS 27: `kVTCompressionPropertyKey_PreserveAlphaChannel` and
//! `kVTCompressionPropertyKey_Depth` do not change it. So samples of an alpha
//! export are re-described with depth 32 when the encoder said otherwise; the
//! coded frames are unchanged (a ProRes frame without coded alpha decodes as
//! opaque, which is what it is).

#![allow(deprecated)]

use std::ptr::NonNull;

use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFDictionary, CFRetained};
use objc2_core_media::*;
use objc2_foundation::{NSData, NSDictionary, NSNumber, NSString};

use super::cf;
use crate::Error;

/// The QuickTime depth of a video sample description with alpha.
pub const DEPTH_WITH_ALPHA: i32 = 32;

fn depth_of(ext: &CFDictionary) -> Option<i64> {
    let nd: &NSDictionary<NSString, AnyObject> =
        unsafe { &*(ext as *const CFDictionary as *const NSDictionary<NSString, AnyObject>) };
    let key = cf::ns(unsafe { kCMFormatDescriptionExtension_Depth });
    let v = nd.objectForKey(key)?;
    let n: &NSNumber = v.downcast_ref()?;
    Some(n.as_i64())
}

/// Byte offset of the depth field in a QuickTime video sample description
/// (size, format, 6 reserved, data reference index, version, revision,
/// vendor, temporal and spatial quality, width, height, resolutions, data
/// size, frame count, 32-byte compressor name).
const STSD_DEPTH_OFFSET: usize = 82;

/// A verbatim video sample description with its depth set to 32.
fn with_depth(stsd: &[u8]) -> Vec<u8> {
    let mut out = stsd.to_vec();
    if out.len() >= STSD_DEPTH_OFFSET + 2 {
        out[STSD_DEPTH_OFFSET..STSD_DEPTH_OFFSET + 2]
            .copy_from_slice(&(DEPTH_WITH_ALPHA as u16).to_be_bytes());
    }
    out
}

/// Format descriptions re-made with depth 32, cached by the encoder's own.
#[derive(Default)]
pub struct AlphaDescriptions {
    last: Option<(
        CFRetained<CMFormatDescription>,
        CFRetained<CMFormatDescription>,
    )>,
}

// The descriptions are immutable CF objects.
unsafe impl Send for AlphaDescriptions {}

impl AlphaDescriptions {
    fn described(
        &mut self,
        fd: &CFRetained<CMFormatDescription>,
    ) -> Result<Option<CFRetained<CMFormatDescription>>, Error> {
        let ext = unsafe { fd.extensions() };
        if ext.as_deref().and_then(depth_of) == Some(i64::from(DEPTH_WITH_ALPHA)) {
            return Ok(None);
        }
        if let Some((from, to)) = &self.last
            && CFRetained::as_ptr(from) == CFRetained::as_ptr(fd)
        {
            return Ok(Some(to.clone()));
        }
        // Every extension the encoder wrote (colors, field and pixel aspect,
        // the format name, vendor), with the depth set.
        let mut dict = cf::Dict::new();
        if let Some(ext) = &ext {
            let nd: &NSDictionary<NSString, AnyObject> = unsafe {
                &*(&**ext as *const CFDictionary as *const NSDictionary<NSString, AnyObject>)
            };
            let depth_key = cf::ns(unsafe { kCMFormatDescriptionExtension_Depth });
            let verbatim_key =
                cf::ns(unsafe { kCMFormatDescriptionExtension_VerbatimSampleDescription });
            for k in nd.allKeys().iter() {
                if &*k == depth_key {
                    continue;
                }
                let Some(v) = nd.objectForKey(&k) else {
                    continue;
                };
                if &*k == verbatim_key {
                    // AVAssetWriter writes this sample description as it is:
                    // its depth field must say 32 too.
                    if let Some(data) = v.downcast_ref::<NSData>() {
                        let patched = NSData::with_bytes(&with_depth(&data.to_vec()));
                        dict = dict.obj(&k, &patched);
                    }
                    continue;
                }
                dict = dict.obj(&k, &v);
            }
        }
        let dict = dict
            .i64(
                cf::ns(unsafe { kCMFormatDescriptionExtension_Depth }),
                i64::from(DEPTH_WITH_ALPHA),
            )
            .build();
        let video: &CMVideoFormatDescription =
            unsafe { &*(CFRetained::as_ptr(fd).as_ptr() as *const CMVideoFormatDescription) };
        let dims = unsafe { CMVideoFormatDescriptionGetDimensions(video) };
        let mut out: *const CMVideoFormatDescription = std::ptr::null();
        let st = unsafe {
            CMVideoFormatDescriptionCreate(
                None,
                fd.media_sub_type(),
                dims.width,
                dims.height,
                Some(cf::cf_dict(&dict)),
                NonNull::from(&mut out),
            )
        };
        cf::check(st, "CMVideoFormatDescriptionCreate (alpha)")?;
        let made = unsafe {
            CFRetained::from_raw(NonNull::new_unchecked(out as *mut CMFormatDescription))
        };
        self.last = Some((fd.clone(), made.clone()));
        Ok(Some(made))
    }

    /// `sample`, described with depth 32 (the same sample when it already is).
    pub fn sample(
        &mut self,
        sample: CFRetained<CMSampleBuffer>,
    ) -> Result<CFRetained<CMSampleBuffer>, Error> {
        let Some(fd) = (unsafe { sample.format_description() }) else {
            return Ok(sample);
        };
        let Some(described) = self.described(&fd)? else {
            return Ok(sample);
        };
        let n = unsafe { sample.num_samples() };
        let mut timing_n: CMItemCount = 0;
        cf::check(
            unsafe { sample.sample_timing_info_array(0, std::ptr::null_mut(), &mut timing_n) },
            "CMSampleBufferGetSampleTimingInfoArray",
        )?;
        let mut timing = vec![
            CMSampleTimingInfo {
                duration: unsafe { kCMTimeInvalid },
                presentationTimeStamp: unsafe { kCMTimeInvalid },
                decodeTimeStamp: unsafe { kCMTimeInvalid },
            };
            timing_n.max(0) as usize
        ];
        cf::check(
            unsafe {
                sample.sample_timing_info_array(timing_n, timing.as_mut_ptr(), std::ptr::null_mut())
            },
            "CMSampleBufferGetSampleTimingInfoArray",
        )?;
        let mut sizes_n: CMItemCount = 0;
        cf::check(
            unsafe { sample.sample_size_array(0, std::ptr::null_mut(), &mut sizes_n) },
            "CMSampleBufferGetSampleSizeArray",
        )?;
        let mut sizes = vec![0usize; sizes_n.max(0) as usize];
        cf::check(
            unsafe { sample.sample_size_array(sizes_n, sizes.as_mut_ptr(), std::ptr::null_mut()) },
            "CMSampleBufferGetSampleSizeArray",
        )?;
        let data = unsafe { sample.data_buffer() }
            .ok_or_else(|| Error::Mux("a compressed sample has no data".into()))?;
        let mut sb: *mut CMSampleBuffer = std::ptr::null_mut();
        let st = unsafe {
            CMSampleBuffer::create_ready(
                None,
                Some(&data),
                Some(&described),
                n,
                timing_n,
                if timing.is_empty() {
                    std::ptr::null()
                } else {
                    timing.as_ptr()
                },
                sizes_n,
                if sizes.is_empty() {
                    std::ptr::null()
                } else {
                    sizes.as_ptr()
                },
                NonNull::from(&mut sb),
            )
        };
        cf::check(st, "CMSampleBufferCreateReady (alpha)")?;
        Ok(unsafe { CFRetained::from_raw(NonNull::new_unchecked(sb)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verbatim_depth_field_is_set() {
        let mut stsd = vec![0u8; 118];
        stsd[4..8].copy_from_slice(b"ap4h");
        stsd[82..84].copy_from_slice(&24u16.to_be_bytes());
        stsd[84..86].copy_from_slice(&0xffffu16.to_be_bytes());
        let out = with_depth(&stsd);
        assert_eq!(&out[82..84], &32u16.to_be_bytes());
        assert_eq!(&out[84..86], &[0xff, 0xff], "the color table id is kept");
        assert_eq!(out.len(), stsd.len());
        assert_eq!(with_depth(&[1, 2, 3]), vec![1, 2, 3], "too short to patch");
    }
}
