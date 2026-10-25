//! Section 2's color bars through the renderer (Standard color):
//!
//! - invariant (2): solids set to `#RRGGBB` decode to the same codes —
//!   exactly for sRGB PNG, within 1 at 10 bits and above, within 2 for
//!   8-bit limited-range Y′CbCr with greys exactly neutral — measured on
//!   the renderer's working output through `pm-export`'s encoder input
//!   (the CPU reference of the GPU pass) and a reference decode;
//! - pipeline transparency with amendment 8: ProRes 4444 XQ → engine at
//!   32-bit → `y416`, and ProRes 422 HQ → `v216` (amendment 6), equal a
//!   VideoToolbox decode clipped to the codable range, sample for sample;
//! - invariant (1) for 4:2:0: an 8-bit H.264 source through `420v` keeps
//!   luma exactly and chroma within 1 CV.
//!
//! Each bar also runs with linear blending on (section 2, "Color
//! invariants, linear blending": bars (1) and (2) also hold): sources and
//! solids are read with E⁻¹, intermediates are linear light, and the frame
//! is encoded with E.
//!
//! The ProRes and H.264 sources come from the media corpus outside the
//! repository (`PM_MEDIA_CORPUS`, default
//! `~/Library/Caches/powermove-engine/media-corpus`); without it those tests
//! report and pass vacuously.

mod support;

use pm_color::transfer::unit_to_code;
use pm_export::convert::{ConvertParams, EncoderInput, WorkingFrame, convert, ycbcr_to_rgb};
use pm_export::settings::{AlphaOutput, ColorTags, InputFormat, WorkingEncoding};
use pm_protocol::color::{MatrixCoefficients, Precision};
use pm_protocol::common::Size;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;
use support::ticks;

/// Block size of the solids grid: wider than the 4:2:0 filter support, so
/// each block's center pixel and chroma sample see only its own color.
const B: usize = 6;
const G: u32 = 16 * B as u32;

/// 256 colors on a 16 × 16 grid of `B` × `B` solids.
fn grid(colors: &[[u8; 3]]) -> serde_json::Value {
    let layers = colors
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let hex = format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
            let (x, y) = ((i % 16 * B) as f64, (i / 16 * B) as f64);
            layer(
                &format!("s{i}"),
                "solid",
                json!({ "color": hex, "w": B, "h": B }),
                x,
                y,
                1.0,
            )
        })
        .collect();
    project(G, G, 1.0, solid_fill("#000000"), layers)
}

fn palettes() -> Vec<Vec<[u8; 3]>> {
    let greys: Vec<[u8; 3]> = (0..=255u8).map(|v| [v, v, v]).collect();
    // A deterministic spread of colors (LCG), plus the extremes.
    let mut state = 0x2545_f491u32;
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 24) as u8
    };
    let mut colors: Vec<[u8; 3]> = (0..250).map(|_| [next(), next(), next()]).collect();
    colors.extend([
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [255, 255, 0],
        [0, 255, 255],
        [255, 0, 255],
    ]);
    vec![greys, colors]
}

fn render(
    renderer: &mut Renderer,
    doc: &serde_json::Value,
    precision: Precision,
    size: (u32, u32),
    t: f64,
    resolver: &MapResolver,
) -> Vec<f32> {
    let mut doc = doc.clone();
    doc["colorDepth"] = json!(if precision == Precision::Float32 {
        32
    } else {
        16
    });
    let request = RenderRequest::at_scale(Size::new(size.0, size.1), 1.0, precision);
    let frame = renderer
        .render(&load(&doc), "Ptest-root", ticks(t), &request, resolver)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    assert_eq!(
        frame.color.format(),
        if precision == Precision::Float32 {
            wgpu::TextureFormat::Rgba32Float
        } else {
            wgpu::TextureFormat::Rgba16Float
        }
    );
    renderer.read_rgba(&frame).unwrap()
}

fn sample(input: &EncoderInput, plane: usize, i: usize) -> u32 {
    match &input.planes[plane].data {
        pm_export::convert::PlaneData::U16(v) => u32::from(v[i]),
        pm_export::convert::PlaneData::U8(v) => u32::from(v[i]),
    }
}

fn params() -> ConvertParams {
    ConvertParams {
        working: WorkingEncoding::Standard,
        alpha: AlphaOutput::None,
        tags: ColorTags::Rec709QuickTime,
        dither: false,
        frame: 0,
    }
}

/// Linear blending on (Settings › Project › Color › Blend in linear light).
fn linear(mut doc: serde_json::Value) -> serde_json::Value {
    doc["color"] = json!({ "mode": "standard", "linearBlending": true });
    doc
}

#[test]
fn invariant_2_solids_decode_to_their_codes() {
    invariant_2(false);
}

#[test]
fn invariant_2_with_linear_blending() {
    invariant_2(true);
}

fn invariant_2(linear_light: bool) {
    let mut r = Renderer::headless().unwrap();
    let none = MapResolver::default();
    let mut worst = [0u32; 4];
    for colors in palettes() {
        let doc = if linear_light {
            linear(grid(&colors))
        } else {
            grid(&colors)
        };
        for precision in [Precision::Float16, Precision::Float32] {
            let rgba = render(&mut r, &doc, precision, (G, G), 0.0, &none);
            let frame = WorkingFrame {
                width: G,
                height: G,
                rgba: &rgba,
            };
            let ycc8 = convert(&frame, InputFormat::V420, &params()).unwrap();
            let v216 = convert(&frame, InputFormat::V216, &params()).unwrap();
            let y416 = convert(&frame, InputFormat::Y416, &params()).unwrap();
            for (i, c) in colors.iter().enumerate() {
                // The block's center: an even column and row (co-sited chroma).
                let (x, y) = (i % 16 * B + 2, i / 16 * B + 2);
                let gw = G as usize;
                let px = (y * gw + x) * 4;
                let working = [rgba[px], rgba[px + 1], rgba[px + 2]];
                // The hex code is the working value (Standard). With linear
                // blending the frame is E(E⁻¹(code)): the codes below.
                for k in 0..3 {
                    if linear_light {
                        break;
                    }
                    let ideal = f32::from(c[k]) / 255.0;
                    if precision == Precision::Float32 {
                        assert_eq!(working[k], ideal, "{c:?} working value");
                    } else {
                        // rgba16float storage: one of the two halves around
                        // the value (the GPU's render-target conversion may
                        // round toward zero).
                        let half = f64::from(ideal) - f64::from(working[k]);
                        assert!(
                            half.abs() <= 2f64.powi(-15).max(f64::from(ideal) * 2f64.powi(-10)),
                            "{c:?} working value at 16-bit: {}",
                            working[k]
                        );
                    }
                }
                // sRGB PNG: exact.
                for k in 0..3 {
                    assert_eq!(
                        unit_to_code(f64::from(working[k]), 8),
                        u32::from(c[k]),
                        "{c:?} sRGB PNG"
                    );
                }
                // Full-range RGB at 10 and 16 bits (32-bit precision exports).
                if precision == Precision::Float32 {
                    for bits in [10u8, 16] {
                        for k in 0..3 {
                            let d = unit_to_code(f64::from(working[k]), bits)
                                .abs_diff(pm_color::standard::ideal_code(c[k], bits));
                            worst[0] = worst[0].max(d);
                            assert!(d <= 1, "{c:?} {bits}-bit RGB off by {d}");
                        }
                    }
                }
                // 8-bit limited Y′CbCr (H.264 `420v`): decode within 2, greys neutral.
                let (cy, cc) = (sample(&ycc8, 0, y * gw + x), (y / 2) * gw + x);
                let (cb, cr) = (sample(&ycc8, 1, cc), sample(&ycc8, 1, cc + 1));
                if c[0] == c[1] && c[1] == c[2] {
                    assert_eq!((cb, cr), (128, 128), "grey {c:?} not neutral at 8 bits");
                }
                let back = ycbcr_to_rgb(
                    (f64::from(cy) - 16.0) / 219.0,
                    (f64::from(cb) - 128.0) / 224.0,
                    (f64::from(cr) - 128.0) / 224.0,
                );
                for k in 0..3 {
                    let d = unit_to_code(back[k], 8).abs_diff(u32::from(c[k]));
                    worst[1] = worst[1].max(d);
                    assert!(d <= 2, "{c:?} 8-bit Y′CbCr off by {d}");
                }
                // 10-bit (`v216`) and 12-bit (`y416`) Y′CbCr at 32-bit
                // precision: decode within 1 of the ideal code, greys neutral.
                if precision == Precision::Float32 {
                    let row = y * gw;
                    let (cb, yy, cr) = (
                        sample(&v216, 0, (row + x) * 2),
                        sample(&v216, 0, (row + x) * 2 + 1),
                        sample(&v216, 0, (row + x) * 2 + 2),
                    );
                    if c[0] == c[1] && c[1] == c[2] {
                        assert_eq!(
                            (cb, cr),
                            (32768, 32768),
                            "grey {c:?} not neutral at 10 bits"
                        );
                    }
                    let back = ycbcr_to_rgb(
                        (f64::from(yy) - 4096.0) / 56064.0,
                        (f64::from(cb) - 32768.0) / 57344.0,
                        (f64::from(cr) - 32768.0) / 57344.0,
                    );
                    for k in 0..3 {
                        let d = unit_to_code(back[k], 10)
                            .abs_diff(pm_color::standard::ideal_code(c[k], 10));
                        worst[2] = worst[2].max(d);
                        assert!(d <= 1, "{c:?} 10-bit Y′CbCr off by {d}");
                    }
                    let i4 = (row + x) * 4;
                    let (yy, cb, cr) = (
                        sample(&y416, 0, i4 + 1),
                        sample(&y416, 0, i4 + 2),
                        sample(&y416, 0, i4 + 3),
                    );
                    if c[0] == c[1] && c[1] == c[2] {
                        assert_eq!(
                            (cb, cr),
                            (32768, 32768),
                            "grey {c:?} not neutral at 12 bits"
                        );
                    }
                    let back = ycbcr_to_rgb(
                        (f64::from(yy) - 4096.0) / 56064.0,
                        (f64::from(cb) - 32768.0) / 57344.0,
                        (f64::from(cr) - 32768.0) / 57344.0,
                    );
                    for k in 0..3 {
                        let d = unit_to_code(back[k], 12)
                            .abs_diff(pm_color::standard::ideal_code(c[k], 12));
                        worst[3] = worst[3].max(d);
                        assert!(d <= 1, "{c:?} 12-bit Y′CbCr off by {d}");
                    }
                }
            }
        }
    }
    eprintln!(
        "worst code differences: RGB 10/16-bit {}, 8-bit Y′CbCr {}, 10-bit Y′CbCr {}, 12-bit Y′CbCr {}",
        worst[0], worst[1], worst[2], worst[3]
    );
}

fn media(name: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("PM_MEDIA_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join("Library/Caches/powermove-engine/media-corpus")
        });
    let p = dir.join(name);
    p.exists().then_some(p)
}

/// A composition showing `path` (w × h) untransformed, full frame, as the
/// only layer.
fn full_frame_video(
    path: &std::path::Path,
    w: u32,
    h: u32,
    dur: f64,
) -> (serde_json::Value, MapResolver) {
    let mut doc = project(
        w,
        h,
        dur,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({ "asset": "clip", "fit": "stretch", "w": w, "h": h, "trim": 0, "speed": 1 }),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            dur,
        )],
    );
    asset(&mut doc, "clip", "video", "mov", w, h, dur);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path.to_path_buf());
    (doc, resolver)
}

/// The reference decode of frame `n` (VideoToolbox, export purpose): the
/// first plane's 16-bit samples, tightly packed.
fn reference_u16(path: &std::path::Path, n: usize) -> (Vec<u16>, String) {
    use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
    let service = DecodeService::new(ServiceConfig::default());
    let src = MediaSource::open(path).unwrap();
    let mut session = service
        .session(&src, 1, pm_media::vt::DecodePurpose::Export)
        .unwrap();
    let frame = session.frame(n).unwrap();
    let label = frame.format.label.clone();
    let samples = frame
        .with_planes(|planes| {
            let p = &planes[0];
            let row = p.desc.width * p.desc.format.components();
            let mut out = Vec::with_capacity(row * p.desc.height);
            for y in 0..p.desc.height {
                // SAFETY: the plane view covers `height` rows of `stride` bytes.
                let line = unsafe {
                    std::slice::from_raw_parts(p.ptr.add(y * p.stride) as *const u16, row)
                };
                out.extend_from_slice(line);
            }
            out
        })
        .unwrap();
    (samples, label)
}

fn frame_of(path: &std::path::Path, t: f64) -> usize {
    let src = pm_media::decode::service::MediaSource::open(path).unwrap();
    src.select((t * pm_time::TICKS_PER_SECOND as f64).round() as i64)
        .unwrap()
}

/// Transparency of one ProRes file into `format`: (samples, differing).
fn transparency(
    name: &str,
    format: InputFormat,
    luma_max: u16,
    linear_light: bool,
) -> Option<(usize, usize)> {
    let path = media(name)?;
    let (w, h) = (320u32, 180u32);
    let (doc, resolver) = full_frame_video(&path, w, h, 3.0);
    let mut doc = if linear_light { linear(doc) } else { doc };
    doc["colorDepth"] = json!(32);
    let mut r = Renderer::headless().unwrap();
    // The corpus files are tagged BT.601 (`smpte170m`); read them as Rec.709
    // so the source and the Rec.709 output share matrix and primaries (the
    // bar's "Rec.709 source exported to Rec.709").
    let mut c = r.video_color(&path).unwrap();
    c.matrix = MatrixCoefficients::Bt709;
    c.primaries = pm_protocol::color::Primaries::Bt709;
    r.interpret(&path, Some(c));
    let mut request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32);
    request.export = true;
    request.background = false;
    let (mut total, mut bad, mut skipped) = (0, 0, 0);
    for t in [0.01, 1.01, 2.51] {
        let frame = r
            .render(&load(&doc), "Ptest-root", ticks(t), &request, &resolver)
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba32Float);
        let rgba = r.read_rgba(&frame).unwrap();
        // ProRes 4444 carries alpha: straight output, as its encoder input.
        let mut p = params();
        if format == InputFormat::Y416 {
            p.alpha = AlphaOutput::Straight;
        }
        let input = convert(
            &WorkingFrame {
                width: w,
                height: h,
                rgba: &rgba,
            },
            format,
            &p,
        )
        .unwrap();
        let ours = match &input.planes[0].data {
            pm_export::convert::PlaneData::U16(v) => v.clone(),
            _ => unreachable!(),
        };
        let (reference, label) = reference_u16(&path, frame_of(&path, t));
        assert_eq!(reference.len(), ours.len(), "{name}: {label} layout");
        for (i, (a, b)) in ours.iter().zip(&reference).enumerate() {
            // y416: A Y′ Cb Cr; v216: Cb Y′ Cr Y′. Alpha is full range; color
            // is clipped to the codable range (amendment 8). Color under
            // α = 0 is not carried by a premultiplied pipeline (S2).
            let is_alpha = format == InputFormat::Y416 && i % 4 == 0;
            if format == InputFormat::Y416 && !is_alpha && reference[i - i % 4] == 0 {
                skipped += 1;
                continue;
            }
            let expected = if is_alpha {
                *b
            } else {
                (*b).clamp(256, luma_max)
            };
            total += 1;
            if *a != expected {
                if bad < 5 {
                    eprintln!(
                        "{name} t={t} sample {i}: ours {a}, reference {b} (clipped {expected})"
                    );
                }
                bad += 1;
            }
        }
    }
    eprintln!(
        "{name} → {}: {total} samples compared, {bad} differ ({skipped} color samples under alpha 0 not compared)",
        String::from_utf8_lossy(&format.fourcc())
    );
    Some((total, bad))
}

#[test]
fn pipeline_transparency_prores_4444_xq_to_y416() {
    match transparency("prores4444xq-vt.mov", InputFormat::Y416, 65264, false) {
        None => eprintln!("media corpus not found; transparency not measured"),
        Some((_, bad)) => assert_eq!(bad, 0, "ProRes 4444 XQ → y416 is not exact"),
    }
}

#[test]
fn pipeline_transparency_with_linear_blending() {
    for (name, format, max) in [
        ("prores4444xq-vt.mov", InputFormat::Y416, 65264),
        ("prores422hq-vt.mov", InputFormat::V216, 65216),
    ] {
        match transparency(name, format, max, true) {
            None => eprintln!("media corpus not found; transparency not measured"),
            Some((_, bad)) => assert_eq!(bad, 0, "{name} with linear blending is not exact"),
        }
    }
}

#[test]
fn pipeline_transparency_prores_422_hq_to_v216() {
    match transparency("prores422hq-vt.mov", InputFormat::V216, 65216, false) {
        None => eprintln!("media corpus not found; transparency not measured"),
        Some((_, bad)) => assert_eq!(bad, 0, "ProRes 422 HQ → v216 is not exact"),
    }
}

/// The reference decode of frame `n`: every plane's 8-bit samples.
fn reference_u8(path: &std::path::Path, n: usize) -> (Vec<Vec<u8>>, String) {
    use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
    let service = DecodeService::new(ServiceConfig::default());
    let src = MediaSource::open(path).unwrap();
    let mut session = service
        .session(&src, 1, pm_media::vt::DecodePurpose::Export)
        .unwrap();
    let frame = session.frame(n).unwrap();
    let label = frame.format.label.clone();
    let planes = frame
        .with_planes(|planes| {
            planes
                .iter()
                .map(|p| {
                    let row = p.desc.width * p.desc.format.bytes();
                    let mut out = Vec::with_capacity(row * p.desc.height);
                    for y in 0..p.desc.height {
                        // SAFETY: the plane view covers `height` rows of `stride` bytes.
                        out.extend_from_slice(unsafe {
                            std::slice::from_raw_parts(p.ptr.add(y * p.stride), row)
                        });
                    }
                    out
                })
                .collect()
        })
        .unwrap();
    (planes, label)
}

#[test]
fn invariant_1_h264_420_keeps_luma_and_chroma_within_one() {
    invariant_1_420(false);
}

#[test]
fn invariant_1_420_with_linear_blending() {
    invariant_1_420(true);
}

fn invariant_1_420(linear_light: bool) {
    let Some(path) = media("h264-bpyramid.mp4") else {
        eprintln!("media corpus not found; 4:2:0 invariant not measured");
        return;
    };
    let (w, h) = (320u32, 180u32);
    let (doc, resolver) = full_frame_video(&path, w, h, 5.0);
    let mut doc = if linear_light { linear(doc) } else { doc };
    doc["colorDepth"] = json!(32);
    let mut r = Renderer::headless().unwrap();
    // Untagged SD reads as BT.601 with SMPTE 170M primaries (the shared
    // table); read it as Rec.709 to export to Rec.709 with the same matrix
    // and primaries.
    let mut c = r.video_color(&path).unwrap();
    c.matrix = MatrixCoefficients::Bt709;
    c.primaries = pm_protocol::color::Primaries::Bt709;
    r.interpret(&path, Some(c));
    let mut request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32);
    request.export = true;
    let (mut luma_bad, mut chroma_max, mut chroma_off, mut samples) = (0, 0u8, 0, 0);
    for t in [0.02, 1.02, 3.52] {
        let frame = r
            .render(&load(&doc), "Ptest-root", ticks(t), &request, &resolver)
            .unwrap();
        assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba32Float);
        let rgba = r.read_rgba(&frame).unwrap();
        let input = convert(
            &WorkingFrame {
                width: w,
                height: h,
                rgba: &rgba,
            },
            InputFormat::V420,
            &params(),
        )
        .unwrap();
        let (reference, label) = reference_u8(&path, frame_of(&path, t));
        assert_eq!(reference.len(), 2, "{label}: two planes");
        eprintln!(
            "reference {label}: planes {} and {} bytes",
            reference[0].len(),
            reference[1].len()
        );
        let plane = |i: usize| match &input.planes[i].data {
            pm_export::convert::PlaneData::U8(v) => v.clone(),
            _ => unreachable!(),
        };
        let (luma, chroma) = (plane(0), plane(1));
        assert_eq!(luma.len(), reference[0].len());
        assert_eq!(chroma.len(), reference[1].len());
        for (a, b) in luma.iter().zip(&reference[0]) {
            // Y′ codable range 1..254.
            if *a != (*b).clamp(1, 254) {
                luma_bad += 1;
            }
        }
        for (a, b) in chroma.iter().zip(&reference[1]) {
            let d = a.abs_diff((*b).clamp(1, 254));
            chroma_max = chroma_max.max(d);
            if d > 1 {
                chroma_off += 1;
            }
        }
        samples += luma.len() + chroma.len();
    }
    eprintln!(
        "H.264 4:2:0 → 420v: {samples} samples; luma differences {luma_bad}; chroma max {chroma_max} CV, {chroma_off} samples over 1"
    );
    assert_eq!(luma_bad, 0, "luma is not exact");
    assert!(chroma_max <= 1, "chroma differs by {chroma_max} CV");
}
