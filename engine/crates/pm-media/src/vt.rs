//! VideoToolbox decode into IOSurface-backed, Metal-compatible CVPixelBuffers.
//!
//! Output formats (section 13 and section 26 amendment 6):
//!
//! | Source | Output |
//! |---|---|
//! | ProRes 422 Proxy, LT, 422, HQ | `x422` for display; `v216` (16-bit) when the frame can reach an export |
//! | ProRes 4444, 4444 XQ | `y416` |
//! | 10-bit H.264, HEVC, AV1 | `x420` / `xf20` (4:2:0), `x422` / `xf22` (4:2:2) |
//! | 8-bit | `420v` / `420f` |
//! | HEVC with an alpha layer | `v0a8` |
//!
//! The range of the requested format follows the bitstream and every output
//! buffer is checked against the request. Sessions are reused across seeks;
//! each seek first submits an end-of-sequence sample (S6 finding 1).

use crate::apple::cf::*;
use crate::error::{MediaError, Result};
use crate::ff::{CodecInfo, Packet};
use crate::frame::{CvBuffer, DecodedFrame, Pixels, corevideo_format, fourcc::*};
use crate::index::FrameIndex;
use pm_time::Rational;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};

/// What a decoded frame is for: ProRes 422 decodes to 16-bit `v216` when it
/// can reach an export (amendment 6), to `x422` for display only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DecodePurpose {
    Display,
    Export,
}

/// Sample description extension atoms: (type, payload).
pub type Atoms = Vec<([u8; 4], Vec<u8>)>;

/// How a stream decodes through VideoToolbox.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VtConfig {
    pub codec_type: String,
    pub pixel_format: String,
    #[serde(skip)]
    pub codec_fcc: u32,
    #[serde(skip)]
    pub format_fcc: u32,
    #[serde(skip)]
    pub atoms: Atoms,
    /// Packets are Annex B (MPEG-TS) and are rewritten to 4-byte length prefixes.
    pub annexb: bool,
    pub alpha_layer: bool,
    pub prores: bool,
    /// `VTIsHardwareDecodeSupported` for the codec type.
    pub hardware_supported: bool,
    /// How the format description was made.
    pub format_source: String,
    #[serde(skip)]
    pub format: Option<Cf>,
    /// The bitstream range, from the format description when it says, else
    /// from FFmpeg's probe.
    pub full_range: bool,
    /// ProRes frames stored without their 8-byte `icpf` header (Matroska):
    /// it is restored before decoding.
    pub prores_header: bool,
}

pub(crate) fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { VTRegisterProfessionalVideoWorkflowVideoDecoders() });
}

/// Whether the stream is ProRes (decoded only through VideoToolbox, decision 12).
pub fn is_prores_fourcc(f: &[u8; 4]) -> bool {
    matches!(f, b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" | b"ap4x")
}

impl VtConfig {
    /// Chooses the VideoToolbox codec, configuration atoms, and output
    /// format, or says why VideoToolbox is not the backend for this stream.
    pub fn new(
        info: &CodecInfo,
        idx: &FrameIndex,
        extradata: &[u8],
        purpose: DecodePurpose,
    ) -> Result<VtConfig> {
        init();
        let cfg = &idx.config;
        let mut prores_header = false;
        let (codec, atoms, annexb): (u32, Atoms, bool) = if let Some(f) = cfg.fourcc {
            let codec = match &f {
                b"avc1" | b"avc3" => fcc(b"avc1"),
                b"hvc1" | b"hev1" => fcc(b"hvc1"),
                f if is_prores_fourcc(f) => u32::from_be_bytes(*f),
                b"av01" => fcc(b"av01"),
                other => {
                    return Err(MediaError::unsupported(format!(
                        "sample entry {} is not decoded through VideoToolbox",
                        String::from_utf8_lossy(other)
                    )));
                }
            };
            // Every child atom of the sample entry, as AVFoundation passes them.
            (
                codec,
                cfg.atoms
                    .iter()
                    .filter(|a| a.1.len() < 1 << 20)
                    .cloned()
                    .collect(),
                false,
            )
        } else if let Some(id) = &cfg.codec_id {
            let cp = cfg.codec_private.clone().unwrap_or_default();
            match id.as_str() {
                "V_MPEG4/ISO/AVC" => (fcc(b"avc1"), vec![(*b"avcC", cp)], false),
                "V_MPEGH/ISO/HEVC" => (fcc(b"hvc1"), vec![(*b"hvcC", cp)], false),
                "V_AV1" => (fcc(b"av01"), vec![(*b"av1C", cp)], false),
                // Matroska ProRes: CodecPrivate is the four-character code;
                // frames are stored without their 8-byte `icpf` header.
                "V_PRORES" => {
                    let f: [u8; 4] = cp
                        .get(..4)
                        .and_then(|b| b.try_into().ok())
                        .unwrap_or(prores_fourcc(info.profile));
                    if !is_prores_fourcc(&f) {
                        return Err(MediaError::invalid(format!(
                            "Matroska ProRes with CodecPrivate {cp:?}"
                        )));
                    }
                    prores_header = true;
                    (u32::from_be_bytes(f), vec![], false)
                }
                other => {
                    return Err(MediaError::unsupported(format!(
                        "{other} goes to FFmpeg (section 13)"
                    )));
                }
            }
        } else if info.codec == "prores" {
            // MXF (RDD 44) and other containers: the codec tag, else FFmpeg's profile.
            let tag = info
                .codec_tag
                .as_deref()
                .and_then(|t| <[u8; 4]>::try_from(t.as_bytes()).ok())
                .filter(is_prores_fourcc);
            (
                u32::from_be_bytes(tag.unwrap_or(prores_fourcc(info.profile))),
                vec![],
                false,
            )
        } else if info.codec == "h264" && extradata.first() == Some(&1) {
            // FLV and others: the extradata is already an `avcC` record and
            // samples are length-prefixed.
            (fcc(b"avc1"), vec![(*b"avcC", extradata.to_vec())], false)
        } else if info.codec == "hevc" && extradata.first() == Some(&1) && extradata.len() > 22 {
            (fcc(b"hvc1"), vec![(*b"hvcC", extradata.to_vec())], false)
        } else if info.codec == "h264" && !extradata.is_empty() {
            // MPEG-TS, AVI: Annex B parameter sets and samples.
            (
                fcc(b"avc1"),
                vec![(*b"avcC", annexb_to_avcc(extradata)?)],
                true,
            )
        } else {
            return Err(MediaError::unsupported(format!(
                "{} without MOV, MP4, or Matroska configuration goes to FFmpeg",
                info.codec
            )));
        };
        let hardware_supported = unsafe { VTIsHardwareDecodeSupported(codec) } != 0;
        let hvcc = atoms
            .iter()
            .find(|a| &a.0 == b"hvcC")
            .map(|a| a.1.as_slice());
        let alpha_layer = hvcc.map(hevc_max_layers).unwrap_or(1) > 1;
        let prores = is_prores_fourcc(&codec.to_be_bytes());
        let (format, format_source) = format_description(codec, &atoms, info.width, info.height)?;
        let ext_full = unsafe {
            cf_bool_value(CMFormatDescriptionGetExtension(
                format.as_mut_ptr(),
                kCMFormatDescriptionExtension_FullRangeVideo,
            ))
        };
        let full = ext_full.unwrap_or(info.full_range);
        let (cw, ch) = (info.log2_chroma_w, info.log2_chroma_h);
        let fmt = match &codec.to_be_bytes() {
            b"ap4h" | b"ap4x" => F_Y416,
            b"apch" | b"apcn" | b"apcs" | b"apco" => match purpose {
                DecodePurpose::Display => F_X422,
                DecodePurpose::Export => F_V216,
            },
            _ if alpha_layer => F_V0A8,
            _ => match (info.bit_depth > 8, cw, ch, full) {
                (false, 1, 1, false) => F_420V,
                (false, 1, 1, true) => F_420F,
                (true, 1, 1, false) => F_X420,
                (true, 1, 1, true) => F_XF20,
                (true, 1, 0, false) => F_X422,
                (true, 1, 0, true) => F_XF22,
                _ => {
                    return Err(MediaError::unsupported(format!(
                        "{} {}-bit with chroma {cw}/{ch} has no section 13 VideoToolbox format",
                        info.codec, info.bit_depth
                    )));
                }
            },
        };
        Ok(VtConfig {
            codec_type: fourcc(codec),
            pixel_format: fourcc(fmt),
            codec_fcc: codec,
            format_fcc: fmt,
            atoms,
            annexb,
            alpha_layer,
            prores,
            hardware_supported,
            format_source,
            format: Some(format),
            full_range: full,
            prores_header,
        })
    }
}

/// FFmpeg's ProRes profile (`AV_PROFILE_PRORES_*`) as a four-character code.
pub fn prores_fourcc(profile: i32) -> [u8; 4] {
    match profile {
        0 => *b"apco",
        1 => *b"apcs",
        3 => *b"apch",
        4 => *b"ap4h",
        5 => *b"ap4x",
        _ => *b"apcn",
    }
}

/// Parameter-set NAL units from an hvcC (every array) or avcC (SPS, PPS).
fn parameter_sets(codec: u32, cfg: &[u8]) -> Option<Vec<&[u8]>> {
    let mut out = vec![];
    if codec == fcc(b"hvc1") {
        let mut p = 23;
        for _ in 0..*cfg.get(22)? {
            let n = u16::from_be_bytes([*cfg.get(p + 1)?, *cfg.get(p + 2)?]) as usize;
            p += 3;
            for _ in 0..n {
                let len = u16::from_be_bytes([*cfg.get(p)?, *cfg.get(p + 1)?]) as usize;
                out.push(cfg.get(p + 2..p + 2 + len)?);
                p += 2 + len;
            }
        }
    } else {
        let mut p = 6;
        let nsps = (*cfg.get(5)? & 0x1F) as usize;
        for _ in 0..nsps {
            let len = u16::from_be_bytes([*cfg.get(p)?, *cfg.get(p + 1)?]) as usize;
            out.push(cfg.get(p + 2..p + 2 + len)?);
            p += 2 + len;
        }
        let npps = *cfg.get(p)? as usize;
        p += 1;
        for _ in 0..npps {
            let len = u16::from_be_bytes([*cfg.get(p)?, *cfg.get(p + 1)?]) as usize;
            out.push(cfg.get(p + 2..p + 2 + len)?);
            p += 2 + len;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The CoreMedia format description. H.264 and HEVC go through
/// `CMVideoFormatDescriptionCreateFrom{H264,HEVC}ParameterSets`, which
/// derives what AVFoundation derives from the bitstream: chroma siting, field
/// count, and for HEVC with alpha `ContainsAlphaChannel` (without which
/// VideoToolbox decodes only the base layer). Other codecs use the sample
/// entry atoms.
fn format_description(
    codec: u32,
    atoms: &[([u8; 4], Vec<u8>)],
    w: i32,
    h: i32,
) -> Result<(Cf, String)> {
    let atom_pairs: Vec<(Cf, Cf)> = atoms
        .iter()
        .map(|(k, v)| (cf_str(&String::from_utf8_lossy(k)), cf_data(v)))
        .collect();
    let atom_dict = cf_dict(
        &atom_pairs
            .iter()
            .map(|(k, v)| (k.as_ptr(), v.as_ptr()))
            .collect::<Vec<_>>(),
    );
    let ext = unsafe {
        cf_dict(&[(
            kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms,
            atom_dict.as_ptr(),
        )])
    };
    let cfg_name: &[u8; 4] = if codec == fcc(b"hvc1") {
        b"hvcC"
    } else {
        b"avcC"
    };
    let cfg = atoms
        .iter()
        .find(|a| &a.0 == cfg_name)
        .map(|a| a.1.as_slice());
    let mut fd: CMFormatDescriptionRef = std::ptr::null_mut();
    if let (true, Some(cfg)) = (codec == fcc(b"hvc1") || codec == fcc(b"avc1"), cfg) {
        let sets = parameter_sets(codec, cfg)
            .ok_or_else(|| MediaError::invalid("malformed decoder configuration record"))?;
        let ptrs: Vec<*const u8> = sets.iter().map(|s| s.as_ptr()).collect();
        let sizes: Vec<usize> = sets.iter().map(|s| s.len()).collect();
        let len_size = if codec == fcc(b"hvc1") {
            (cfg[21] & 3) as i32 + 1
        } else {
            (cfg[4] & 3) as i32 + 1
        };
        let st = unsafe {
            if codec == fcc(b"hvc1") {
                CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                    kCFAllocatorDefault,
                    sets.len(),
                    ptrs.as_ptr(),
                    sizes.as_ptr(),
                    len_size,
                    ext.as_ptr(),
                    &mut fd,
                )
            } else {
                CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    kCFAllocatorDefault,
                    sets.len(),
                    ptrs.as_ptr(),
                    sizes.as_ptr(),
                    len_size,
                    &mut fd,
                )
            }
        };
        if st != 0 {
            return Err(MediaError::VideoToolbox {
                what: "format description from parameter sets".into(),
                status: st,
            });
        }
        let src = format!(
            "{} parameter sets",
            if codec == fcc(b"hvc1") {
                "HEVC"
            } else {
                "H.264"
            }
        );
        return Ok((
            unsafe { Cf::from_create(fd as CFTypeRef) }
                .ok_or_else(|| MediaError::invalid("null format description"))?,
            src,
        ));
    }
    let st = unsafe {
        CMVideoFormatDescriptionCreate(kCFAllocatorDefault, codec, w, h, ext.as_ptr(), &mut fd)
    };
    if st != 0 {
        return Err(MediaError::VideoToolbox {
            what: "format description".into(),
            status: st,
        });
    }
    Ok((
        unsafe { Cf::from_create(fd as CFTypeRef) }
            .ok_or_else(|| MediaError::invalid("null format description"))?,
        "sample entry atoms".into(),
    ))
}

/// vps_max_layers_minus1 + 1 from the first VPS in an hvcC record.
pub fn hevc_max_layers(hvcc: &[u8]) -> u32 {
    if hvcc.len() < 23 {
        return 1;
    }
    let mut p = 23;
    for _ in 0..hvcc[22] {
        if p + 3 > hvcc.len() {
            return 1;
        }
        let ty = hvcc[p] & 0x3F;
        let n = u16::from_be_bytes([hvcc[p + 1], hvcc[p + 2]]) as usize;
        p += 3;
        for _ in 0..n {
            if p + 2 > hvcc.len() {
                return 1;
            }
            let len = u16::from_be_bytes([hvcc[p], hvcc[p + 1]]) as usize;
            let nal = &hvcc[(p + 2).min(hvcc.len())..(p + 2 + len).min(hvcc.len())];
            if ty == 32 && nal.len() >= 4 {
                // 2-byte NAL header, then id(4) base_internal(1) base_available(1) max_layers_minus1(6).
                let v = u16::from_be_bytes([nal[2], nal[3]]);
                return ((v >> 4) & 0x3F) as u32 + 1;
            }
            p += 2 + len;
        }
    }
    1
}

/// Splits an Annex B byte stream into NAL units.
pub fn annexb_nals(b: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let mut e = if k + 1 < starts.len() {
            starts[k + 1] - 3
        } else {
            b.len()
        };
        while e > s && b[e - 1] == 0 {
            e -= 1;
        }
        if e > s {
            out.push(&b[s..e]);
        }
    }
    out
}

fn annexb_to_avcc(extradata: &[u8]) -> Result<Vec<u8>> {
    let nals = annexb_nals(extradata);
    let sps: Vec<&[u8]> = nals.iter().copied().filter(|n| n[0] & 0x1F == 7).collect();
    let pps: Vec<&[u8]> = nals.iter().copied().filter(|n| n[0] & 0x1F == 8).collect();
    if sps.is_empty() || pps.is_empty() || sps[0].len() < 4 {
        return Err(MediaError::unsupported(
            "no SPS and PPS in Annex B extradata",
        ));
    }
    let mut v = vec![
        1,
        sps[0][1],
        sps[0][2],
        sps[0][3],
        0xFF,
        0xE0 | sps.len() as u8,
    ];
    for s in &sps {
        v.extend_from_slice(&(s.len() as u16).to_be_bytes());
        v.extend_from_slice(s);
    }
    v.push(pps.len() as u8);
    for p in &pps {
        v.extend_from_slice(&(p.len() as u16).to_be_bytes());
        v.extend_from_slice(p);
    }
    Ok(v)
}

/// Annex B access unit → 4-byte length-prefixed NAL units without parameter
/// sets and delimiters (the format description carries the parameter sets).
pub fn annexb_to_length_prefixed(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() + 16);
    for n in annexb_nals(b) {
        if matches!(n[0] & 0x1F, 7..=9) {
            continue;
        }
        out.extend_from_slice(&(n.len() as u32).to_be_bytes());
        out.extend_from_slice(n);
    }
    out
}

/// One output callback: (status, info flags, buffer, presentation time value).
type Output = (OSStatus, u32, Option<Cf>, CMTime);

struct Collector {
    frames: Mutex<Vec<Output>>,
    eos_callbacks: AtomicUsize,
}

/// `sourceFrameRefCon` of end-of-sequence samples.
const EOS_REFCON: usize = 1;
const ASYNC: u32 = 1; // kVTDecodeFrame_EnableAsynchronousDecompression
/// kVTDecodeFrame_DoNotOutputFrame: the picture is decoded (references
/// stay correct) but no image buffer is emitted for it.
const NO_OUTPUT: u32 = 1 << 1;

unsafe extern "C" fn on_output(
    refcon: *mut c_void,
    src: *mut c_void,
    status: OSStatus,
    flags: u32,
    image: CVPixelBufferRef,
    pts: CMTime,
    _dur: CMTime,
) {
    let c = unsafe { &*(refcon as *const Collector) };
    if src as usize == EOS_REFCON {
        // The end-of-sequence sample has no picture; VideoToolbox reports it
        // as bad data (−12909). Counted, never surfaced as a frame.
        c.eos_callbacks.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let buf = unsafe { Cf::retain(image as CFTypeRef) };
    c.frames.lock().unwrap().push((status, flags, buf, pts));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NalKind {
    H264,
    Hevc,
}

/// One VideoToolbox decompression session.
pub struct VtDecoder {
    session: VTDecompressionSessionRef,
    format: Cf,
    collector: Box<Collector>,
    /// CMTime timescale: the media timebase's denominator. Timestamps in
    /// media units are multiplied by `unit` (the timebase's numerator) so
    /// VideoToolbox sees real durations; outputs are divided back.
    timescale: i32,
    unit: i64,
    annexb: bool,
    nal: Option<(NalKind, usize)>,
    hardware: bool,
    requested: u32,
    prores_header: bool,
    /// Frames whose decode failed: (PTS, OSStatus).
    pub failures: Vec<(i64, OSStatus)>,
}

unsafe impl Send for VtDecoder {}

impl VtDecoder {
    /// `allow_hardware` false asks for VideoToolbox's software decoder
    /// (when the hardware arbiter has no session to give).
    /// `timebase` is the media timebase of the timestamps fed in (and out).
    pub fn new(cfg: &VtConfig, timebase: Rational, allow_hardware: bool) -> Result<Self> {
        let timescale = i32::try_from(timebase.den())
            .map_err(|_| MediaError::unsupported("timescale too large for CMTime"))?;
        let unit = timebase.num();
        if unit <= 0 {
            return Err(MediaError::invalid(format!("timebase {timebase:?}")));
        }
        unsafe {
            let format = cfg
                .format
                .clone()
                .ok_or_else(|| MediaError::invalid("configuration without a format description"))?;
            let spec = cf_dict(&[(
                kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder,
                cf_bool(allow_hardware),
            )]);
            let pixfmt = cf_i32(cfg.format_fcc as i32);
            let iosurface = cf_dict(&[]);
            let attrs = cf_dict(&[
                (kCVPixelBufferPixelFormatTypeKey, pixfmt.as_ptr()),
                (kCVPixelBufferMetalCompatibilityKey, cf_bool(true)),
                (kCVPixelBufferIOSurfacePropertiesKey, iosurface.as_ptr()),
            ]);
            let collector = Box::new(Collector {
                frames: Mutex::new(Vec::new()),
                eos_callbacks: AtomicUsize::new(0),
            });
            let rec = VTDecompressionOutputCallbackRecord {
                callback: on_output,
                refcon: &*collector as *const Collector as *mut c_void,
            };
            let mut session: VTDecompressionSessionRef = std::ptr::null_mut();
            let st = VTDecompressionSessionCreate(
                kCFAllocatorDefault,
                format.as_mut_ptr(),
                spec.as_ptr(),
                attrs.as_ptr(),
                &rec,
                &mut session,
            );
            if st != 0 {
                return Err(MediaError::VideoToolbox {
                    what: format!(
                        "create session ({} to {})",
                        cfg.codec_type, cfg.pixel_format
                    ),
                    status: st,
                });
            }
            let mut hw: CFTypeRef = std::ptr::null();
            let hardware = VTSessionCopyProperty(
                session,
                kVTDecompressionPropertyKey_UsingHardwareAcceleratedVideoDecoder,
                kCFAllocatorDefault,
                &mut hw as *mut CFTypeRef as *mut c_void,
            ) == 0
                && !hw.is_null()
                && CFBooleanGetValue(hw) != 0;
            if !hw.is_null() {
                CFRelease(hw);
            }
            let atom = |n: &[u8; 4]| cfg.atoms.iter().find(|a| &a.0 == n).map(|a| a.1.as_slice());
            let nal = match &cfg.codec_fcc.to_be_bytes() {
                b"avc1" if cfg.annexb => Some((NalKind::H264, 4)),
                b"avc1" => atom(b"avcC")
                    .filter(|c| c.len() > 4)
                    .map(|c| (NalKind::H264, (c[4] & 3) as usize + 1)),
                b"hvc1" => atom(b"hvcC")
                    .filter(|c| c.len() > 21)
                    .map(|c| (NalKind::Hevc, (c[21] & 3) as usize + 1)),
                _ => None,
            };
            Ok(VtDecoder {
                session,
                format,
                collector,
                timescale,
                unit,
                annexb: cfg.annexb,
                nal,
                hardware,
                requested: cfg.format_fcc,
                prores_header: cfg.prores_header,
                failures: vec![],
            })
        }
    }

    pub fn hardware(&self) -> bool {
        self.hardware
    }

    /// Submits one sample (asynchronous decompression).
    pub fn send(&mut self, p: &Packet, out: &mut Vec<DecodedFrame>) -> Result<()> {
        self.send_flags(p, out, ASYNC)
    }

    /// Submits one sample whose picture is only needed as a reference: it
    /// is decoded, but VideoToolbox emits no image buffer for it.
    pub fn send_reference_only(&mut self, p: &Packet, out: &mut Vec<DecodedFrame>) -> Result<()> {
        self.send_flags(p, out, ASYNC | NO_OUTPUT)
    }

    fn send_flags(&mut self, p: &Packet, out: &mut Vec<DecodedFrame>, flags: u32) -> Result<()> {
        let converted;
        let data: &[u8] = if self.annexb {
            converted = annexb_to_length_prefixed(&p.data);
            &converted
        } else if self.prores_header && p.data.get(4..8) != Some(b"icpf") {
            let mut v = Vec::with_capacity(p.data.len() + 8);
            v.extend_from_slice(&((p.data.len() + 8) as u32).to_be_bytes());
            v.extend_from_slice(b"icpf");
            v.extend_from_slice(&p.data);
            converted = v;
            &converted
        } else {
            &p.data
        };
        let st = self.submit(data, p, flags, 0)?;
        if st != 0 {
            self.failures.push((p.pts, st));
        }
        self.collect(out)
    }

    /// Waits for every submitted sample and outputs its frame.
    pub fn sync(&mut self, out: &mut Vec<DecodedFrame>) -> Result<()> {
        unsafe {
            VTDecompressionSessionFinishDelayedFrames(self.session);
            VTDecompressionSessionWaitForAsynchronousFrames(self.session);
        }
        self.collect(out)
    }

    /// Starts a new coded video sequence before the next sample (a seek
    /// within the session). VideoToolbox has no reference-flush call, and an
    /// H.264 decoder keeps its short-term references across a non-IDR I
    /// picture, so an open-GOP random-access point decoded after unrelated
    /// pictures would build reference lists from stale pictures (S6: 35 of
    /// 150 seeks wrong). An end-of-sequence NAL unit (H.264 type 10, HEVC
    /// type 36) submitted as its own sample ends the previous sequence.
    /// Other codecs need nothing.
    pub fn start_sequence(&mut self) -> Result<()> {
        unsafe { VTDecompressionSessionWaitForAsynchronousFrames(self.session) };
        self.collector.frames.lock().unwrap().clear();
        self.failures.clear();
        let Some((kind, ls)) = self.nal else {
            return Ok(());
        };
        let eos: &[u8] = match kind {
            NalKind::H264 => &[0x0A],
            NalKind::Hevc => &[36 << 1, 0x01],
        };
        let mut v = Vec::with_capacity(ls + eos.len());
        v.extend_from_slice(&(eos.len() as u64).to_be_bytes()[8 - ls..]);
        v.extend_from_slice(eos);
        let timing = Packet::default();
        self.submit(&v, &timing, 0, EOS_REFCON)?;
        Ok(())
    }

    /// End-of-sequence samples whose callbacks arrived.
    pub fn eos_callbacks(&self) -> usize {
        self.collector.eos_callbacks.load(Ordering::Relaxed)
    }

    fn submit(&mut self, data: &[u8], pkt: &Packet, flags: u32, refcon: usize) -> Result<OSStatus> {
        unsafe {
            let mut bb: CMBlockBufferRef = std::ptr::null_mut();
            let st = CMBlockBufferCreateWithMemoryBlock(
                kCFAllocatorDefault,
                std::ptr::null_mut(),
                data.len(),
                kCFAllocatorDefault,
                std::ptr::null(),
                0,
                data.len(),
                kCMBlockBufferAssureMemoryNowFlag,
                &mut bb,
            );
            if st != 0 {
                return Err(MediaError::VideoToolbox {
                    what: "block buffer".into(),
                    status: st,
                });
            }
            let bb = Cf::from_create(bb as CFTypeRef).unwrap();
            let st =
                CMBlockBufferReplaceDataBytes(data.as_ptr() as _, bb.as_mut_ptr(), 0, data.len());
            if st != 0 {
                return Err(MediaError::VideoToolbox {
                    what: "block buffer copy".into(),
                    status: st,
                });
            }
            let at = |v: i64| {
                v.checked_mul(self.unit)
                    .map(|v| CMTime::new(v, self.timescale))
                    .ok_or_else(|| MediaError::invalid(format!("timestamp {v} overflows CMTime")))
            };
            let timing = CMSampleTimingInfo {
                duration: at(pkt.duration)?,
                pts: at(pkt.pts)?,
                dts: match pkt.dts {
                    Some(d) => at(d)?,
                    None => CMTime::invalid(),
                },
            };
            let size = data.len();
            let mut sb: CMSampleBufferRef = std::ptr::null_mut();
            let st = CMSampleBufferCreateReady(
                kCFAllocatorDefault,
                bb.as_mut_ptr(),
                self.format.as_mut_ptr(),
                1,
                1,
                &timing,
                1,
                &size,
                &mut sb,
            );
            if st != 0 {
                return Err(MediaError::VideoToolbox {
                    what: "sample buffer".into(),
                    status: st,
                });
            }
            let sb = Cf::from_create(sb as CFTypeRef).unwrap();
            let mut info = 0u32;
            Ok(VTDecompressionSessionDecodeFrame(
                self.session,
                sb.as_mut_ptr(),
                flags,
                refcon as *mut c_void,
                &mut info,
            ))
        }
    }

    /// Moves every arrived output into `out` (failed frames are recorded).
    fn collect(&mut self, out: &mut Vec<DecodedFrame>) -> Result<()> {
        let got = std::mem::take(&mut *self.collector.frames.lock().unwrap());
        for (status, flags, buf, pts) in got {
            if flags & kVTDecodeInfo_FrameDropped != 0 || (status == 0 && buf.is_none()) {
                // Dropped, or decoded without output (`send_reference_only`).
                continue;
            }
            if !pts.is_valid() || pts.timescale != self.timescale || pts.value % self.unit != 0 {
                return Err(MediaError::decode(format!(
                    "VideoToolbox output time {pts:?}"
                )));
            }
            match buf {
                Some(pb) if status == 0 => {
                    let got_fmt = unsafe { CVPixelBufferGetPixelFormatType(pb.as_mut_ptr()) };
                    if got_fmt != self.requested {
                        return Err(MediaError::decode(format!(
                            "VideoToolbox returned {} for a {} request",
                            fourcc(got_fmt),
                            fourcc(self.requested)
                        )));
                    }
                    if unsafe { CVPixelBufferGetIOSurface(pb.as_mut_ptr()) }.is_null() {
                        return Err(MediaError::decode(
                            "VideoToolbox buffer without an IOSurface",
                        ));
                    }
                    let format = corevideo_format(&pb)?;
                    out.push(DecodedFrame {
                        pts: pts.value / self.unit,
                        format: Arc::new(format),
                        pixels: Pixels::CoreVideo(CvBuffer::untracked(pb)),
                        color: None,
                    });
                }
                _ => self.failures.push((pts.value / self.unit, status)),
            }
        }
        Ok(())
    }
}

impl Drop for VtDecoder {
    fn drop(&mut self) {
        unsafe {
            VTDecompressionSessionWaitForAsynchronousFrames(self.session);
            VTDecompressionSessionInvalidate(self.session);
            CFRelease(self.session as CFTypeRef);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annexb_rewrites_to_length_prefixes_without_parameter_sets() {
        let au = [
            0, 0, 0, 1, 0x67, 0x64, 0, 0x1F, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 9, 9, 9,
        ];
        let v = annexb_to_length_prefixed(&au);
        assert_eq!(v, vec![0, 0, 0, 4, 0x65, 9, 9, 9]);
        let avcc = annexb_to_avcc(&au[..13]).unwrap();
        assert_eq!(
            avcc,
            vec![
                1, 0x64, 0, 0x1F, 0xFF, 0xE1, 0, 4, 0x67, 0x64, 0, 0x1F, 1, 0, 2, 0x68, 3
            ]
        );
    }
}
