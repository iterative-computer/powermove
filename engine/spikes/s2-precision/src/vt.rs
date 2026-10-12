//! VideoToolbox: decode to a requested pixel format, encode from engine buffers,
//! encoder enumeration.

#![allow(deprecated)]

use std::path::Path;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFType};
use objc2_core_media::{CMFormatDescription, CMSampleBuffer, CMTime, CMVideoCodecType};
use objc2_core_video::CVImageBuffer;
use objc2_foundation::{NSArray, NSDictionary, NSString};
use objc2_video_toolbox::*;

use crate::avio;
use crate::cf::{self, Result, check, err};
use crate::pixbuf::{self, PixelBuffer};

/// A decoded frame with its presentation time (in the track timescale).
pub struct Frame {
    pub pts: CMTime,
    pub buffer: PixelBuffer,
}

pub struct DecodeInfo {
    pub frames: Vec<Frame>,
    pub hardware: Option<bool>,
    pub codec: u32,
}

fn session_bool(session: &CFType, key: &CFString) -> Option<bool> {
    let mut out: *const CFType = std::ptr::null();
    let st = unsafe { VTSessionCopyProperty(session, key, None, &mut out as *mut _ as *mut std::ffi::c_void) };
    if st != 0 || out.is_null() {
        return None;
    }
    let v = unsafe { CFRetained::from_raw(NonNull::new_unchecked(out as *mut CFType)) };
    Some(cf::describe_cf(&v) == "1")
}

/// Decode every frame of the first video track to `format` with VTDecompressionSession,
/// sorted by presentation time.
pub fn decode_file(path: &Path, format: u32) -> Result<DecodeInfo> {
    decode_file_as(path, Some(format))
}

/// Decode to the decoder's native output format (no pixel format requested).
pub fn decode_file_native(path: &Path) -> Result<DecodeInfo> {
    decode_file_as(path, None)
}

fn decode_file_as(path: &Path, format: Option<u32>) -> Result<DecodeInfo> {
    let samples = avio::read_compressed(path)?;
    let first = samples.first().ok_or_else(|| cf::Error("no samples".into()))?;
    let fd = unsafe { first.format_description() }.ok_or_else(|| cf::Error("no format description".into()))?;
    let codec = unsafe { fd.media_sub_type() };
    let attrs = pixbuf::surface_attrs(format);
    let spec = cf::Dict::new().build();
    let mut sess_ptr: *mut VTDecompressionSession = std::ptr::null_mut();
    let st = unsafe {
        VTDecompressionSession::create(
            None,
            &*(&*fd as *const CMFormatDescription as *const objc2_core_media::CMVideoFormatDescription),
            Some(cf::cf_dict(&spec)),
            Some(cf::cf_dict(&attrs)),
            std::ptr::null(),
            NonNull::from(&mut sess_ptr),
        )
    };
    check(st, &format!("VTDecompressionSessionCreate ('{}' → '{}')", cf::fourcc_str(codec), format.map(cf::fourcc_str).unwrap_or_else(|| "native".into())))?;
    let session = unsafe { CFRetained::from_raw(NonNull::new_unchecked(sess_ptr)) };
    let hardware =
        session_bool(&session, unsafe { kVTDecompressionPropertyKey_UsingHardwareAcceleratedVideoDecoder });

    let frames: Arc<Mutex<Vec<Frame>>> = Arc::new(Mutex::new(vec![]));
    let errors: Arc<Mutex<Vec<i32>>> = Arc::new(Mutex::new(vec![]));
    for sb in &samples {
        let f2 = frames.clone();
        let e2 = errors.clone();
        let handler = RcBlock::new(
            move |status: i32, _flags: VTDecodeInfoFlags, img: *mut CVImageBuffer, pts: CMTime, _dur: CMTime| {
                if status != 0 || img.is_null() {
                    e2.lock().unwrap().push(status);
                    return;
                }
                let pb = unsafe { PixelBuffer::retain(NonNull::new_unchecked(img)) };
                f2.lock().unwrap().push(Frame { pts, buffer: pb });
            },
        );
        let st = unsafe {
            session.decode_frame_with_output_handler(
                sb,
                VTDecodeFrameFlags(0),
                std::ptr::null_mut(),
                &*handler as *const _ as *mut _,
            )
        };
        check(st, "VTDecompressionSessionDecodeFrame")?;
    }
    unsafe {
        session.finish_delayed_frames();
        session.wait_for_asynchronous_frames();
        session.invalidate();
    }
    let errs = errors.lock().unwrap().clone();
    if !errs.is_empty() {
        return err(format!("decode errors: {errs:?}"));
    }
    let mut frames = std::mem::take(&mut *frames.lock().unwrap());
    frames.sort_by_key(|f| (f.pts.value as i128) * 1_000_000_000 / f.pts.timescale.max(1) as i128);
    Ok(DecodeInfo { frames, hardware, codec })
}

/// Encoder settings for `encode_to_file`.
pub struct EncodeSpec<'a> {
    pub codec: CMVideoCodecType,
    pub width: usize,
    pub height: usize,
    pub timescale: i32,
    pub frame_duration: i64,
    /// Session properties set before the first frame (key, value).
    pub properties: Vec<(&'a CFString, Retained<AnyObject>)>,
    pub encoder_id: Option<&'a str>,
}

/// Encode buffers with VTCompressionSession and mux with AVAssetWriter pass-through.
/// Returns the format description of the encoded stream.
pub fn encode_to_file(
    path: &Path,
    file_type: &objc2_av_foundation::AVFileType,
    spec: &EncodeSpec,
    frames: &[&PixelBuffer],
) -> Result<CFRetained<CMFormatDescription>> {
    let samples = encode_samples(spec, frames)?;
    mux(path, file_type, &samples)
}

/// Mux compressed samples with AVAssetWriter pass-through; returns the first format description.
pub fn mux(
    path: &Path,
    file_type: &objc2_av_foundation::AVFileType,
    samples: &[CFRetained<CMSampleBuffer>],
) -> Result<CFRetained<CMFormatDescription>> {
    let fd = unsafe { samples[0].format_description() }.ok_or_else(|| cf::Error("no format description".into()))?;
    let timescale = unsafe { samples[0].presentation_time_stamp() }.timescale;
    let mut w = avio::VideoWriter::passthrough(path, file_type, &fd, timescale)?;
    for sb in samples {
        w.append_sample(sb)?;
    }
    w.finish()?;
    Ok(fd)
}

/// Create a compression session for `spec` with its properties set and frames prepared.
pub fn create_session(spec: &EncodeSpec) -> Result<CFRetained<VTCompressionSession>> {
    let enc_spec = spec.encoder_id.map(|id| {
        cf::Dict::new()
            .cfkey(unsafe { kVTVideoEncoderSpecification_EncoderID }, &NSString::from_str(id))
            .build()
    });
    let mut sess_ptr: *mut VTCompressionSession = std::ptr::null_mut();
    let st = unsafe {
        VTCompressionSession::create(
            None,
            spec.width as i32,
            spec.height as i32,
            spec.codec,
            enc_spec.as_deref().map(cf::cf_dict),
            None,
            None,
            None,
            std::ptr::null_mut(),
            NonNull::from(&mut sess_ptr),
        )
    };
    check(st, &format!("VTCompressionSessionCreate '{}'", cf::fourcc_str(spec.codec)))?;
    let session = unsafe { CFRetained::from_raw(NonNull::new_unchecked(sess_ptr)) };
    for (k, v) in &spec.properties {
        let st = unsafe { VTSessionSetProperty(&session, k, Some(&*(&**v as *const AnyObject as *const CFType))) };
        check(st, &format!("VTSessionSetProperty {}", cf::ns(k)))?;
    }
    unsafe { VTCompressionSession::prepare_to_encode_frames(&session) };
    Ok(session)
}

/// The pixel buffer attributes of the session's pool: the encoder's native input formats.
pub fn native_input_attributes(spec: &EncodeSpec) -> Result<Vec<(String, String)>> {
    let session = create_session(spec)?;
    let pool = unsafe { session.pixel_buffer_pool() }.ok_or_else(|| cf::Error("no pixel buffer pool".into()))?;
    let attrs = pool.pixel_buffer_attributes().ok_or_else(|| cf::Error("no pool attributes".into()))?;
    unsafe { session.invalidate() };
    Ok(pixbuf::dict_pairs(&attrs))
}

/// Encode buffers with VTCompressionSession; compressed samples in decode order.
pub fn encode_samples(spec: &EncodeSpec, frames: &[&PixelBuffer]) -> Result<Vec<CFRetained<CMSampleBuffer>>> {
    let session = create_session(spec)?;
    let out: Arc<Mutex<Vec<CFRetained<CMSampleBuffer>>>> = Arc::new(Mutex::new(vec![]));
    let errors: Arc<Mutex<Vec<i32>>> = Arc::new(Mutex::new(vec![]));
    for (i, pb) in frames.iter().enumerate() {
        let o2 = out.clone();
        let e2 = errors.clone();
        let handler = RcBlock::new(move |status: i32, _flags: VTEncodeInfoFlags, sb: *mut CMSampleBuffer| {
            if status != 0 || sb.is_null() {
                e2.lock().unwrap().push(status);
                return;
            }
            o2.lock().unwrap().push(unsafe { CFRetained::retain(NonNull::new_unchecked(sb)) });
        });
        let pts = avio::time(i as i64 * spec.frame_duration, spec.timescale);
        let dur = avio::time(spec.frame_duration, spec.timescale);
        let st = unsafe {
            session.encode_frame_with_output_handler(
                pb.as_cf(),
                pts,
                dur,
                None,
                std::ptr::null_mut(),
                &*handler as *const _ as *mut _,
            )
        };
        check(st, "VTCompressionSessionEncodeFrame")?;
    }
    let st = unsafe { session.complete_frames(objc2_core_media::kCMTimeInvalid) };
    check(st, "VTCompressionSessionCompleteFrames")?;
    unsafe { session.invalidate() };
    let errs = errors.lock().unwrap().clone();
    if !errs.is_empty() {
        return err(format!("encode errors: {errs:?}"));
    }
    let samples = std::mem::take(&mut *out.lock().unwrap());
    if samples.is_empty() {
        return err("encoder produced no samples");
    }
    Ok(samples)
}

/// One entry of VTCopyVideoEncoderList.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EncoderEntry {
    pub codec_type: String,
    pub codec_name: String,
    pub encoder_id: String,
    pub encoder_name: String,
    pub hardware: Option<bool>,
    pub fields: Vec<(String, String)>,
}

pub fn encoder_list() -> Result<Vec<EncoderEntry>> {
    let mut arr: *const CFArray = std::ptr::null();
    let st = unsafe { VTCopyVideoEncoderList(None, NonNull::from(&mut arr)) };
    check(st, "VTCopyVideoEncoderList")?;
    let arr: Retained<NSArray<NSDictionary<NSString, AnyObject>>> =
        unsafe { Retained::from_raw(arr as *mut NSArray<NSDictionary<NSString, AnyObject>>).unwrap() };
    let mut out = vec![];
    for d in arr.iter() {
        let get = |k: &CFString| cf::dict_get_string(&d, k).unwrap_or_default();
        let ct: u32 = get(unsafe { kVTVideoEncoderList_CodecType }).parse().unwrap_or(0);
        let hw = cf::dict_get_string(&d, unsafe { kVTVideoEncoderList_IsHardwareAccelerated }).map(|s| s == "1");
        out.push(EncoderEntry {
            codec_type: cf::fourcc_str(ct),
            codec_name: get(unsafe { kVTVideoEncoderList_CodecName }),
            encoder_id: get(unsafe { kVTVideoEncoderList_EncoderID }),
            encoder_name: get(unsafe { kVTVideoEncoderList_EncoderName }),
            hardware: hw,
            fields: pixbuf::dict_pairs(unsafe { &*(&*d as *const NSDictionary<NSString, AnyObject> as *const CFDictionary) }),
        });
    }
    Ok(out)
}

/// Supported properties of the encoder VideoToolbox picks for a codec.
pub fn supported_properties(codec: u32, width: i32, height: i32) -> Result<(String, Vec<(String, String)>)> {
    let mut id: *const CFString = std::ptr::null();
    let mut props: *const CFDictionary = std::ptr::null();
    let st = unsafe { VTCopySupportedPropertyDictionaryForEncoder(width, height, codec, None, &mut id, &mut props) };
    check(st, "VTCopySupportedPropertyDictionaryForEncoder")?;
    let id = unsafe { CFRetained::from_raw(NonNull::new_unchecked(id as *mut CFString)) };
    let props = unsafe { CFRetained::from_raw(NonNull::new_unchecked(props as *mut CFDictionary)) };
    Ok((id.to_string(), pixbuf::dict_pairs(&props)))
}

pub fn cf_true() -> Retained<AnyObject> {
    let b: &CFBoolean = unsafe { objc2_core_foundation::kCFBooleanTrue.unwrap() };
    unsafe { Retained::retain(b as *const CFBoolean as *mut AnyObject).unwrap() }
}

pub fn cf_str(s: &CFString) -> Retained<AnyObject> {
    unsafe { Retained::retain(s as *const CFString as *mut AnyObject).unwrap() }
}

/// Convert `src` to `format` with VTPixelTransferSession (the converter VideoToolbox
/// uses between decoder or encoder native formats and the client's format).
pub fn transfer(src: &PixelBuffer, format: u32) -> Result<PixelBuffer> {
    let dst = PixelBuffer::new(src.width(), src.height(), format)?;
    let mut sess: *mut VTPixelTransferSession = std::ptr::null_mut();
    let st = unsafe { VTPixelTransferSession::create(None, NonNull::from(&mut sess)) };
    check(st, "VTPixelTransferSessionCreate")?;
    let session = unsafe { CFRetained::from_raw(NonNull::new_unchecked(sess)) };
    let st = unsafe { session.transfer_image(src.as_cf(), dst.as_cf()) };
    unsafe { session.invalidate() };
    check(st, &format!("VTPixelTransferSessionTransferImage '{}' → '{}'", cf::fourcc_str(src.format()), cf::fourcc_str(format)))?;
    Ok(dst)
}

/// Read session properties by key name after the session is prepared (descriptions).
pub fn session_properties(spec: &EncodeSpec, keys: &[&str]) -> Result<Vec<(String, String)>> {
    let session = create_session(spec)?;
    let mut out = vec![];
    for k in keys {
        let key = CFString::from_str(k);
        let mut v: *const CFType = std::ptr::null();
        let st = unsafe { VTSessionCopyProperty(&session, &key, None, &mut v as *mut _ as *mut std::ffi::c_void) };
        let desc = if st != 0 {
            format!("error {st}")
        } else if v.is_null() {
            "null".into()
        } else {
            let r = unsafe { CFRetained::from_raw(NonNull::new_unchecked(v as *mut CFType)) };
            cf::describe_cf(&r).split_whitespace().collect::<Vec<_>>().join(" ")
        };
        out.push((k.to_string(), desc));
    }
    unsafe { session.invalidate() };
    Ok(out)
}
