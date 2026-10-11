//! VideoToolbox decode into IOSurface-backed, Metal-compatible CVPixelBuffers.
//!
//! Output formats follow plan section 13: ProRes 422 → `x422`; ProRes 4444/XQ →
//! `y416`; 10-bit 4:2:0 → `x420` (`xf20` full range), 4:2:2 → `x422` (`xf22`);
//! 8-bit → `420v` (`420f`). HEVC with an alpha layer → `v0a8` (8-bit 4:2:0 plus
//! an alpha plane; the plan does not name one, this is VideoToolbox's).

use crate::apple::*;
use crate::ff::{Packet, StreamInfo};
use crate::index::FrameIndex;
use anyhow::{Context, Result, bail, ensure};
use std::ffi::c_void;
use std::sync::Mutex;

pub const F_420V: u32 = fcc(b"420v");
pub const F_420F: u32 = fcc(b"420f");
pub const F_X420: u32 = fcc(b"x420");
pub const F_XF20: u32 = fcc(b"xf20");
pub const F_X422: u32 = fcc(b"x422");
pub const F_XF22: u32 = fcc(b"xf22");
pub const F_Y416: u32 = fcc(b"y416");
pub const F_V0A8: u32 = fcc(b"v0a8");

#[derive(Clone, Debug, serde::Serialize)]
pub struct VtPlan {
    pub codec_type: String,
    pub pixel_format: String,
    #[serde(skip)]
    pub codec_fcc: u32,
    #[serde(skip)]
    pub format_fcc: u32,
    #[serde(skip)]
    pub atoms: Vec<([u8; 4], Vec<u8>)>,
    /// Packets are Annex B and must be rewritten to 4-byte length prefixes.
    #[serde(skip)]
    pub annexb: bool,
    pub alpha_layer: bool,
    /// `VTIsHardwareDecodeSupported` for the codec type.
    pub hw_supported: bool,
    /// How the format description was made, and the extensions CoreMedia
    /// derived that matter here (alpha, chroma siting).
    pub format_source: String,
    pub format_extensions: String,
    #[serde(skip)]
    pub format: Option<Cf>,
}

/// Chooses the VideoToolbox codec, configuration atoms, and output format, or
/// says why VideoToolbox is not the backend for this stream.
pub fn plan(info: &StreamInfo, idx: &FrameIndex, extradata: &[u8]) -> Result<VtPlan, String> {
    let (codec, atoms, annexb): (u32, Vec<([u8; 4], Vec<u8>)>, bool) = if let Some(se) = &idx.sample_entry {
        let f = se.fourcc;
        let codec = match &f {
            b"avc1" | b"avc3" => fcc(b"avc1"),
            b"hvc1" | b"hev1" => fcc(b"hvc1"),
            b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" | b"ap4x" => u32::from_be_bytes(f),
            b"av01" => fcc(b"av01"),
            other => return Err(format!("sample entry {} is not a VideoToolbox codec here", String::from_utf8_lossy(other))),
        };
        // Pass every child atom of the sample entry, as AVFoundation does.
        let atoms = se.atoms.iter().filter(|a| a.1.len() < 1 << 20).cloned().collect();
        (codec, atoms, false)
    } else if let Some(id) = &idx.codec_id {
        let cp = idx.codec_private.clone().unwrap_or_default();
        match id.as_str() {
            "V_MPEG4/ISO/AVC" => (fcc(b"avc1"), vec![(*b"avcC", cp)], false),
            "V_MPEGH/ISO/HEVC" => (fcc(b"hvc1"), vec![(*b"hvcC", cp)], false),
            "V_AV1" => (fcc(b"av01"), vec![(*b"av1C", cp)], false),
            other => return Err(format!("{other} goes to FFmpeg (plan section 13)")),
        }
    } else if info.codec == "h264" {
        let avcc = annexb_to_avcc(extradata).map_err(|e| e.to_string())?;
        (fcc(b"avc1"), vec![(*b"avcC", avcc)], true)
    } else {
        return Err(format!("{} in {} goes to FFmpeg", info.codec, info.container));
    };
    let hw_supported = unsafe { VTIsHardwareDecodeSupported(codec) } != 0;
    let hvcc = atoms.iter().find(|a| &a.0 == b"hvcC").map(|a| a.1.as_slice());
    let alpha_layer = hvcc.map(hevc_max_layers).unwrap_or(1) > 1;
    let cw = info.log2_chroma_w;
    let ch = info.log2_chroma_h;
    let full = info.full_range;
    let fmt = match &codec.to_be_bytes() {
        b"ap4h" | b"ap4x" => F_Y416,
        b"apch" | b"apcn" | b"apcs" | b"apco" => F_X422,
        _ if alpha_layer => F_V0A8,
        _ => match (info.bit_depth > 8, cw, ch, full) {
            (false, 1, 1, false) => F_420V,
            (false, 1, 1, true) => F_420F,
            (true, 1, 1, false) => F_X420,
            (true, 1, 1, true) => F_XF20,
            (true, 1, 0, false) => F_X422,
            (true, 1, 0, true) => F_XF22,
            _ => return Err(format!("{} {}-bit chroma {cw}/{ch} has no section 13 VideoToolbox format", info.codec, info.bit_depth)),
        },
    };
    let (format, format_source) = format_description(codec, &atoms, info.width, info.height)?;
    let ext = |k: &str| unsafe { cf_value(CMFormatDescriptionGetExtension(format.as_mut_ptr(), cf_str(k).as_ptr())) };
    let format_extensions = ["ContainsAlphaChannel", "AlphaChannelMode", "CVImageBufferChromaLocationTopField", "FullRangeVideo"]
        .iter()
        .filter(|k| ext(k) != "absent")
        .map(|k| format!("{k}={}", ext(k)))
        .collect::<Vec<_>>()
        .join(" ");
    let format_extensions = if format_extensions.is_empty() { "none derived".to_string() } else { format_extensions };
    Ok(VtPlan {
        format_source,
        format_extensions,
        format: Some(format),
        codec_type: fourcc(codec),
        pixel_format: fourcc(fmt),
        codec_fcc: codec,
        format_fcc: fmt,
        atoms,
        annexb,
        alpha_layer,
        hw_supported,
    })
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

/// The CoreMedia format description for a stream. H.264 and HEVC go through
/// `CMVideoFormatDescriptionCreateFrom{H264,HEVC}ParameterSets`, which derives
/// what AVFoundation derives from the bitstream: chroma siting, field count,
/// and for HEVC with alpha `ContainsAlphaChannel` and `AlphaChannelMode`
/// (without which VideoToolbox decodes only the base layer and fills the alpha
/// plane with opaque). Other codecs use the sample entry atoms.
fn format_description(codec: u32, atoms: &[([u8; 4], Vec<u8>)], w: i32, h: i32) -> Result<(Cf, String), String> {
    let atom_pairs: Vec<(Cf, Cf)> = atoms.iter().map(|(k, v)| (cf_str(&String::from_utf8_lossy(k)), cf_data(v))).collect();
    let atom_dict = cf_dict(&atom_pairs.iter().map(|(k, v)| (k.as_ptr(), v.as_ptr())).collect::<Vec<_>>());
    let ext = unsafe { cf_dict(&[(kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms, atom_dict.as_ptr())]) };
    let cfg_name: &[u8; 4] = if codec == fcc(b"hvc1") { b"hvcC" } else { b"avcC" };
    let cfg = atoms.iter().find(|a| &a.0 == cfg_name).map(|a| a.1.as_slice());
    let mut fd: CMFormatDescriptionRef = std::ptr::null_mut();
    if let (true, Some(cfg)) = (codec == fcc(b"hvc1") || codec == fcc(b"avc1"), cfg) {
        let sets = parameter_sets(codec, cfg).ok_or("malformed decoder configuration record")?;
        let ptrs: Vec<*const u8> = sets.iter().map(|s| s.as_ptr()).collect();
        let sizes: Vec<usize> = sets.iter().map(|s| s.len()).collect();
        let len_size = if codec == fcc(b"hvc1") { (cfg[21] & 3) as i32 + 1 } else { (cfg[4] & 3) as i32 + 1 };
        let st = unsafe {
            if codec == fcc(b"hvc1") {
                CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                    kCFAllocatorDefault, sets.len(), ptrs.as_ptr(), sizes.as_ptr(), len_size, ext.as_ptr(), &mut fd,
                )
            } else {
                CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    kCFAllocatorDefault, sets.len(), ptrs.as_ptr(), sizes.as_ptr(), len_size, &mut fd,
                )
            }
        };
        if st != 0 {
            return Err(format!("CMVideoFormatDescriptionCreateFrom*ParameterSets: OSStatus {st}"));
        }
        let src = format!("{} parameter sets ({} NAL units)", if codec == fcc(b"hvc1") { "HEVC" } else { "H.264" }, sets.len());
        return Ok((unsafe { Cf::from_create(fd as CFTypeRef) }.ok_or("null format description")?, src));
    }
    let st = unsafe { CMVideoFormatDescriptionCreate(kCFAllocatorDefault, codec, w, h, ext.as_ptr(), &mut fd) };
    if st != 0 {
        return Err(format!("CMVideoFormatDescriptionCreate: OSStatus {st}"));
    }
    Ok((unsafe { Cf::from_create(fd as CFTypeRef) }.ok_or("null format description")?, "sample entry atoms".into()))
}

/// vps_max_layers_minus1 + 1 from the first VPS in an hvcC record.
fn hevc_max_layers(hvcc: &[u8]) -> u32 {
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
                // 2-byte NAL header, then id(4) base_internal(1) base_available(1) max_layers_minus1(6)
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
        let mut e = if k + 1 < starts.len() { starts[k + 1] - 3 } else { b.len() };
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
    ensure!(!sps.is_empty() && !pps.is_empty() && sps[0].len() >= 4, "no SPS/PPS in Annex B extradata");
    let mut v = vec![1, sps[0][1], sps[0][2], sps[0][3], 0xFF, 0xE0 | sps.len() as u8];
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

/// Annex B access unit → 4-byte length-prefixed NAL units, without parameter
/// sets and delimiters (the format description carries the parameter sets).
pub fn annexb_to_length_prefixed(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() + 16);
    for n in annexb_nals(b) {
        if matches!(n[0] & 0x1F, 7 | 8 | 9) {
            continue;
        }
        out.extend_from_slice(&(n.len() as u32).to_be_bytes());
        out.extend_from_slice(n);
    }
    out
}

pub struct VtFrame {
    pub pts: i64,
    /// Callback status; a failed frame has no buffer (for example a leading
    /// picture whose references precede the random-access point).
    pub status: OSStatus,
    pub buffer: Option<Cf>,
}

struct Collector {
    frames: Mutex<Vec<(OSStatus, u32, Option<Cf>, CMTime)>>,
    /// Throughput mode: count successful frames and drop the buffers at once.
    count_only: std::sync::atomic::AtomicBool,
    counted: std::sync::atomic::AtomicUsize,
    failed: std::sync::atomic::AtomicUsize,
}

unsafe extern "C" fn on_output(
    refcon: *mut c_void,
    _src: *mut c_void,
    status: OSStatus,
    flags: u32,
    image: CVPixelBufferRef,
    pts: CMTime,
    _dur: CMTime,
) {
    use std::sync::atomic::Ordering::Relaxed;
    let c = unsafe { &*(refcon as *const Collector) };
    if c.count_only.load(Relaxed) {
        if status == 0 && !image.is_null() {
            c.counted.fetch_add(1, Relaxed);
        } else if flags & kVTDecodeInfo_FrameDropped == 0 {
            c.failed.fetch_add(1, Relaxed);
        }
        return;
    }
    let buf = unsafe { Cf::retain(image as CFTypeRef) };
    c.frames.lock().unwrap().push((status, flags, buf, pts));
}

pub struct VtDecoder {
    session: VTDecompressionSessionRef,
    format: Cf,
    collector: Box<Collector>,
    timescale: i32,
    annexb: bool,
    pub hardware: bool,
    pub requested: u32,
}

unsafe impl Send for VtDecoder {}

impl VtDecoder {
    pub fn new(p: &VtPlan, timescale: i64, require_hw: bool) -> Result<Self> {
        ensure!(timescale <= i32::MAX as i64, "timescale too large for CMTime");
        unsafe {
            let format = p.format.clone().context("plan without a format description")?;
            let fd = format.as_mut_ptr();

            let spec = cf_dict(&[(
                if require_hw {
                    kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder
                } else {
                    kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder
                },
                cf_bool(true),
            )]);
            let pixfmt = cf_i32(p.format_fcc as i32);
            let iosurface = cf_dict(&[]);
            let attrs = cf_dict(&[
                (kCVPixelBufferPixelFormatTypeKey, pixfmt.as_ptr()),
                (kCVPixelBufferMetalCompatibilityKey, cf_bool(true)),
                (kCVPixelBufferIOSurfacePropertiesKey, iosurface.as_ptr()),
            ]);
            let collector = Box::new(Collector {
                frames: Mutex::new(Vec::new()),
                count_only: Default::default(),
                counted: Default::default(),
                failed: Default::default(),
            });
            let rec = VTDecompressionOutputCallbackRecord {
                callback: on_output,
                refcon: &*collector as *const Collector as *mut c_void,
            };
            let mut session: VTDecompressionSessionRef = std::ptr::null_mut();
            let st = VTDecompressionSessionCreate(
                kCFAllocatorDefault,
                fd,
                spec.as_ptr(),
                attrs.as_ptr(),
                &rec,
                &mut session,
            );
            if st != 0 {
                bail!("VTDecompressionSessionCreate({}, {}): OSStatus {st}", p.codec_type, p.pixel_format);
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
            Ok(VtDecoder {
                session,
                format,
                collector,
                timescale: timescale as i32,
                annexb: p.annexb,
                hardware,
                requested: p.format_fcc,
            })
        }
    }

    /// Decode throughput: submits every packet with asynchronous decompression
    /// and waits once at the end; output buffers are counted and released in
    /// the callback. Returns (frames output, frames failed, seconds).
    pub fn throughput(&mut self, pkts: &[Packet]) -> Result<(usize, usize, f64)> {
        use std::sync::atomic::Ordering::Relaxed;
        self.collector.count_only.store(true, Relaxed);
        let t0 = std::time::Instant::now();
        for p in pkts {
            let st = self.submit(p, 1 /* kVTDecodeFrame_EnableAsynchronousDecompression */)?;
            ensure!(st == 0, "VTDecompressionSessionDecodeFrame: OSStatus {st}");
        }
        unsafe {
            VTDecompressionSessionFinishDelayedFrames(self.session);
            VTDecompressionSessionWaitForAsynchronousFrames(self.session);
        }
        let secs = t0.elapsed().as_secs_f64();
        self.collector.count_only.store(false, Relaxed);
        Ok((self.collector.counted.swap(0, Relaxed), self.collector.failed.swap(0, Relaxed), secs))
    }

    /// Submits one sample. Returns the synchronous decode status (non-zero
    /// statuses are also reported through the output callback when the session
    /// produced a callback for the frame).
    pub fn decode(&mut self, pkt: &Packet) -> Result<OSStatus> {
        self.submit(pkt, 0)
    }

    fn submit(&mut self, pkt: &Packet, flags: u32) -> Result<OSStatus> {
        let converted;
        let data: &[u8] = if self.annexb {
            converted = annexb_to_length_prefixed(&pkt.data);
            &converted
        } else {
            &pkt.data
        };
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
            ensure!(st == 0, "CMBlockBufferCreateWithMemoryBlock: {st}");
            let bb = Cf::from_create(bb as CFTypeRef).unwrap();
            ensure!(CMBlockBufferReplaceDataBytes(data.as_ptr() as _, bb.as_mut_ptr(), 0, data.len()) == 0, "block copy");
            let timing = CMSampleTimingInfo {
                duration: CMTime::new(pkt.duration, self.timescale),
                pts: CMTime::new(pkt.pts, self.timescale),
                dts: pkt.dts.map(|d| CMTime::new(d, self.timescale)).unwrap_or(CMTime::invalid()),
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
            ensure!(st == 0, "CMSampleBufferCreateReady: {st}");
            let sb = Cf::from_create(sb as CFTypeRef).unwrap();
            let mut info = 0u32;
            let st = VTDecompressionSessionDecodeFrame(self.session, sb.as_mut_ptr(), flags, std::ptr::null_mut(), &mut info);
            Ok(st)
        }
    }

    /// Waits for every submitted frame and returns them (decode order),
    /// including failed ones; dropped frames are skipped.
    pub fn finish(&mut self) -> Result<Vec<VtFrame>> {
        unsafe {
            VTDecompressionSessionFinishDelayedFrames(self.session);
            VTDecompressionSessionWaitForAsynchronousFrames(self.session);
        }
        let got = std::mem::take(&mut *self.collector.frames.lock().unwrap());
        let mut out = Vec::with_capacity(got.len());
        for (status, flags, buf, pts) in got {
            if flags & kVTDecodeInfo_FrameDropped != 0 {
                continue;
            }
            ensure!(pts.is_valid() && pts.timescale == self.timescale, "callback PTS {pts:?}");
            if status == 0 {
                ensure!(buf.is_some(), "callback without image");
            }
            out.push(VtFrame { pts: pts.value, status, buffer: if status == 0 { buf } else { None } });
        }
        Ok(out)
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

/// Facts about one output buffer, checked against the request (section 13: the
/// requested format's range matches the bitstream, verified on the buffer).
#[derive(Clone, Debug, serde::Serialize)]
pub struct BufferFacts {
    pub pixel_format: String,
    pub matches_request: bool,
    pub iosurface: bool,
    pub width: usize,
    pub height: usize,
    pub planes: usize,
    pub attachments: String,
}

pub fn buffer_facts(pb: &Cf, requested: u32) -> BufferFacts {
    unsafe {
        let p = pb.as_mut_ptr();
        let f = CVPixelBufferGetPixelFormatType(p);
        let att = CVBufferCopyAttachments(p, kCVAttachmentMode_ShouldPropagate);
        let attachments = cf_describe(att).split_whitespace().collect::<Vec<_>>().join(" ");
        if !att.is_null() {
            CFRelease(att);
        }
        BufferFacts {
            pixel_format: fourcc(f),
            matches_request: f == requested,
            iosurface: !CVPixelBufferGetIOSurface(p).is_null(),
            width: CVPixelBufferGetWidth(p),
            height: CVPixelBufferGetHeight(p),
            planes: CVPixelBufferGetPlaneCount(p),
            attachments,
        }
    }
}
