//! VTCompressionSession: ProRes, H.264, and HEVC (with alpha at 8 bits) encode from
//! engine encoder input, with color tags as session properties (amendment 7; HEVC
//! with alpha drops buffer attachments, S2) and properties set before the first frame.

#![allow(deprecated)]

use std::collections::VecDeque;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFRetained, CFString, CFType};
use objc2_core_media::{CMSampleBuffer, CMTime, kCMTimeInvalid};
use objc2_video_toolbox::*;

use super::cf::{self, any_bool, any_num_f64, any_num_i64, any_str};
use super::pixbuf::{PixelBuffer, color_constants};
use crate::Error;
use crate::settings::{AlphaOutput, Codec, ExportSpec};
use crate::timescale::VideoTiming;

/// kVTPropertyNotSupportedErr.
const PROPERTY_NOT_SUPPORTED: i32 = -12900;

#[derive(Default)]
struct Output {
    samples: VecDeque<CFRetained<CMSampleBuffer>>,
    error: Option<String>,
}

/// A compression session for one export.
pub struct Encoder {
    session: CFRetained<VTCompressionSession>,
    output: Arc<Mutex<Output>>,
    timing: VideoTiming,
    invalidated: bool,
    /// Whether VideoToolbox reports a hardware encoder.
    pub hardware: Option<bool>,
}

// The session is a CF object; VideoToolbox calls the output handler on its own threads.
unsafe impl Send for Encoder {}

fn set(
    session: &VTCompressionSession,
    key: &CFString,
    value: &AnyObject,
    optional: bool,
) -> Result<(), Error> {
    let st = unsafe {
        VTSessionSetProperty(
            session,
            key,
            Some(&*(value as *const AnyObject as *const CFType)),
        )
    };
    if optional && st == PROPERTY_NOT_SUPPORTED {
        return Ok(());
    }
    cf::check(st, &format!("VTSessionSetProperty {}", cf::ns(key)))
}

fn copy_bool(session: &VTCompressionSession, key: &CFString) -> Option<bool> {
    let mut out: *const CFType = std::ptr::null();
    let st = unsafe {
        VTSessionCopyProperty(
            session,
            key,
            None,
            &mut out as *mut _ as *mut std::ffi::c_void,
        )
    };
    if st != 0 || out.is_null() {
        return None;
    }
    let v = unsafe { CFRetained::from_raw(NonNull::new_unchecked(out as *mut CFType)) };
    Some(cf::describe(&cf::any_cf(&v)) == "1")
}

/// The CMVideoCodecType of an export: HEVC with alpha is its own codec type
/// (`kCMVideoCodecType_HEVCWithAlpha`); every other codec is its sample entry.
pub fn codec_type(spec: &ExportSpec) -> u32 {
    match spec.codec {
        Codec::Hevc { .. } if spec.alpha != AlphaOutput::None => u32::from_be_bytes(*b"muxa"),
        codec => u32::from_be_bytes(codec.fourcc()),
    }
}

fn set_alpha_mode(session: &VTCompressionSession, alpha: AlphaOutput) -> Result<(), Error> {
    unsafe {
        match alpha {
            AlphaOutput::None => Ok(()),
            AlphaOutput::Straight => set(
                session,
                kVTCompressionPropertyKey_AlphaChannelMode,
                &any_str(kVTAlphaChannelMode_StraightAlpha),
                true,
            ),
            AlphaOutput::Premultiplied => set(
                session,
                kVTCompressionPropertyKey_AlphaChannelMode,
                &any_str(kVTAlphaChannelMode_PremultipliedAlpha),
                true,
            ),
        }
    }
}

/// Rate control and GOP of the long-GOP codecs: variable bit rate at today's
/// quality tiers, B-frames, and a keyframe at least every two seconds so
/// exports stay seekable.
///
/// VBR, not `AverageBitRate`: with an average target the hardware encoder
/// undershoots badly on easy and grainy content (measured on this Mac, 1080p
/// H.264: 0.72 Mbps for a 4 Mbps tier, VMAF 93.4 against x264's 95.1), while
/// VBR lands on the tier and matches or beats x264 (VMAF 95.6 at 4.16 Mbps;
/// `tests/measure_delivery.rs`). Encoders without VBR (before macOS 15) take
/// the average target, and so does HEVC with alpha: its encoder accepts the
/// VBR property and then fails every frame (-12912, macOS 27).
fn set_long_gop(
    session: &VTCompressionSession,
    bitrate_bps: u32,
    alpha: AlphaOutput,
) -> Result<(), Error> {
    unsafe {
        let rate = any_num_i64(bitrate_bps as i64);
        let st = if alpha == AlphaOutput::None {
            VTSessionSetProperty(
                session,
                kVTCompressionPropertyKey_VariableBitRate,
                Some(&*(&*rate as *const AnyObject as *const CFType)),
            )
        } else {
            PROPERTY_NOT_SUPPORTED
        };
        if st == PROPERTY_NOT_SUPPORTED {
            set(
                session,
                kVTCompressionPropertyKey_AverageBitRate,
                &rate,
                false,
            )?;
        } else {
            cf::check(st, "VTSessionSetProperty VariableBitRate")?;
        }
        set(
            session,
            kVTCompressionPropertyKey_AllowFrameReordering,
            &any_bool(true),
            true,
        )?;
        set(
            session,
            kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration,
            &any_num_f64(2.0),
            true,
        )
    }
}

impl Encoder {
    pub fn new(spec: &ExportSpec, timing: VideoTiming) -> Result<Self, Error> {
        if !matches!(
            spec.codec,
            Codec::ProRes(_) | Codec::H264 { .. } | Codec::Hevc { .. }
        ) {
            return Err(Error::InvalidSpec(format!(
                "{} is not a VideoToolbox codec",
                spec.codec.name()
            )));
        }
        let codec_type = codec_type(spec);
        let mut sess: *mut VTCompressionSession = std::ptr::null_mut();
        let st = unsafe {
            VTCompressionSession::create(
                None,
                spec.width as i32,
                spec.height as i32,
                codec_type,
                None,
                None,
                None,
                None,
                std::ptr::null_mut(),
                NonNull::from(&mut sess),
            )
        };
        cf::check(
            st,
            &format!(
                "VTCompressionSessionCreate '{}'",
                cf::fourcc_str(codec_type)
            ),
        )?;
        let session = unsafe { CFRetained::from_raw(NonNull::new_unchecked(sess)) };
        let (primaries, transfer, matrix) = color_constants(spec.tags);
        unsafe {
            // Tags as session properties for every codec: HEVC with alpha drops buffer
            // attachments, and an untagged stream reads as 1-1-1 (S2).
            set(
                &session,
                kVTCompressionPropertyKey_ColorPrimaries,
                &any_str(primaries),
                false,
            )?;
            set(
                &session,
                kVTCompressionPropertyKey_TransferFunction,
                &any_str(transfer),
                false,
            )?;
            set(
                &session,
                kVTCompressionPropertyKey_YCbCrMatrix,
                &any_str(matrix),
                false,
            )?;
            set(
                &session,
                kVTCompressionPropertyKey_RealTime,
                &any_bool(false),
                true,
            )?;
            set(
                &session,
                kVTCompressionPropertyKey_ExpectedFrameRate,
                &any_num_f64(spec.rate.to_f64()),
                true,
            )?;
            match spec.codec {
                Codec::H264 { bitrate_bps } => {
                    set(
                        &session,
                        kVTCompressionPropertyKey_ProfileLevel,
                        &any_str(kVTProfileLevel_H264_High_AutoLevel),
                        false,
                    )?;
                    set(
                        &session,
                        kVTCompressionPropertyKey_H264EntropyMode,
                        &any_str(kVTH264EntropyMode_CABAC),
                        true,
                    )?;
                    set_long_gop(&session, bitrate_bps, AlphaOutput::None)?;
                }
                Codec::Hevc {
                    bitrate_bps,
                    ten_bit,
                } => {
                    set(
                        &session,
                        kVTCompressionPropertyKey_ProfileLevel,
                        &any_str(if ten_bit {
                            kVTProfileLevel_HEVC_Main10_AutoLevel
                        } else {
                            kVTProfileLevel_HEVC_Main_AutoLevel
                        }),
                        false,
                    )?;
                    set_long_gop(&session, bitrate_bps, spec.alpha)?;
                    set_alpha_mode(&session, spec.alpha)?;
                }
                Codec::ProRes(_) => set_alpha_mode(&session, spec.alpha)?,
                Codec::Vp9 { .. } | Codec::Vp8 { .. } | Codec::Png { .. } => {
                    unreachable!("checked above")
                }
            }
            VTCompressionSession::prepare_to_encode_frames(&session);
        }
        let hardware = copy_bool(&session, unsafe {
            kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder
        });
        Ok(Self {
            session,
            output: Arc::default(),
            timing,
            invalidated: false,
            hardware,
        })
    }

    /// Submits frame `index`; compressed samples arrive in decode order.
    pub fn encode(&mut self, buffer: &PixelBuffer, index: u64) -> Result<(), Error> {
        let out = self.output.clone();
        let handler = RcBlock::new(
            move |status: i32, flags: VTEncodeInfoFlags, sb: *mut CMSampleBuffer| {
                let mut o = out.lock().unwrap_or_else(|p| p.into_inner());
                if status != 0 {
                    o.error
                        .get_or_insert(format!("VideoToolbox encode failed: OSStatus {status}"));
                } else if sb.is_null() {
                    if flags.contains(VTEncodeInfoFlags::FrameDropped) {
                        o.error.get_or_insert("VideoToolbox dropped a frame".into());
                    }
                } else {
                    o.samples
                        .push_back(unsafe { CFRetained::retain(NonNull::new_unchecked(sb)) });
                }
            },
        );
        let pts = unsafe { CMTime::new(self.timing.pts(index), self.timing.timescale) };
        let dur = unsafe { CMTime::new(self.timing.frame_duration, self.timing.timescale) };
        let st = unsafe {
            self.session.encode_frame_with_output_handler(
                buffer.as_cf(),
                pts,
                dur,
                None,
                std::ptr::null_mut(),
                &*handler as *const _ as *mut _,
            )
        };
        cf::check(st, "VTCompressionSessionEncodeFrame")?;
        self.check()
    }

    fn check(&self) -> Result<(), Error> {
        match &self.output.lock().unwrap_or_else(|p| p.into_inner()).error {
            Some(e) => Err(Error::Encode(e.clone())),
            None => Ok(()),
        }
    }

    /// Compressed samples produced so far, in decode order.
    pub fn take(&self) -> Result<Vec<CFRetained<CMSampleBuffer>>, Error> {
        self.check()?;
        Ok(self
            .output
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .samples
            .drain(..)
            .collect())
    }

    /// Flushes every pending frame.
    pub fn complete(&mut self) -> Result<Vec<CFRetained<CMSampleBuffer>>, Error> {
        let st = unsafe { self.session.complete_frames(kCMTimeInvalid) };
        cf::check(st, "VTCompressionSessionCompleteFrames")?;
        self.take()
    }

    fn invalidate(&mut self) {
        if !self.invalidated {
            unsafe { self.session.invalidate() };
            self.invalidated = true;
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.invalidate();
    }
}
