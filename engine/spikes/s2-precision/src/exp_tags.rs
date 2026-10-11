//! AVAssetWriter tags: which color tags a VTCompressionSession + AVAssetWriter export
//! actually writes to disk (MOV `colr` `nclc`/`nclx`, `gama`, H.264 VUI), per codec and
//! container, for the candidate "Rec.709 (QuickTime and Safari)" tags (section 9).

use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::CFString;
use objc2_core_video::*;
use objc2_foundation::NSNumber;
use objc2_video_toolbox::*;
use serde::Serialize;

use crate::avio;
use crate::cf::{self, Result, err};
use crate::gpu::{Gpu, PipeParams, Precision};
use crate::material;
use crate::pixbuf::{self, PixelBuffer};
use crate::vt;

/// A candidate tag set. `transfer: None` writes no transfer function.
#[derive(Clone, Copy)]
pub struct Tags {
    pub name: &'static str,
    pub primaries: &'static CFString,
    pub transfer: Option<&'static CFString>,
    pub matrix: &'static CFString,
    pub gamma: Option<f64>,
}

pub fn candidates() -> Vec<Tags> {
    unsafe {
        vec![
            Tags {
                name: "1-1-1 (Rec.709 broadcast)",
                primaries: kCVImageBufferColorPrimaries_ITU_R_709_2,
                transfer: Some(kCVImageBufferTransferFunction_ITU_R_709_2),
                matrix: kCVImageBufferYCbCrMatrix_ITU_R_709_2,
                gamma: None,
            },
            Tags {
                name: "1-13-1 (sRGB transfer)",
                primaries: kCVImageBufferColorPrimaries_ITU_R_709_2,
                transfer: Some(kCVImageBufferTransferFunction_sRGB),
                matrix: kCVImageBufferYCbCrMatrix_ITU_R_709_2,
                gamma: None,
            },
            Tags {
                name: "1-2-1 + gamma 2.2",
                primaries: kCVImageBufferColorPrimaries_ITU_R_709_2,
                transfer: Some(kCVImageBufferTransferFunction_UseGamma),
                matrix: kCVImageBufferYCbCrMatrix_ITU_R_709_2,
                gamma: Some(2.2),
            },
            Tags {
                name: "1-?-1 (no transfer function)",
                primaries: kCVImageBufferColorPrimaries_ITU_R_709_2,
                transfer: None,
                matrix: kCVImageBufferYCbCrMatrix_ITU_R_709_2,
                gamma: None,
            },
        ]
    }
}

fn any(s: &CFString) -> Retained<AnyObject> {
    vt::cf_str(s)
}

fn num(v: f64) -> Retained<AnyObject> {
    Retained::into_super(Retained::into_super(NSNumber::new_f64(v))).into()
}

/// Output codec of a tagged export.
#[derive(Clone, Copy, Serialize, PartialEq, Debug)]
pub enum Codec {
    ProRes422HQ,
    H264,
    Hevc10,
}

impl Codec {
    pub fn input_format(self) -> u32 {
        match self {
            Codec::ProRes422HQ => pixbuf::V216,
            Codec::H264 => pixbuf::V420,
            Codec::Hevc10 => pixbuf::X420,
        }
    }
}

/// Encode engine encoder-input buffers with `tags` (session properties and buffer
/// attachments) and mux with AVAssetWriter.
pub fn encode_tagged(path: &Path, mp4: bool, codec: Codec, frames: &[PixelBuffer], tags: &Tags) -> Result<()> {
    for f in frames {
        f.set_attachment(unsafe { kCVImageBufferColorPrimariesKey }, tags.primaries.as_ref());
        f.set_attachment(unsafe { kCVImageBufferYCbCrMatrixKey }, tags.matrix.as_ref());
        match tags.transfer {
            Some(t) => f.set_attachment(unsafe { kCVImageBufferTransferFunctionKey }, t.as_ref()),
            None => f.as_cf().remove_attachment(unsafe { kCVImageBufferTransferFunctionKey }),
        }
        match tags.gamma {
            Some(g) => {
                let n = num(g);
                let cf: &objc2_core_foundation::CFType = unsafe { &*(Retained::as_ptr(&n) as *const objc2_core_foundation::CFType) };
                f.set_attachment(unsafe { kCVImageBufferGammaLevelKey }, cf);
            }
            None => f.as_cf().remove_attachment(unsafe { kCVImageBufferGammaLevelKey }),
        }
    }
    let mut properties: Vec<(&CFString, Retained<AnyObject>)> = unsafe {
        vec![(kVTCompressionPropertyKey_ColorPrimaries, any(tags.primaries)), (kVTCompressionPropertyKey_YCbCrMatrix, any(tags.matrix))]
    };
    if let Some(t) = tags.transfer {
        properties.push((unsafe { kVTCompressionPropertyKey_TransferFunction }, any(t)));
    }
    if let Some(g) = tags.gamma {
        properties.push((unsafe { kVTCompressionPropertyKey_GammaLevel }, num(g)));
    }
    let codec_type = match codec {
        Codec::ProRes422HQ => objc2_core_media::kCMVideoCodecType_AppleProRes422HQ,
        Codec::H264 => objc2_core_media::kCMVideoCodecType_H264,
        Codec::Hevc10 => {
            properties.push((unsafe { kVTCompressionPropertyKey_ProfileLevel }, any(unsafe { kVTProfileLevel_HEVC_Main10_AutoLevel })));
            objc2_core_media::kCMVideoCodecType_HEVC
        }
    };
    if codec != Codec::ProRes422HQ {
        properties.push((unsafe { kVTCompressionPropertyKey_AverageBitRate }, num(40_000_000.0)));
    }
    let spec = vt::EncodeSpec {
        codec: codec_type,
        width: frames[0].width(),
        height: frames[0].height(),
        timescale: material::TIMESCALE,
        frame_duration: material::FRAME_DUR,
        properties,
        encoder_id: None,
    };
    let refs: Vec<&PixelBuffer> = frames.iter().collect();
    vt::encode_to_file(path, if mp4 { avio::mp4() } else { avio::mov() }, &spec, &refs)?;
    Ok(())
}

// ---- On-disk parsing ----

#[derive(Serialize, Clone, Debug)]
pub struct Colr {
    pub kind: String,
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: Option<bool>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Vui {
    pub profile_idc: u8,
    pub level_idc: u8,
    pub bit_depth_luma: u32,
    pub chroma_format_idc: u32,
    pub video_signal_type_present: bool,
    pub full_range: Option<bool>,
    pub colour_description: Option<(u8, u8, u8)>,
    pub chroma_loc: Option<(u32, u32)>,
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct OnDisk {
    pub sample_entry: String,
    pub colr: Option<Colr>,
    pub gama: Option<f64>,
    pub fiel: Option<(u8, u8)>,
    pub pasp: Option<(u32, u32)>,
    pub child_boxes: Vec<String>,
    pub h264_vui: Option<Vui>,
    pub mvhd_timescale: Option<u32>,
    pub mdhd_timescale: Option<u32>,
    /// Video `stts` entries: (sample count, sample delta).
    pub stts: Vec<(u32, u32)>,
}

fn be16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}
fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// Child boxes of `d[start..end]`: (type, payload start, end).
fn boxes(d: &[u8], start: usize, end: usize) -> Vec<(String, usize, usize)> {
    let mut out = vec![];
    let mut o = start;
    while o + 8 <= end {
        let mut size = be32(d, o) as usize;
        let kind = String::from_utf8_lossy(&d[o + 4..o + 8]).into_owned();
        let mut hdr = 8;
        if size == 1 {
            size = u64::from_be_bytes(d[o + 8..o + 16].try_into().unwrap()) as usize;
            hdr = 16;
        } else if size == 0 {
            size = end - o;
        }
        if size < hdr || o + size > end {
            break;
        }
        out.push((kind, o + hdr, o + size));
        o += size;
    }
    out
}

fn find<'a>(v: &'a [(String, usize, usize)], k: &str) -> Option<&'a (String, usize, usize)> {
    v.iter().find(|b| b.0 == k)
}

pub fn parse_file(path: &Path) -> Result<OnDisk> {
    let d = std::fs::read(path).map_err(|e| cf::Error(e.to_string()))?;
    let top = boxes(&d, 0, d.len());
    let moov = find(&top, "moov").ok_or_else(|| cf::Error("no moov".into()))?;
    let mut out = OnDisk::default();
    let mv = boxes(&d, moov.1, moov.2);
    if let Some(h) = find(&mv, "mvhd") {
        out.mvhd_timescale = Some(if d[h.1] == 1 { be32(&d, h.1 + 20) } else { be32(&d, h.1 + 12) });
    }
    for trak in mv.iter().filter(|b| b.0 == "trak") {
        let t = boxes(&d, trak.1, trak.2);
        let Some(mdia) = find(&t, "mdia") else { continue };
        let m = boxes(&d, mdia.1, mdia.2);
        let Some(hdlr) = find(&m, "hdlr") else { continue };
        if &d[hdlr.1 + 8..hdlr.1 + 12] != b"vide" {
            continue;
        }
        if let Some(h) = find(&m, "mdhd") {
            out.mdhd_timescale = Some(if d[h.1] == 1 { be32(&d, h.1 + 20) } else { be32(&d, h.1 + 12) });
        }
        let minf = find(&m, "minf").ok_or_else(|| cf::Error("no minf".into()))?;
        let mi = boxes(&d, minf.1, minf.2);
        let stbl = find(&mi, "stbl").ok_or_else(|| cf::Error("no stbl".into()))?;
        let st = boxes(&d, stbl.1, stbl.2);
        if let Some(t) = find(&st, "stts") {
            let n = be32(&d, t.1 + 4) as usize;
            out.stts = (0..n).map(|i| (be32(&d, t.1 + 8 + i * 8), be32(&d, t.1 + 12 + i * 8))).collect();
        }
        let stsd = find(&st, "stsd").ok_or_else(|| cf::Error("no stsd".into()))?;
        let entries = boxes(&d, stsd.1 + 8, stsd.2);
        let e = entries.first().ok_or_else(|| cf::Error("empty stsd".into()))?;
        out.sample_entry = e.0.clone();
        // VisualSampleEntry: 78 bytes of fields before child boxes.
        let kids = boxes(&d, e.1 + 78, e.2);
        out.child_boxes = kids.iter().map(|k| k.0.clone()).collect();
        for (k, s, end) in &kids {
            match k.as_str() {
                "colr" => {
                    let kind = String::from_utf8_lossy(&d[*s..s + 4]).into_owned();
                    if kind == "nclc" || kind == "nclx" {
                        out.colr = Some(Colr {
                            primaries: be16(&d, s + 4),
                            transfer: be16(&d, s + 6),
                            matrix: be16(&d, s + 8),
                            full_range: (kind == "nclx" && end - s > 10).then(|| d[s + 10] & 0x80 != 0),
                            kind,
                        });
                    } else {
                        out.colr = Some(Colr { kind, primaries: 0, transfer: 0, matrix: 0, full_range: None });
                    }
                }
                "gama" => out.gama = Some(be32(&d, *s) as f64 / 65536.0),
                "fiel" => out.fiel = Some((d[*s], d[s + 1])),
                "pasp" => out.pasp = Some((be32(&d, *s), be32(&d, s + 4))),
                "avcC" => {
                    let nsps = (d[s + 5] & 0x1f) as usize;
                    if nsps > 0 {
                        let len = be16(&d, s + 6) as usize;
                        out.h264_vui = Some(parse_sps(&d[s + 8..s + 8 + len]));
                    }
                }
                _ => {}
            }
        }
        break;
    }
    Ok(out)
}

/// RBSP bit reader with emulation-prevention bytes removed.
struct Bits {
    d: Vec<u8>,
    pos: usize,
}

impl Bits {
    fn new(nal: &[u8]) -> Self {
        let mut d = Vec::with_capacity(nal.len());
        let mut zeros = 0;
        for &b in nal {
            if zeros >= 2 && b == 3 {
                zeros = 0;
                continue;
            }
            zeros = if b == 0 { zeros + 1 } else { 0 };
            d.push(b);
        }
        Self { d, pos: 0 }
    }
    fn u(&mut self, n: u32) -> std::result::Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.d.get(self.pos / 8).ok_or("end of SPS")?;
            v = (v << 1) | ((byte >> (7 - self.pos % 8)) & 1) as u32;
            self.pos += 1;
        }
        Ok(v)
    }
    fn flag(&mut self) -> std::result::Result<bool, String> {
        Ok(self.u(1)? == 1)
    }
    fn ue(&mut self) -> std::result::Result<u32, String> {
        let mut zeros = 0;
        while self.u(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err("bad exp-Golomb".into());
            }
        }
        Ok((1u32 << zeros) - 1 + self.u(zeros)?)
    }
    fn se(&mut self) -> std::result::Result<i32, String> {
        let k = self.ue()? as i64;
        Ok(if k % 2 == 1 { ((k + 1) / 2) as i32 } else { -(k / 2) as i32 })
    }
}

/// Parse an H.264 SPS (with NAL header) far enough to read the VUI video signal type.
pub fn parse_sps(nal: &[u8]) -> Vui {
    let mut v = Vui::default();
    let r = (|| -> std::result::Result<(), String> {
        let mut b = Bits::new(&nal[1..]);
        v.profile_idc = b.u(8)? as u8;
        b.u(8)?;
        v.level_idc = b.u(8)? as u8;
        b.ue()?;
        v.chroma_format_idc = 1;
        v.bit_depth_luma = 8;
        if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&v.profile_idc) {
            v.chroma_format_idc = b.ue()?;
            if v.chroma_format_idc == 3 {
                b.flag()?;
            }
            v.bit_depth_luma = b.ue()? + 8;
            b.ue()?;
            b.flag()?;
            if b.flag()? {
                let lists = if v.chroma_format_idc == 3 { 12 } else { 8 };
                for i in 0..lists {
                    if b.flag()? {
                        let size = if i < 6 { 16 } else { 64 };
                        let (mut last, mut next) = (8i32, 8i32);
                        for _ in 0..size {
                            if next != 0 {
                                next = (last + b.se()? + 256) % 256;
                            }
                            last = if next == 0 { last } else { next };
                        }
                    }
                }
            }
        }
        b.ue()?;
        let poc = b.ue()?;
        if poc == 0 {
            b.ue()?;
        } else if poc == 1 {
            b.flag()?;
            b.se()?;
            b.se()?;
            let n = b.ue()?;
            for _ in 0..n {
                b.se()?;
            }
        }
        b.ue()?;
        b.flag()?;
        b.ue()?;
        b.ue()?;
        if !b.flag()? {
            b.flag()?;
        }
        b.flag()?;
        if b.flag()? {
            for _ in 0..4 {
                b.ue()?;
            }
        }
        if !b.flag()? {
            return Ok(());
        }
        if b.flag()? && b.u(8)? == 255 {
            b.u(32)?;
        }
        if b.flag()? {
            b.flag()?;
        }
        v.video_signal_type_present = b.flag()?;
        if v.video_signal_type_present {
            b.u(3)?;
            v.full_range = Some(b.flag()?);
            if b.flag()? {
                v.colour_description = Some((b.u(8)? as u8, b.u(8)? as u8, b.u(8)? as u8));
            }
        }
        if b.flag()? {
            v.chroma_loc = Some((b.ue()?, b.ue()?));
        }
        Ok(())
    })();
    if let Err(e) = r {
        v.error = Some(e);
    }
    v
}

// ---- The experiment ----

#[derive(Serialize)]
pub struct TagRow {
    pub tags: String,
    pub codec: Codec,
    pub container: String,
    pub encoded: bool,
    pub error: Option<String>,
    pub on_disk: Option<OnDisk>,
    /// Color-related format description extensions AVFoundation reads back.
    pub read_back: Vec<(String, String)>,
}

/// A small engine-rendered test frame set for tag checks.
pub fn test_frames(gpu: &Gpu, format: u32, n: usize) -> Result<Vec<PixelBuffer>> {
    let src = PixelBuffer::new(material::W, material::H, pixbuf::RGBA64)?;
    let mut d = vec![0u16; material::W * material::H * 4];
    for y in 0..material::H {
        for x in 0..material::W {
            let i = (y * material::W + x) * 4;
            d[i] = (x * 65535 / (material::W - 1)) as u16;
            d[i + 1] = (y * 65535 / (material::H - 1)) as u16;
            d[i + 2] = 32768;
            d[i + 3] = 65535;
        }
    }
    src.write_u16(None, &d)?;
    let p = PipeParams::standard(Precision::F32);
    (0..n).map(|_| gpu.engine_convert(&src, format, &p)).collect()
}

pub fn run(gpu: &Gpu, dir: &Path) -> Result<Vec<TagRow>> {
    std::fs::create_dir_all(dir).map_err(|e| cf::Error(e.to_string()))?;
    let mut rows = vec![];
    for (codec, mp4) in [(Codec::ProRes422HQ, false), (Codec::H264, false), (Codec::H264, true), (Codec::Hevc10, false), (Codec::Hevc10, true)] {
        let frames = test_frames(gpu, codec.input_format(), 4)?;
        for tags in candidates() {
            let ext = if mp4 { "mp4" } else { "mov" };
            let path: PathBuf = dir.join(format!("tags_{:?}_{}.{ext}", codec, tags.name.split(' ').next().unwrap().replace('?', "x")));
            let r = encode_tagged(&path, mp4, codec, &frames, &tags);
            let (on_disk, read_back, error) = match r {
                Ok(()) => {
                    let od = parse_file(&path)?;
                    let rb = avio::track_info(&path)?
                        .extensions
                        .into_iter()
                        .filter(|(k, _)| k.contains("Color") || k.contains("Transfer") || k.contains("Matrix") || k.contains("Gamma") || k.contains("FullRange") || k.contains("Chroma"))
                        .collect();
                    (Some(od), rb, None)
                }
                Err(e) => (None, vec![], Some(e.0)),
            };
            rows.push(TagRow {
                tags: tags.name.into(),
                codec,
                container: if mp4 { "MP4".into() } else { "MOV".into() },
                encoded: error.is_none(),
                error,
                on_disk,
                read_back,
            });
        }
    }
    // AVAssetWriter's default timescales, for comparison (no setMovieTimeScale/setMediaTimeScale).
    let frames = test_frames(gpu, pixbuf::V420, 4)?;
    let path = dir.join("tags_default_timescale.mp4");
    let samples = {
        let spec = vt::EncodeSpec {
            codec: objc2_core_media::kCMVideoCodecType_H264,
            width: material::W,
            height: material::H,
            timescale: material::TIMESCALE,
            frame_duration: material::FRAME_DUR,
            properties: vec![],
            encoder_id: None,
        };
        let refs: Vec<&PixelBuffer> = frames.iter().collect();
        vt::encode_samples(&spec, &refs)?
    };
    let fd = unsafe { samples[0].format_description() }.ok_or_else(|| cf::Error("no fd".into()))?;
    let mut w = avio::VideoWriter::passthrough(&path, avio::mp4(), &fd, 0)?;
    for sb in &samples {
        w.append_sample(sb)?;
    }
    w.finish()?;
    rows.push(TagRow {
        tags: "AVAssetWriter default timescales (none set), untagged".into(),
        codec: Codec::H264,
        container: "MP4".into(),
        encoded: true,
        error: None,
        on_disk: Some(parse_file(&path)?),
        read_back: vec![],
    });
    if rows.iter().all(|r| !r.encoded) {
        return err("no tagged export succeeded");
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An x264-style High-profile SPS with VUI colour description 1-1-1, limited range.
    #[test]
    fn parses_vui_colour_description() {
        // Built bit by bit: High (100), level 40, sps_id 0, chroma 1, depth 8/8, no
        // scaling, log2_max_frame_num 4, poc 0 (log2 4), refs 4, gaps 0, 120×68 MBs,
        // frame_mbs_only, direct8x8, crop to 1080, VUI with video signal type and colour.
        let mut bits = String::new();
        let ue = |v: u32| {
            let x = v + 1;
            let n = 32 - x.leading_zeros();
            format!("{}{:0width$b}", "0".repeat((n - 1) as usize), x, width = n as usize)
        };
        let pieces: Vec<String> = vec![
            format!("{:08b}{:08b}{:08b}", 100, 0, 40),
            ue(0), ue(1), ue(0), ue(0), "0".into(), "0".into(),
            ue(0), ue(0), ue(0), ue(4), "0".into(), ue(119), ue(67), "1".into(), "1".into(),
            "1".into(), ue(0), ue(0), ue(0), ue(4),
            "1".into(), "0".into(), "0".into(), "1".into(), "101".into(), "0".into(), "1".into(),
            format!("{:08b}{:08b}{:08b}", 1, 13, 1),
            "0".into(),
            "1".into(),
        ];
        bits += &pieces.concat();
        while bits.len() % 8 != 0 {
            bits.push('0');
        }
        let mut nal = vec![0x67u8];
        nal.extend(bits.as_bytes().chunks(8).map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 2).unwrap()));
        let v = parse_sps(&nal);
        assert_eq!(v.error, None);
        assert_eq!(v.profile_idc, 100);
        assert_eq!(v.full_range, Some(false));
        assert_eq!(v.colour_description, Some((1, 13, 1)));
    }
}
