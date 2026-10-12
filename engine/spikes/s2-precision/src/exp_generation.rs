//! Codec generation (section 2): the engine's ProRes and H.264 exports versus Apple's
//! re-encode (`AVAssetExportSession`) of the same source. Engine path: VideoToolbox
//! decode → engine (f32, no effects) → encoder-input buffer → VTCompressionSession →
//! AVAssetWriter. Both outputs are decoded by VideoToolbox and compared with each
//! other and with the reference decode of the source.

use std::path::{Path, PathBuf};

use objc2_foundation::NSString;
use serde::Serialize;

use crate::avio::{self, TrackInfo};
use crate::cf::{self, Result};
use crate::color::DiffStats;
use crate::gpu::{Gpu, PipeParams, Precision};
use crate::material::{self, Material};
use crate::pixbuf::{self, PixelBuffer};
use crate::vt;

#[derive(Serialize, Clone, Default)]
pub struct Channels {
    pub y: DiffStats,
    pub cb: DiffStats,
    pub cr: DiffStats,
    pub a: Option<DiffStats>,
    /// Pixels whose color was not compared because α = 0 in either frame.
    pub alpha_zero_skipped: u64,
}

impl Channels {
    fn all(&self) -> DiffStats {
        let mut d = self.y.clone();
        d.merge(&self.cb);
        d.merge(&self.cr);
        d
    }
}

#[derive(Serialize, Clone, Copy)]
pub struct Psnr {
    pub y: f64,
    pub cb: f64,
    pub cr: f64,
    pub ycbcr: f64,
}

fn psnr(c: &Channels, peak: f64) -> Psnr {
    Psnr { y: c.y.psnr(peak), cb: c.cb.psnr(peak), cr: c.cr.psnr(peak), ycbcr: c.all().psnr(peak) }
}

#[derive(Serialize)]
pub struct GenCase {
    pub name: String,
    pub preset: String,
    pub apple: TrackInfo,
    pub engine: TrackInfo,
    pub engine_settings: String,
    pub frames_apple: usize,
    pub frames_engine: usize,
    /// Comparison depth in bits (codes compared after rounding to this depth).
    pub depth: u32,
    /// Engine output versus Apple output, decoded.
    pub engine_vs_apple: Channels,
    pub max_cv_engine_vs_apple: i64,
    /// Engine versus Apple at the codec's native depth (12-bit for 4444, 10-bit otherwise).
    pub native_depth: u32,
    pub max_cv_engine_vs_apple_native: i64,
    pub mismatches_engine_vs_apple_native: u64,
    pub psnr_engine_vs_reference: Psnr,
    pub psnr_apple_vs_reference: Psnr,
    /// Against the original synthetic frames that were encoded to make the source.
    pub psnr_engine_vs_original: Psnr,
    pub psnr_apple_vs_original: Psnr,
    pub psnr_delta_vs_original_db: f64,
    /// (6·Y′ + Cb + Cr) / 8 PSNR delta against the reference decode.
    pub psnr_delta_weighted_611_db: f64,
    /// PSNR(engine) − PSNR(Apple) on Y′CbCr; the bar is |Δ| ≤ 0.1 dB (or engine better).
    pub psnr_delta_db: f64,
    pub psnr_delta_y_db: f64,
    pub pass_max_1cv: bool,
    pub pass_psnr_0_1db: bool,
    /// Control: VideoToolbox encode of the decoded source buffers themselves (no engine).
    pub direct_vs_apple_max_cv: i64,
    pub direct_vs_apple_mismatches: u64,
    pub psnr_direct_vs_reference: Psnr,
    /// Control: the engine export run twice (encoder determinism).
    pub engine_repeat_max_cv: i64,
    pub engine_repeat_mismatches: u64,
    /// Engine encoder input versus the decoded source buffers at the input's depth.
    pub encoder_input_vs_decode_mismatches: u64,
    /// Alpha sources: engine versus Apple at 10-bit, excluding pixels within N px of an
    /// α = 0 pixel (whose color the engine does not carry), as (N, max CV, mismatches).
    pub engine_vs_apple_away_from_alpha0: Vec<(usize, i64, u64)>,
    /// H.264: requested AverageBitRate per iteration and the resulting data rate.
    pub rate_matching: Vec<(i64, f64)>,
    /// Which decode format, re-encoded by VideoToolbox, reproduces Apple's output.
    pub apple_path_probe: Vec<PathProbe>,
    pub notes: Vec<String>,
}

#[derive(Serialize)]
pub struct PathProbe {
    pub path: String,
    pub error: Option<String>,
    pub max_cv_vs_apple: Option<i64>,
    pub mismatches_vs_apple: Option<u64>,
    pub psnr_vs_reference: Option<f64>,
}

/// Separable box dilation of a boolean mask by radius r.
fn dilate(mask: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let mut tmp = vec![false; w * h];
    for y in 0..h {
        let row = &mask[y * w..(y + 1) * w];
        let mut pre = vec![0u32; w + 1];
        for x in 0..w {
            pre[x + 1] = pre[x] + row[x] as u32;
        }
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r + 1).min(w));
            tmp[y * w + x] = pre[b] > pre[a];
        }
    }
    let mut out = vec![false; w * h];
    for x in 0..w {
        let mut pre = vec![0u32; h + 1];
        for y in 0..h {
            pre[y + 1] = pre[y] + tmp[y * w + x] as u32;
        }
        for y in 0..h {
            let (a, b) = (y.saturating_sub(r), (y + r + 1).min(h));
            out[y * w + x] = pre[b] > pre[a];
        }
    }
    out
}

fn away_from_alpha0(e: &[vt::Frame], a: &[vt::Frame], radii: &[usize]) -> Result<Vec<(usize, i64, u64)>> {
    let mut out: Vec<(usize, i64, u64)> = radii.iter().map(|&r| (r, 0, 0)).collect();
    for (fe, fa) in e.iter().zip(a) {
        let (w, h) = (fa.buffer.width(), fa.buffer.height());
        let (x, _, _) = fe.buffer.read_u16(None)?;
        let (y, _, _) = fa.buffer.read_u16(None)?;
        let mask: Vec<bool> = y.chunks_exact(4).map(|p| p[0] == 0).collect();
        for o in out.iter_mut() {
            let m = dilate(&mask, w, h, o.0);
            for (i, (p, r)) in x.chunks_exact(4).zip(y.chunks_exact(4)).enumerate() {
                if m[i] {
                    continue;
                }
                for c in 1..4 {
                    let d = (q(p[c], 10) - q(r[c], 10)).abs();
                    if d != 0 {
                        o.2 += 1;
                        o.1 = o.1.max(d);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Round a 16-bit MSB-aligned sample to `depth` bits.
fn q(v: u16, depth: u32) -> i64 {
    let shift = 16 - depth;
    if shift == 0 {
        return v as i64;
    }
    ((v as i64) + (1 << (shift - 1))) >> shift
}

/// Samples of a frame as 16-bit MSB-aligned planes: Y′ per pixel, Cb and Cr per chroma
/// site, and alpha per pixel when present.
struct Planes {
    y: Vec<u16>,
    cb: Vec<u16>,
    cr: Vec<u16>,
    a: Option<Vec<u16>>,
    /// Pixel index of each chroma site (for alpha masking).
    chroma_px: usize,
}

fn planes(pb: &PixelBuffer) -> Result<Planes> {
    Ok(match pb.format() {
        pixbuf::Y416 => {
            let (d, _, _) = pb.read_u16(None)?;
            let mut p = Planes { y: vec![], cb: vec![], cr: vec![], a: Some(vec![]), chroma_px: 1 };
            for px in d.chunks_exact(4) {
                p.a.as_mut().unwrap().push(px[0]);
                p.y.push(px[1]);
                p.cb.push(px[2]);
                p.cr.push(px[3]);
            }
            p
        }
        pixbuf::X422 => {
            let (y, _, _) = pb.read_u16(Some(0))?;
            let (c, _, _) = pb.read_u16(Some(1))?;
            Planes {
                y,
                cb: c.iter().step_by(2).copied().collect(),
                cr: c.iter().skip(1).step_by(2).copied().collect(),
                a: None,
                chroma_px: 2,
            }
        }
        pixbuf::V216 => {
            let (d, _, _) = pb.read_u16(None)?;
            let mut p = Planes { y: vec![], cb: vec![], cr: vec![], a: None, chroma_px: 2 };
            for t in d.chunks_exact(4) {
                p.cb.push(t[0]);
                p.y.push(t[1]);
                p.cr.push(t[2]);
                p.y.push(t[3]);
            }
            p
        }
        f => return cf::err(format!("planes: unsupported '{}'", cf::fourcc_str(f))),
    })
}

/// Compare two frames (same subsampling) at `depth` bits.
fn compare(a: &PixelBuffer, b: &PixelBuffer, depth: u32, acc: &mut Channels) -> Result<()> {
    let (pa, pb) = (planes(a)?, planes(b)?);
    if pa.y.len() != pb.y.len() || pa.cb.len() != pb.cb.len() {
        return cf::err("compare: geometry or subsampling differs");
    }
    // Color at α = 0 is not carried by premultiplied working values (section 9).
    let zero = |i: usize| -> bool {
        pa.a.as_ref().map(|v| v[i] == 0).unwrap_or(false) || pb.a.as_ref().map(|v| v[i] == 0).unwrap_or(false)
    };
    if let (Some(x), Some(y)) = (&pa.a, &pb.a) {
        let al = acc.a.get_or_insert_with(DiffStats::default);
        for (p, r) in x.iter().zip(y) {
            al.add(q(*p, depth), q(*r, depth));
        }
    }
    for i in 0..pa.y.len() {
        if zero(i) {
            acc.alpha_zero_skipped += 1;
            continue;
        }
        acc.y.add(q(pa.y[i], depth), q(pb.y[i], depth));
    }
    for i in 0..pa.cb.len() {
        if zero(i * pa.chroma_px) {
            continue;
        }
        acc.cb.add(q(pa.cb[i], depth), q(pb.cb[i], depth));
        acc.cr.add(q(pa.cr[i], depth), q(pb.cr[i], depth));
    }
    Ok(())
}

fn compare_all(a: &[vt::Frame], b: &[vt::Frame], depth: u32) -> Result<Channels> {
    let mut acc = Channels::default();
    for (fa, fb) in a.iter().zip(b) {
        compare(&fa.buffer, &fb.buffer, depth, &mut acc)?;
    }
    Ok(acc)
}

struct Spec<'a> {
    name: &'a str,
    source: &'a Path,
    preset: &'a NSString,
    file_type: &'static objc2_av_foundation::AVFileType,
    /// Engine decode format, encoder-input format, and comparison format.
    decode: u32,
    encoder_input: u32,
    compare_format: u32,
    depth: u32,
    h264: bool,
    probe_apple_path: bool,
    /// Regenerates the original frames (format, alpha) that were encoded into the source.
    original: (u32, bool),
}

fn native_depth_of(format: u32) -> u32 {
    if format == pixbuf::Y416 { 12 } else { 10 }
}

fn max_and_mismatches(c: &Channels) -> (i64, u64) {
    let a = c.all();
    (
        a.max_abs.max(c.a.as_ref().map(|x| x.max_abs).unwrap_or(0)),
        a.mismatches + c.a.as_ref().map(|x| x.mismatches).unwrap_or(0),
    )
}

fn run_case(gpu: &Gpu, out_dir: &Path, s: &Spec) -> Result<GenCase> {
    let stem = s.name.replace([' ', '.', '/', ','], "_");
    let ext = if s.file_type == avio::mp4() { "mp4" } else { "mov" };
    let apple_path: PathBuf = out_dir.join(format!("gen_{stem}_apple.{ext}"));
    let engine_path: PathBuf = out_dir.join(format!("gen_{stem}_engine.{ext}"));
    let mut notes = vec![];

    avio::export_session(s.source, &apple_path, s.preset, s.file_type)?;
    let apple = avio::track_info(&apple_path)?;
    let apple_codec = cf::fourcc(apple.codec.as_bytes().try_into().map_err(|_| cf::Error("codec".into()))?);

    // Engine export of the same source.
    let src = vt::decode_file(s.source, s.decode)?;
    let p = PipeParams::standard(Precision::F32);
    let mut inputs = vec![];
    for f in &src.frames {
        inputs.push(gpu.engine_convert(&f.buffer, s.encoder_input, &p)?);
    }
    let refs: Vec<&PixelBuffer> = inputs.iter().collect();
    let mut input_mismatch = 0u64;
    if s.encoder_input == s.decode {
        for (f, e) in src.frames.iter().zip(&inputs) {
            let planes: Vec<Option<usize>> =
                if e.plane_count() == 0 { vec![None] } else { (0..e.plane_count()).map(Some).collect() };
            for p in planes {
                let (a, _, _) = f.buffer.read_u16(p)?;
                let (b, _, _) = e.read_u16(p)?;
                input_mismatch += a.iter().zip(&b).filter(|(x, y)| x != y).count() as u64;
            }
        }
    }
    let mut settings = format!("codec '{}', Rec.709 1-1-1 tags", apple.codec);
    let rate_cell = std::cell::Cell::new(apple.estimated_data_rate_bps.round() as i64);
    let props = || {
        let mut properties = vec![
            (
                unsafe { objc2_video_toolbox::kVTCompressionPropertyKey_ColorPrimaries },
                vt::cf_str(unsafe { objc2_core_video::kCVImageBufferColorPrimaries_ITU_R_709_2 }),
            ),
            (
                unsafe { objc2_video_toolbox::kVTCompressionPropertyKey_TransferFunction },
                vt::cf_str(unsafe { objc2_core_video::kCVImageBufferTransferFunction_ITU_R_709_2 }),
            ),
            (
                unsafe { objc2_video_toolbox::kVTCompressionPropertyKey_YCbCrMatrix },
                vt::cf_str(unsafe { objc2_core_video::kCVImageBufferYCbCrMatrix_ITU_R_709_2 }),
            ),
        ];
        if s.h264 {
            // Match Apple's measured average bitrate and the High profile.
            let rate = rate_cell.get();
            properties.push((
                unsafe { objc2_video_toolbox::kVTCompressionPropertyKey_AverageBitRate },
                objc2::rc::Retained::into_super(objc2::rc::Retained::into_super(objc2_foundation::NSNumber::new_i64(
                    rate,
                )))
                .into(),
            ));
            properties.push((
                unsafe { objc2_video_toolbox::kVTCompressionPropertyKey_ProfileLevel },
                vt::cf_str(unsafe { objc2_video_toolbox::kVTProfileLevel_H264_High_AutoLevel }),
            ));
        }
        vt::EncodeSpec {
            codec: apple_codec,
            width: material::W,
            height: material::H,
            timescale: material::TIMESCALE,
            frame_duration: material::FRAME_DUR,
            properties,
            encoder_id: None,
        }
    };
    let mut rate_matching = vec![];
    vt::encode_to_file(&engine_path, s.file_type, &props(), &refs)?;
    let mut engine = avio::track_info(&engine_path)?;
    if s.h264 {
        // Iterate the requested average bitrate until the engine file's data rate is
        // within 1% of Apple's, so both are compared at equal bits.
        let target = apple.estimated_data_rate_bps;
        rate_matching.push((rate_cell.get(), engine.estimated_data_rate_bps));
        for _ in 0..8 {
            let err = engine.estimated_data_rate_bps / target - 1.0;
            if err.abs() <= 0.01 {
                break;
            }
            let next = (rate_cell.get() as f64 * target / engine.estimated_data_rate_bps).round() as i64;
            rate_cell.set(next);
            vt::encode_to_file(&engine_path, s.file_type, &props(), &refs)?;
            engine = avio::track_info(&engine_path)?;
            rate_matching.push((next, engine.estimated_data_rate_bps));
        }
        settings += &format!(
            ", High profile, AverageBitRate {} chosen so the data rate matches Apple's {:.0} bps",
            rate_cell.get(),
            target
        );
    }
    let repeat_path = out_dir.join(format!("gen_{stem}_engine_repeat.{ext}"));
    vt::encode_to_file(&repeat_path, s.file_type, &props(), &refs)?;
    let direct_path = out_dir.join(format!("gen_{stem}_direct.{ext}"));
    let direct_refs: Vec<&PixelBuffer> = src.frames.iter().map(|f| &f.buffer).collect();
    if s.encoder_input == s.decode {
        vt::encode_to_file(&direct_path, s.file_type, &props(), &direct_refs)?;
    } else {
        notes.push(
            "direct control encodes the decoded 4:2:2 buffers; VideoToolbox converts them to 4:2:0 itself".into(),
        );
        vt::encode_to_file(&direct_path, s.file_type, &props(), &direct_refs)?;
    }

    let reference = vt::decode_file(s.source, s.compare_format)?.frames;
    let da = vt::decode_file(&apple_path, s.compare_format)?.frames;
    let de = vt::decode_file(&engine_path, s.compare_format)?.frames;
    let dr = vt::decode_file(&repeat_path, s.compare_format)?.frames;
    let dd = vt::decode_file(&direct_path, s.compare_format)?.frames;
    let rep = compare_all(&de, &dr, native_depth_of(s.compare_format))?;
    let dva = compare_all(&dd, &da, s.depth)?;
    let dref = compare_all(&dd, &reference, s.depth)?;
    if da.len() != reference.len() || de.len() != reference.len() {
        notes.push(format!("frame counts differ: source {}, Apple {}, engine {}", reference.len(), da.len(), de.len()));
    }
    let peak = ((1u32 << s.depth) - 1) as f64;
    let eva = compare_all(&de, &da, s.depth)?;
    let er = compare_all(&de, &reference, s.depth)?;
    let ar = compare_all(&da, &reference, s.depth)?;
    let pe = psnr(&er, peak);
    let pa = psnr(&ar, peak);
    let originals: Vec<vt::Frame> = (0..reference.len())
        .map(|f| {
            material::natural_frame(f, s.original.0, s.original.1)
                .map(|b| vt::Frame { pts: reference[f].pts, buffer: b })
        })
        .collect::<Result<_>>()?;
    let peo = psnr(&compare_all(&de, &originals, s.depth)?, peak);
    let pao = psnr(&compare_all(&da, &originals, s.depth)?, peak);
    let w611 = |p: &Psnr| (6.0 * p.y + p.cb + p.cr) / 8.0;
    let max_cv = eva.all().max_abs.max(eva.a.as_ref().map(|a| a.max_abs).unwrap_or(0));
    let native_depth = native_depth_of(s.compare_format);
    let away = if s.compare_format == pixbuf::Y416 && eva.alpha_zero_skipped > 0 {
        away_from_alpha0(&de, &da, &[1, 16, 64, 128])?
    } else {
        vec![]
    };
    let mut probes = vec![];
    if !s.h264 && s.probe_apple_path {
        let formats = [
            pixbuf::Y416,
            objc2_core_video::kCVPixelFormatType_422YpCbCr10,
            objc2_core_video::kCVPixelFormatType_422YpCbCr16,
            objc2_core_video::kCVPixelFormatType_422YpCbCr8,
            objc2_core_video::kCVPixelFormatType_4444AYpCbCrFloat,
            pixbuf::B64A,
            pixbuf::RGBA64,
            pixbuf::RGBA_HALF,
            pixbuf::BGRA,
        ];
        for fmt in formats {
            let path = out_dir.join(format!("probe_{stem}_{}.mov", cf::fourcc_str(fmt)));
            let r = (|| -> Result<(i64, u64, f64)> {
                let dec = vt::decode_file(s.source, fmt)?;
                let bufs: Vec<&PixelBuffer> = dec.frames.iter().map(|f| &f.buffer).collect();
                vt::encode_to_file(&path, s.file_type, &props(), &bufs)?;
                let d = vt::decode_file(&path, s.compare_format)?.frames;
                let c = compare_all(&d, &da, s.depth)?;
                let (mx, mm) = max_and_mismatches(&c);
                Ok((mx, mm, compare_all(&d, &reference, s.depth)?.all().psnr(peak)))
            })();
            probes.push(match r {
                Ok((mx, mm, p)) => PathProbe {
                    path: format!("VT decode to '{}' → VTCompressionSession", cf::fourcc_str(fmt)),
                    error: None,
                    max_cv_vs_apple: Some(mx),
                    mismatches_vs_apple: Some(mm),
                    psnr_vs_reference: Some(p),
                },
                Err(e) => PathProbe {
                    path: format!("VT decode to '{}' → VTCompressionSession", cf::fourcc_str(fmt)),
                    error: Some(e.0),
                    max_cv_vs_apple: None,
                    mismatches_vs_apple: None,
                    psnr_vs_reference: None,
                },
            });
            avio::remove(&path);
        }
    }
    let native = compare_all(&de, &da, native_depth)?;
    let native_all = native.all();
    let delta = pe.ycbcr - pa.ycbcr;
    let delta_y = pe.y - pa.y;
    let both_lossless = pe.ycbcr.is_infinite() && pa.ycbcr.is_infinite();
    Ok(GenCase {
        name: s.name.into(),
        preset: s.preset.to_string(),
        apple,
        engine,
        engine_settings: settings,
        frames_apple: da.len(),
        frames_engine: de.len(),
        depth: s.depth,
        max_cv_engine_vs_apple: max_cv,
        native_depth,
        max_cv_engine_vs_apple_native: native_all.max_abs.max(native.a.as_ref().map(|a| a.max_abs).unwrap_or(0)),
        mismatches_engine_vs_apple_native: native_all.mismatches + native.a.as_ref().map(|a| a.mismatches).unwrap_or(0),
        pass_max_1cv: max_cv <= 1,
        pass_psnr_0_1db: both_lossless || delta >= -0.1,
        psnr_delta_db: delta,
        psnr_delta_y_db: delta_y,
        engine_vs_apple: eva,
        psnr_engine_vs_reference: pe,
        psnr_delta_vs_original_db: peo.ycbcr - pao.ycbcr,
        psnr_delta_weighted_611_db: w611(&pe) - w611(&pa),
        psnr_engine_vs_original: peo,
        psnr_apple_vs_original: pao,
        psnr_apple_vs_reference: pa,
        direct_vs_apple_max_cv: max_and_mismatches(&dva).0,
        direct_vs_apple_mismatches: max_and_mismatches(&dva).1,
        psnr_direct_vs_reference: psnr(&dref, peak),
        engine_repeat_max_cv: max_and_mismatches(&rep).0,
        engine_repeat_mismatches: max_and_mismatches(&rep).1,
        encoder_input_vs_decode_mismatches: input_mismatch,
        engine_vs_apple_away_from_alpha0: away,
        rate_matching,
        apple_path_probe: probes,
        notes,
    })
}

pub fn run(gpu: &Gpu, m: &Material) -> Result<Vec<GenCase>> {
    let out_dir = m.dir.join("../generation");
    std::fs::create_dir_all(&out_dir).map_err(|e| cf::Error(e.to_string()))?;
    let specs = unsafe {
        [
            Spec {
                name: "ProRes 422 HQ source, ProRes 422 preset, engine via x422",
                source: &m.natural_422hq,
                preset: objc2_av_foundation::AVAssetExportPresetAppleProRes422LPCM,
                file_type: avio::mov(),
                decode: pixbuf::X422,
                encoder_input: pixbuf::X422,
                compare_format: pixbuf::V216,
                depth: 10,
                h264: false,
                probe_apple_path: true,
                original: (pixbuf::X422, false),
            },
            Spec {
                name: "ProRes 422 HQ source, ProRes 422 preset, engine via v216",
                source: &m.natural_422hq,
                preset: objc2_av_foundation::AVAssetExportPresetAppleProRes422LPCM,
                file_type: avio::mov(),
                decode: pixbuf::V216,
                encoder_input: pixbuf::V216,
                compare_format: pixbuf::V216,
                depth: 10,
                h264: false,
                probe_apple_path: false,
                original: (pixbuf::X422, false),
            },
            Spec {
                name: "ProRes 4444 XQ source, ProRes 4444 preset",
                source: &m.natural_4444xq,
                preset: objc2_av_foundation::AVAssetExportPresetAppleProRes4444LPCM,
                file_type: avio::mov(),
                decode: pixbuf::Y416,
                encoder_input: pixbuf::Y416,
                compare_format: pixbuf::Y416,
                depth: 10,
                h264: false,
                probe_apple_path: false,
                original: (pixbuf::Y416, false),
            },
            Spec {
                name: "ProRes 4444 XQ alpha source, ProRes 4444 preset",
                source: &m.natural_4444xq_alpha,
                preset: objc2_av_foundation::AVAssetExportPresetAppleProRes4444LPCM,
                file_type: avio::mov(),
                decode: pixbuf::Y416,
                encoder_input: pixbuf::Y416,
                compare_format: pixbuf::Y416,
                depth: 10,
                h264: false,
                probe_apple_path: false,
                original: (pixbuf::Y416, true),
            },
            Spec {
                name: "ProRes 422 HQ source, H.264 1920x1080 preset",
                source: &m.natural_422hq,
                preset: objc2_av_foundation::AVAssetExportPreset1920x1080,
                file_type: avio::mp4(),
                decode: pixbuf::X422,
                encoder_input: pixbuf::V420,
                compare_format: pixbuf::V216,
                depth: 10,
                h264: true,
                probe_apple_path: false,
                original: (pixbuf::X422, false),
            },
        ]
    };
    let mut out = vec![];
    for s in specs.iter() {
        match run_case(gpu, &out_dir, s) {
            Ok(c) => out.push(c),
            Err(e) => return cf::err(format!("{}: {e}", s.name)),
        }
    }
    Ok(out)
}
