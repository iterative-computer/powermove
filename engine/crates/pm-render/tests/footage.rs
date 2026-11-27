//! Footage as interpreted, through the renderer (plan sections 9 and 13):
//!
//! - HDR10 and HLG sources tone mapped with the BT.2390 EETF equal the f64
//!   oracle (`pm_color::tonemap`) applied to the same decode;
//! - interlaced sources (4:2:2 8- and 10-bit, 4:2:0, 4:1:1) at field rate
//!   equal FFmpeg's bwdif (`mode=send_field`) run on the same decode and
//!   converted with the same matrix and chroma filter, and at frame rate
//!   show each frame's first field;
//! - HDR, interlaced, non-square-pixel, cropping-clean-aperture and
//!   wide-gamut video draw by default, with the same pictures as an
//!   `interpret` member restating what the file says;
//! - display rotations and the clean aperture are exact permutations and
//!   crops of the unrotated picture, rotated as FFmpeg's autorotate does;
//! - `interpret` overrides in the document change the reading (matrix,
//!   alpha), and a project stamped `requiresEngine` for them is drawn;
//! - a conformed frame rate shows presented frame n from n / rate.
//!
//! Media comes from the format corpus (`engine/crates/pm-media/tests/corpus/
//! formats.sh`, outside the repository); without it each test reports and
//! passes vacuously, like the rest of this crate's corpus tests.

mod support;

use pm_color::chroma::{Factors, Plane, upsample};
use pm_color::tonemap::{HdrSignal, bt2020_to_709, to_sdr};
use pm_color::ycbcr::Matrix;
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::frame::DecodedFrame;
use pm_media::interpret::{Interpretation, resolve_video};
use pm_protocol::color::{ChromaSiting, MatrixCoefficients, Precision};
use pm_protocol::common::Size;
use pm_render::footage::{Geometry, Timing};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use support::doc::*;
use support::ticks;

fn formats(rel: &str) -> Option<PathBuf> {
    let base = std::env::var_os("PM_MEDIA_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .join("Library/Caches/powermove-engine/media-corpus")
        })
        .join("formats");
    let p = base.join(rel);
    if p.exists() {
        Some(p)
    } else {
        eprintln!(
            "{} missing: run engine/crates/pm-media/tests/corpus/formats.sh (skipped)",
            p.display()
        );
        None
    }
}

/// A composition of `w` × `h` at `fps` showing `path` stretched over the
/// whole frame; the asset's display size is `w` × `h`, with `interpret`.
fn document(
    path: &Path,
    (w, h): (u32, u32),
    fps: f64,
    interpret: Option<Value>,
) -> (Value, MapResolver) {
    let dur = 4.0;
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
    doc["colorDepth"] = json!(32);
    doc["fps"] = json!(fps);
    asset(&mut doc, "clip", "video", "mov", w, h, dur);
    if let Some(i) = interpret {
        doc["assets"]["clip"]["interpret"] = i;
        doc["requiresEngine"] = json!("native");
    }
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path.to_path_buf());
    (doc, resolver)
}

fn render(r: &mut Renderer, doc: &Value, size: (u32, u32), t: f64, res: &MapResolver) -> Vec<f32> {
    let request = RenderRequest::at_scale(Size::new(size.0, size.1), 1.0, Precision::Float32);
    let frame = r
        .render(&load(doc), "Ptest-root", ticks(t), &request, res)
        .unwrap_or_else(|e| panic!("render: {e:?}"));
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba32Float);
    r.read_rgba(&frame).unwrap()
}

fn decode(path: &Path, n: usize) -> DecodedFrame {
    let service = DecodeService::new(ServiceConfig::default());
    let src = MediaSource::open(path).unwrap();
    let mut s = service
        .session(&src, 1, pm_media::vt::DecodePurpose::Display)
        .unwrap();
    s.frame(n).unwrap()
}

/// R′G′B′ of normalized Y′CbCr planes: chroma upsampled with section 9's
/// filters (`pm_color::chroma`), then the matrix, in f64.
#[allow(clippy::too_many_arguments)]
fn rgb_of(
    (w, h): (usize, usize),
    luma: &[f32],
    cb: &[f32],
    cr: &[f32],
    (cw, ch): (usize, usize),
    factors: Factors,
    siting: ChromaSiting,
    matrix: MatrixCoefficients,
) -> Vec<[f64; 3]> {
    let up = |c: &[f32]| {
        upsample(
            &Plane {
                width: cw as u32,
                height: ch as u32,
                samples: c,
            },
            w as u32,
            h as u32,
            factors,
            siting,
        )
    };
    let (ub, ur) = (up(cb), up(cr));
    let m = Matrix::of(matrix).unwrap();
    (0..w * h)
        .map(|i| m.ycbcr_to_rgb([f64::from(luma[i]), f64::from(ub[i]), f64::from(ur[i])]))
        .collect()
}

/// A decoded frame's R′G′B′ (CPU reference).
fn frame_rgb(f: &DecodedFrame, siting: ChromaSiting, matrix: MatrixCoefficients) -> Vec<[f64; 3]> {
    let (ya, cc) = f.reference().unwrap();
    let w = f.format.width;
    let h = f.format.height;
    let cw = (w + (1 << f.format.log2_chroma_w) - 1) >> f.format.log2_chroma_w;
    let ch = (h + (1 << f.format.log2_chroma_h) - 1) >> f.format.log2_chroma_h;
    let luma: Vec<f32> = ya.iter().map(|p| p[0]).collect();
    let cb: Vec<f32> = cc.iter().map(|p| p[0]).collect();
    let cr: Vec<f32> = cc.iter().map(|p| p[1]).collect();
    rgb_of(
        (w, h),
        &luma,
        &cb,
        &cr,
        (cw, ch),
        Factors::from_log2(f.format.log2_chroma_w, f.format.log2_chroma_h),
        siting,
        matrix,
    )
}

/// E-encoded R′G′B′ in `primaries` to Rec.709, in linear light, extended:
/// superwhites and sub-blacks are kept.
fn in_709(rgb: Vec<[f64; 3]>, primaries: pm_protocol::color::Primaries) -> Vec<[f64; 3]> {
    use pm_color::transfer::{display_decode, display_encode};
    let m =
        pm_color::primaries::conversion(primaries, pm_protocol::color::Primaries::Bt709).unwrap();
    rgb.into_iter()
        .map(|c| pm_color::primaries::apply(&m, c.map(display_decode)).map(display_encode))
        .collect()
}

fn max_diff(ours: &[f32], want: &[[f64; 3]]) -> f64 {
    want.iter()
        .enumerate()
        .flat_map(|(i, w)| (0..3).map(move |c| (i, c, w[c])))
        .map(|(i, c, w)| (f64::from(ours[i * 4 + c]) - w).abs())
        .fold(0.0, f64::max)
}

/// Interpret Footage › Tone Mapping: an HDR source in an SDR project takes
/// the method chosen for it, each against `pm_color::cst::tone_to_sdr` on
/// the decoded frame; Automatic is BT.2390 per channel, the same bits as
/// choosing it.
#[test]
fn an_hdr_source_takes_the_tone_mapping_chosen_for_it() {
    for (file, transfer) in [("hdr/hdr10.mov", "pq"), ("hdr/hlg.mov", "hlg")] {
        let Some(path) = formats(file) else { return };
        let src = MediaSource::open(&path).unwrap();
        let v = src.probe.video.as_ref().unwrap();
        let reading = resolve_video(v, &Interpretation::default());
        let peak = reading.tone_map.expect("tone mapped").peak_nits;
        let (w, h) = (v.width, v.height);
        let signal = if transfer == "pq" {
            HdrSignal::Pq
        } else {
            HdrSignal::Hlg
        };
        let mut r = Renderer::headless().unwrap();
        let mut draw = |tone: Option<&str>| {
            let mut interpret = json!({ "transfer": transfer });
            if let Some(tone) = tone {
                interpret["toneMapping"] = json!(tone);
            }
            let (doc, res) = document(&path, (w, h), 25.0, Some(interpret));
            render(&mut r, &doc, (w, h), 0.13, &res)
        };
        let automatic = draw(None);
        assert_eq!(
            draw(Some("bt2390")),
            automatic,
            "{file}: Automatic is BT.2390"
        );
        let n = src.select(ticks(0.13).get()).unwrap();
        let f = decode(&path, n);
        let rgb = frame_rgb(&f, reading.siting.value, reading.matrix.value);
        for (index, name) in pm_media::interpret::TONE_MAPPING.iter().enumerate() {
            let ours = draw(Some(name));
            let want: Vec<[f64; 3]> = rgb
                .iter()
                .map(|code| {
                    let nits =
                        pm_color::tonemap::to_working_light(signal, code.map(|v| v.clamp(0., 1.)))
                            .map(|l| l * pm_color::tonemap::SDR_WHITE);
                    let sdr = pm_color::cst::tone_to_sdr(nits, index, peak, 15.);
                    pm_color::primaries::apply(&bt2020_to_709(), sdr)
                        .map(pm_color::transfer::display_encode)
                })
                .collect();
            // Relative above 1: None keeps light far above SDR white.
            let d = want
                .iter()
                .enumerate()
                .flat_map(|(i, w)| {
                    let ours = &ours;
                    (0..3).map(move |c| {
                        (f64::from(ours[i * 4 + c]) - w[c]).abs() / w[c].abs().max(1.)
                    })
                })
                .fold(0f64, f64::max);
            eprintln!(
                "{file} {name}: max |Δ| {d:.2e} (relative above 1) against the f64 reference"
            );
            // A quarter of an 8-bit code, as the BT.2390 test; None compresses
            // nothing, so the PQ curve's steep top magnifies f32 differences of
            // the decoded codes about tenfold: twice that.
            let limit = if index == 0 { 2e-3 } else { 1e-3 };
            assert!(d < limit, "{file} {name}: {d}");
        }
    }
}

/// PQ and HLG clips in an HDR project keep their BT.2100 light, each pixel
/// against the f64 oracle on the decoded frame (1.0 = 203 cd/m²; HLG shown
/// at its 1000 cd/m² nominal peak): relative above 1, as the tone mapping
/// test.
#[test]
fn hdr_sources_keep_their_light_in_an_hdr_project() {
    for (file, signal) in [
        ("hdr/hdr10.mov", HdrSignal::Pq),
        ("hdr/hlg.mov", HdrSignal::Hlg),
    ] {
        let Some(path) = formats(file) else { return };
        let src = MediaSource::open(&path).unwrap();
        let v = src.probe.video.as_ref().unwrap();
        let reading = resolve_video(v, &Interpretation::default());
        let (w, h) = (v.width, v.height);
        let mut r = Renderer::headless().unwrap();
        let transfer = if signal == HdrSignal::Pq { "pq" } else { "hlg" };
        let (mut doc, res) = document(&path, (w, h), 25.0, Some(json!({ "transfer": transfer })));
        doc["color"] = json!({ "mode": "hdr" });
        let ours = render(&mut r, &doc, (w, h), 0.13, &res);
        let n = src.select(ticks(0.13).get()).unwrap();
        let f = decode(&path, n);
        let want: Vec<[f64; 3]> = frame_rgb(&f, reading.siting.value, reading.matrix.value)
            .into_iter()
            .map(|code| {
                let light =
                    pm_color::tonemap::to_working_light(signal, code.map(|v| v.clamp(0., 1.)));
                pm_color::primaries::apply(&bt2020_to_709(), light)
                    .map(pm_color::transfer::display_encode)
            })
            .collect();
        let d = want
            .iter()
            .enumerate()
            .flat_map(|(i, w)| {
                let ours = &ours;
                (0..3).map(move |c| (f64::from(ours[i * 4 + c]) - w[c]).abs() / w[c].abs().max(1.))
            })
            .fold(0f64, f64::max);
        eprintln!(
            "{file} in an HDR project: max |Δ| {d:.2e} (relative above 1) against the f64 oracle"
        );
        // As the tone mapping test's None: light kept up to the PQ curve's
        // steep top magnifies f32 differences of the decoded codes about
        // tenfold, so twice the BT.2390 bound.
        assert!(d < 2e-3, "{file}: {d}");
    }
}

/// The project's Output sets the clip defaults: an HDR clip keeps its light
/// in either HDR output and is tone mapped only when the output is SDR; the
/// HDR transfer (PQ or HLG) is a delivery choice and leaves the picture as is.
#[test]
fn an_hdr_clip_is_tone_mapped_only_when_the_output_is_sdr() {
    let Some(path) = formats("hdr/hlg.mov") else {
        return;
    };
    let src = MediaSource::open(&path).unwrap();
    let v = src.probe.video.as_ref().unwrap();
    let (w, h) = (v.width, v.height);
    let mut r = Renderer::headless().unwrap();
    let mut draw = |color: Value| {
        let (mut doc, res) = document(&path, (w, h), 25.0, Some(json!({ "transfer": "hlg" })));
        doc["color"] = color;
        render(&mut r, &doc, (w, h), 0.13, &res)
    };
    let pq = draw(json!({ "mode": "hdr" }));
    let hlg = draw(json!({ "mode": "hdr", "hdrTransfer": "hlg" }));
    let sdr = draw(json!({ "mode": "standard", "hdrTransfer": "hlg" }));
    assert_eq!(pq, hlg, "the HDR transfer leaves the picture as it is");
    // The brightest light, as Rec. 709 luminance relative to SDR white
    // (a saturated BT.2020 colour can leave one Rec. 709 channel above 1).
    let top = |px: &[f32]| {
        px.chunks(4)
            .map(|p| {
                let l = |c: f32| pm_color::transfer::display_decode(f64::from(c));
                0.2126 * l(p[0]) + 0.7152 * l(p[1]) + 0.0722 * l(p[2])
            })
            .fold(0f64, f64::max)
    };
    assert!(
        top(&hlg) > 1.5,
        "HDR keeps light above SDR white: {}",
        top(&hlg)
    );
    assert!(
        top(&sdr) <= 1.0 + 1e-3,
        "SDR tone maps to SDR white: {}",
        top(&sdr)
    );
    // An SDR clip sits at SDR white (1.0 = 203 cd/m²) whatever the output.
    let Some(path) = formats("video/color-p3.mov") else {
        return;
    };
    let src = MediaSource::open(&path).unwrap();
    let v = src.probe.video.as_ref().unwrap();
    let (w, h) = (v.width, v.height);
    let mut draw = |color: Value| {
        let (mut doc, res) = document(&path, (w, h), 25.0, None);
        doc["color"] = color;
        render(&mut r, &doc, (w, h), 0.13, &res)
    };
    let sdr = draw(json!({ "mode": "standard" }));
    for color in [
        json!({ "mode": "hdr" }),
        json!({ "mode": "hdr", "hdrTransfer": "hlg" }),
    ] {
        assert_eq!(draw(color.clone()), sdr, "{color}");
    }
}

#[test]
fn hdr_sources_are_tone_mapped_with_bt2390() {
    for (file, signal) in [
        ("hdr/hdr10.mov", HdrSignal::Pq),
        ("hdr/hlg.mov", HdrSignal::Hlg),
    ] {
        let Some(path) = formats(file) else { return };
        let src = MediaSource::open(&path).unwrap();
        let v = src.probe.video.as_ref().unwrap();
        let reading = resolve_video(v, &Interpretation::default());
        let tm = reading.tone_map.expect("tone mapped");
        let (w, h) = (v.width, v.height);
        // Without overrides HDR routes to the legacy renderer (today's
        // Chromium handling is not measured yet); interpreted, it is drawn.
        let mut r = Renderer::headless().unwrap();
        let (doc, res) = document(
            &path,
            (w, h),
            25.0,
            Some(json!({ "transfer": if signal == HdrSignal::Pq { "pq" } else { "hlg" } })),
        );
        let ours = render(&mut r, &doc, (w, h), 0.13, &res);
        // Overrides that restate the file's own reading change nothing.
        let (doc_i, _) = document(
            &path,
            (w, h),
            25.0,
            Some(json!({ "primaries": "bt2020", "matrix": "bt2020Ncl" })),
        );
        assert_eq!(render(&mut r, &doc_i, (w, h), 0.13, &res), ours);
        let n = src.select(ticks(0.13).get()).unwrap();
        let f = decode(&path, n);
        let want: Vec<[f64; 3]> = frame_rgb(&f, reading.siting.value, reading.matrix.value)
            .into_iter()
            .map(|rgb| {
                let sdr = to_sdr(signal, rgb.map(|v| v.clamp(0.0, 1.0)), tm.peak_nits);
                pm_color::primaries::apply(&bt2020_to_709(), sdr)
                    .map(pm_color::transfer::display_encode)
            })
            .collect();
        let d = max_diff(&ours, &want);
        if std::env::var_os("PM_TRACE").is_some() {
            let mut worst = (0.0, 0);
            for (i, w) in want.iter().enumerate() {
                for c in 0..3 {
                    let e = (f64::from(ours[i * 4 + c]) - w[c]).abs();
                    if e > worst.0 {
                        worst = (e, i);
                    }
                }
            }
            let i = worst.1;
            eprintln!(
                "worst at {i}: ours {:?} want {:?}",
                &ours[i * 4..i * 4 + 3],
                want[i]
            );
            let mut hist = [0usize; 6];
            for (i, w) in want.iter().enumerate() {
                for c in 0..3 {
                    let e = (f64::from(ours[i * 4 + c]) - w[c]).abs();
                    hist[((e.log10() + 8.0).clamp(0.0, 5.0)) as usize] += 1;
                }
            }
            eprintln!("error histogram by decade from 1e-8: {hist:?}");
        }
        eprintln!("{file}: tone mapped frame {n}, max |Δ| {d:.2e} against the f64 oracle");
        // f32 on the GPU against f64, a quarter of an 8-bit code value at
        // most: the PQ curve's powers and the matrix's cancellation.
        assert!(d < 1e-3, "{file}: {d}");
    }
}

#[test]
fn interlaced_sources_deinterlace_like_ffmpeg_bwdif() {
    // 4:2:2 MXF (bottom field first), 4:2:2 10-bit QuickTime (top field
    // first), DV PAL 4:2:0 and DVCPRO 4:1:1 (bottom field first): bwdif runs
    // on each plane before chroma upsampling, so interlaced 4:2:0 and 4:1:1
    // chroma is deinterlaced at its own resolution, as FFmpeg's is.
    for (file, order) in [
        ("video/interlaced-bff.mxf", "bottomFirst"),
        ("video/interlaced-tff.mov", "topFirst"),
        ("video/dv-pal.dv", "bottomFirst"),
        ("video/dvcpro25-pal.mov", "bottomFirst"),
    ] {
        let Some(path) = formats(file) else {
            return;
        };
        deinterlaces_like_bwdif(&path, order);
    }
}

fn deinterlaces_like_bwdif(path: &Path, order: &str) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let src = MediaSource::open(path).unwrap();
    let v = src.probe.video.as_ref().unwrap();
    let probed = match v.fields.order {
        pm_media::probe::FieldOrder::TopFirst => "topFirst",
        pm_media::probe::FieldOrder::BottomFirst => "bottomFirst",
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(probed, order, "{name}: field order");
    // The file's field order deinterlaces with no override (today's viewer
    // shows the woven frame; native has one path).
    assert_eq!(pm_render::media::probe_video(path), Ok(()), "{name}");
    let interp = json!({ "fieldOrder": order });
    let reading = resolve_video(v, &Interpretation::from_json(&interp).0);
    let (w, h) = (v.width as usize, v.height as usize);
    let f0 = decode(path, 0);
    let (lw, lh) = (f0.format.log2_chroma_w, f0.format.log2_chroma_h);
    let (cw, ch) = (w >> lw, h >> lh);
    let deep = f0.format.bits > 8;
    let pix_fmt = match (lw, lh, deep) {
        (1, 0, false) => "yuv422p",
        (1, 0, true) if f0.format.bits == 10 => "yuv422p10le",
        (1, 1, false) => "yuv420p",
        (2, 0, false) => "yuv411p",
        other => panic!("{name}: chroma {other:?}"),
    };
    // Our decode, as planar code values: FFmpeg's bwdif runs on the same
    // pictures (VideoToolbox's ProRes decode and FFmpeg's are not
    // bit-identical, and this test is about deinterlacing).
    let (y_max, y_off, c_off) = if deep {
        (876.0, 64.0, 512.0)
    } else {
        (219.0, 16.0, 128.0)
    };
    let c_max = y_max * 224.0 / 219.0;
    let mut decoded = Vec::new();
    {
        let service = DecodeService::new(ServiceConfig::default());
        let mut session = service
            .session(&src, 1, pm_media::vt::DecodePurpose::Display)
            .unwrap();
        let push = |out: &mut Vec<u8>, v: f64| {
            let code = v.round() as u16;
            if deep {
                out.extend(code.to_le_bytes());
            } else {
                out.push(code as u8);
            }
        };
        for n in 0..src.pts.len() {
            let (ya, cc) = session.frame(n).unwrap().reference().unwrap();
            for p in &ya {
                push(&mut decoded, f64::from(p[0]) * y_max + y_off);
            }
            for k in 0..2 {
                for p in &cc {
                    push(&mut decoded, f64::from(p[k]) * c_max + c_off);
                }
            }
        }
    }
    // FFmpeg's bwdif, one picture per field, with the field order we read.
    let tmp = std::env::temp_dir();
    let id = format!("{}-{name}", std::process::id());
    let input = tmp.join(format!("pm-render-bwdif-in-{id}.raw"));
    let raw = tmp.join(format!("pm-render-bwdif-{id}.raw"));
    std::fs::write(&input, &decoded).unwrap();
    let ffmpeg = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Caches/powermove-engine/ffmpeg/out/bin/ffmpeg");
    let parity = if order == "topFirst" { "tff" } else { "bff" };
    let ok = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", pix_fmt])
        .args(["-s", &format!("{w}x{h}"), "-r", "25", "-i"])
        .arg(&input)
        .args([
            "-vf",
            &format!("bwdif=mode=send_field:parity={parity}:deint=all"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            pix_fmt,
        ])
        .arg(&raw)
        .status()
        .unwrap()
        .success();
    let _ = std::fs::remove_file(&input);
    assert!(ok, "{name}: ffmpeg");
    let reference = std::fs::read(&raw).unwrap();
    let _ = std::fs::remove_file(&raw);
    // Limited-range code values at the source depth, as 8-bit codes
    // (10-bit codes / 4), so one normalization serves both depths.
    let bps = if deep { 2usize } else { 1 };
    let norm = |b: &[u8], i: usize| {
        if deep {
            f32::from(u16::from_le_bytes([b[2 * i], b[2 * i + 1]])) / 4.0
        } else {
            f32::from(b[i])
        }
    };
    let frame_bytes = (w * h + 2 * cw * ch) * bps;
    let fields = reference.len() / frame_bytes;
    assert!(fields >= 8, "{name}: {fields} fields from bwdif");
    let mut r = Renderer::headless().unwrap();
    let (doc50, res) = document(path, (w as u32, h as u32), 50.0, Some(interp.clone()));
    let (doc25, _) = document(path, (w as u32, h as u32), 25.0, Some(interp));
    let mut worst = 0f64;
    let picks = [0usize, 1, 2, 3, 20, fields - 3, fields - 2, fields - 1];
    for field in picks {
        let t = (field as f64 + 0.25) / 50.0;
        let ours = render(&mut r, &doc50, (w as u32, h as u32), t, &res);
        let p = &reference[field * frame_bytes..][..frame_bytes];
        let plane = |offset: usize, n: usize, f: &dyn Fn(f32) -> f32| -> Vec<f32> {
            (0..n).map(|i| f(norm(&p[offset * bps..], i))).collect()
        };
        let y = plane(0, w * h, &|v| (v - 16.0) / 219.0);
        let cb = plane(w * h, cw * ch, &|v| (v - 128.0) / 224.0);
        let cr = plane(w * h + cw * ch, cw * ch, &|v| (v - 128.0) / 224.0);
        let want = rgb_of(
            (w, h),
            &y,
            &cb,
            &cr,
            (cw, ch),
            Factors::from_log2(lw, lh),
            reading.siting.value,
            reading.matrix.value,
        );
        let want = match reading.primaries.value {
            pm_protocol::color::Primaries::Bt709 => want,
            p => in_709(want, p),
        };
        let d = max_diff(&ours, &want);
        worst = worst.max(d);
        assert!(d < 1e-5, "{name} field {field}: max |Δ| {d}");
        // At 25 fps the frame shows its first field.
        if field % 2 == 0 {
            let frame = render(&mut r, &doc25, (w as u32, h as u32), t, &res);
            assert_eq!(
                frame, ours,
                "{name}: frame-rate output keeps the first field"
            );
        }
    }
    eprintln!(
        "{name} ({pix_fmt}, {order}) at 50 fps: max |Δ| {worst:.2e} against FFmpeg's bwdif over {} fields",
        picks.len()
    );
}

#[test]
fn rotations_and_clean_aperture_are_exact() {
    let Some(base) = formats("video/geom-base.mov") else {
        return;
    };
    let (w, h) = (320usize, 180usize);
    let mut r = Renderer::headless().unwrap();
    let (doc, res) = document(&base, (w as u32, h as u32), 25.0, None);
    let t = 0.21;
    let plain = render(&mut r, &doc, (w as u32, h as u32), t, &res);
    // FFmpeg's autorotate defines the direction: the CLI's rotated luma
    // equals our mapping of the unrotated luma.
    let ffmpeg = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Caches/powermove-engine/ffmpeg/out/bin/ffmpeg");
    let gray = |path: &Path, autorotate: bool| {
        let mut c = std::process::Command::new(&ffmpeg);
        c.args(["-v", "error"]);
        if !autorotate {
            c.arg("-noautorotate");
        }
        let out = c
            .arg("-i")
            .arg(path)
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "gray", "-"])
            .output()
            .unwrap();
        assert!(out.status.success());
        out.stdout
    };
    let base_gray = gray(&base, false);
    for turns in [1u32, 2, 3] {
        let file = format!("video/geom-rot{}.mov", turns * 90);
        let path = formats(&file).unwrap();
        let src = MediaSource::open(&path).unwrap();
        let reading = resolve_video(
            src.probe.video.as_ref().unwrap(),
            &Interpretation::default(),
        );
        let g = Geometry::of(&reading, (w as u32, h as u32));
        assert_eq!(g.quarter_turns, turns, "{file}");
        let (ow, oh) = g.output_size((w as u32, h as u32));
        let ff = gray(&path, true);
        let mapped: Vec<u8> = (0..oh)
            .flat_map(|y| (0..ow).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (u, v) = g.source_of(x, y, (w as u32, h as u32));
                base_gray[v as usize * w + u as usize]
            })
            .collect();
        assert_eq!(mapped, ff, "{file}: FFmpeg rotates the other way");
        // Quarter turns use the same exact geometry with the file defaults.
        assert_eq!(pm_render::media::probe_video(&path), Ok(()), "{file}");
        // The renderer's picture is the same permutation of the unrotated
        // picture, exactly.
        for interpret in [None, Some(json!({ "pixelAspect": [1, 1] }))] {
            let (doc_r, res_r) = document(&path, (ow, oh), 25.0, interpret);
            let ours = render(&mut r, &doc_r, (ow, oh), t, &res_r);
            for y in 0..oh {
                for x in 0..ow {
                    let (u, v) = g.source_of(x, y, (w as u32, h as u32));
                    let o = (y * ow + x) as usize * 4;
                    let p = (v as usize * w + u as usize) * 4;
                    assert_eq!(ours[o..o + 4], plain[p..p + 4], "{file} ({x}, {y})");
                }
            }
        }
    }
    // Rotations off the quarter turns still require unsupported geometry.
    let oblique = formats("video/geom-rot45.mov").expect("generate the 45-degree witness");
    assert!(
        pm_render::media::probe_video(&oblique)
            .is_err_and(|reason| reason.contains("display rotation"))
    );
    // The clean aperture: the full frame by default, the crop on request.
    let clap = formats("video/geom-clap.mov").unwrap();
    assert_eq!(pm_render::media::probe_video(&clap), Ok(()));
    let (doc_c, res_c) = document(&clap, (w as u32, h as u32), 25.0, None);
    assert_eq!(
        render(&mut r, &doc_c, (w as u32, h as u32), t, &res_c),
        plain
    );
    let (doc_c, res_c) = document(
        &clap,
        (w as u32, h as u32),
        25.0,
        Some(json!({ "cleanAperture": false })),
    );
    assert_eq!(
        render(&mut r, &doc_c, (w as u32, h as u32), t, &res_c),
        plain
    );
    let (doc_c, res_c) = document(
        &clap,
        (300, 160),
        25.0,
        Some(json!({ "cleanAperture": true })),
    );
    let crop = render(&mut r, &doc_c, (300, 160), t, &res_c);
    for y in 0..160usize {
        let o = y * 300 * 4;
        let p = ((y + 12) * w + 6) * 4;
        assert_eq!(crop[o..o + 300 * 4], plain[p..p + 300 * 4], "row {y}");
    }
}

#[test]
fn interpret_overrides_change_the_reading() {
    let Some(path) = formats("video/color-601-625.mov") else {
        return;
    };
    let (w, h) = (720u32, 576u32);
    let mut r = Renderer::headless().unwrap();
    let src = MediaSource::open(&path).unwrap();
    let n = src.select(ticks(0.1).get()).unwrap();
    let f = decode(&path, n);
    // As tagged: BT.601. Overridden: BT.709 — the document's `interpret`.
    // Every reading uses section 9's siting and filters in unclipped
    // floats. Restating the file's own primaries leaves the tagged matrix
    // to the file, and an uninterpreted level 1 layer reads it the same way.
    for (interp, matrix) in [
        (
            json!({ "primaries": "bt601_625" }),
            MatrixCoefficients::Bt601,
        ),
        (json!({ "matrix": "bt709" }), MatrixCoefficients::Bt709),
    ] {
        let (doc, res) = document(&path, (w, h), 25.0, Some(interp.clone()));
        let ours = render(&mut r, &doc, (w, h), 0.1, &res);
        let want = in_709(
            frame_rgb(&f, ChromaSiting::Left, matrix),
            pm_protocol::color::Primaries::Bt601_625,
        );
        let d = max_diff(&ours, &want);
        eprintln!("color-601-625.mov read with {matrix:?}: max |Δ| {d:.2e}");
        assert!(d < 1e-5, "{interp}: {d}");
    }
    let (doc, res) = document(&path, (w, h), 25.0, None);
    let level1 = render(&mut r, &doc, (w, h), 0.1, &res);
    let tagged = in_709(
        frame_rgb(&f, ChromaSiting::Left, MatrixCoefficients::Bt601),
        pm_protocol::color::Primaries::Bt601_625,
    );
    let d = max_diff(&level1, &tagged);
    eprintln!("color-601-625.mov at level 1: max |Δ| {d:.2e}");
    assert!(d < 1e-5, "level 1 reading: {d}");
    // Members that only choose or place frames leave the colours as they
    // are: restating the file's rate, square pixels, full aperture,
    // progressive scan, or an alpha choice on an opaque file.
    for interp in [
        json!({ "frameRate": 25 }),
        json!({ "pixelAspect": [1, 1] }),
        json!({ "cleanAperture": false }),
        json!({ "fieldOrder": "progressive" }),
        json!({ "alpha": "ignore" }),
        json!({ "frameRate": 25, "pixelAspect": [1, 1], "cleanAperture": false }),
    ] {
        let (doc, res) = document(&path, (w, h), 25.0, Some(interp.clone()));
        let ours = render(&mut r, &doc, (w, h), 0.1, &res);
        assert!(ours == level1, "{interp}: changed the level 1 colours");
    }
    // Ignoring alpha makes an alpha source opaque.
    let alpha = formats("video/prores-4444-alpha.mov").unwrap();
    let (doc, res) = document(&alpha, (320, 180), 25.0, Some(json!({ "alpha": "ignore" })));
    let ours = render(&mut r, &doc, (320, 180), 0.1, &res);
    assert!(ours.chunks_exact(4).all(|p| p[3] == 1.0));
    let (doc, res) = document(&alpha, (320, 180), 25.0, None);
    let straight = render(&mut r, &doc, (320, 180), 0.1, &res);
    assert!(straight.chunks_exact(4).any(|p| p[3] < 0.5));
}

/// A clip whose range flag is wrong, as full-range cameras and screen
/// recorders write them: 8-bit Y′CbCr holding codes over the whole 0..255,
/// in a file that says limited. Four grey bands; `luma(frame)` gives their
/// codes. Stored lossless (FFV1), so the file holds exactly these codes.
fn mistagged_clip(dir: &Path, frames: usize, luma: impl Fn(usize) -> [u8; 4]) -> PathBuf {
    banded_clip(dir, "mistagged.mkv", "tv", frames, luma)
}

/// Four bands of luma codes on neutral chroma, 64×48 FFV1 4:2:0 whose range
/// flag says `range` ("tv" or "pc").
fn banded_clip(
    dir: &Path,
    name: &str,
    range: &str,
    frames: usize,
    luma: impl Fn(usize) -> [u8; 4],
) -> PathBuf {
    let (w, h) = (64usize, 48usize);
    let mut raw = Vec::new();
    for n in 0..frames {
        let bands = luma(n);
        for _ in 0..h {
            for x in 0..w {
                raw.push(bands[x / 16]);
            }
        }
        raw.extend(std::iter::repeat_n(128u8, w * h / 2));
    }
    let input = dir.join("codes.yuv");
    std::fs::write(&input, raw).unwrap();
    let output = dir.join(name);
    let ffmpeg = PathBuf::from(
        std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh before this test"),
    )
    .join("bin/ffmpeg");
    let status = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "yuv420p"])
        .args(["-s", "64x48", "-r", "25", "-i"])
        .arg(&input)
        .args(["-c:v", "ffv1", "-color_range", range])
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg wrote the clip");
    output
}

/// Interpret Footage › Range reaches the numbers: the codes are read as the
/// chosen range, on the ordinary path and through deinterlacing, and the
/// file's own flag is used when nothing is chosen.
#[test]
fn a_range_override_reads_the_codes_the_file_holds() {
    let dir = tempfile::tempdir().unwrap();
    let luma = |n: usize| [16 + n as u8, 128 - 4 * n as u8, 235, 255 - n as u8];
    let moving = mistagged_clip(dir.path(), 8, luma);
    let (w, h) = (64u32, 48u32);
    let mut r = Renderer::headless().unwrap();
    let frame = |r: &mut Renderer, path: &Path, interpret: Option<Value>, t: f64| {
        let (doc, res) = document(path, (w, h), 25.0, interpret);
        let pixels = render(r, &doc, (w, h), t, &res);
        let at = |band: usize| {
            let o = ((h as usize / 2) * w as usize + band * 16 + 8) * 4;
            [pixels[o], pixels[o + 1], pixels[o + 2]].map(f64::from)
        };
        [at(0), at(1), at(2), at(3)]
    };
    let check = |what: &str, got: [[f64; 3]; 4], n: usize, read: &dyn Fn(f64) -> f64| {
        for (band, code) in luma(n).into_iter().enumerate() {
            let expected = read(f64::from(code));
            for value in got[band] {
                assert!(
                    (value - expected).abs() < 2e-5,
                    "{what}, frame {n}, code {code}: {value}, expected {expected}"
                );
            }
        }
    };
    let limited = |code: f64| (code - 16.) / 219.;
    let full = |code: f64| code / 255.;
    for n in [0, 3] {
        let t = n as f64 / 25. + 0.01;
        check(
            "as the file says",
            frame(&mut r, &moving, None, t),
            n,
            &limited,
        );
        let chosen = frame(&mut r, &moving, Some(json!({ "range": "limited" })), t);
        check("limited chosen", chosen, n, &limited);
        let chosen = frame(&mut r, &moving, Some(json!({ "range": "full" })), t);
        check("full chosen", chosen, n, &full);
        // Back to Automatic after the override: the earlier frame is not kept.
        check(
            "automatic again",
            frame(&mut r, &moving, None, t),
            n,
            &limited,
        );
    }
    // Through deinterlacing (a held picture, so every field shows it).
    let held = mistagged_clip(dir.path(), 8, |_| luma(2));
    for (interpret, read) in [
        (
            json!({ "fieldOrder": "topFirst" }),
            &limited as &dyn Fn(f64) -> f64,
        ),
        (json!({ "fieldOrder": "topFirst", "range": "full" }), &full),
        (
            json!({ "fieldOrder": "bottomFirst", "range": "full" }),
            &full,
        ),
    ] {
        let got = frame(&mut r, &held, Some(interpret.clone()), 0.13);
        check(&interpret.to_string(), got, 2, read);
    }
}

/// A proxy really made from `original` by the app's generator (ProRes Proxy
/// at full size), or the generator's refusal.
fn proxy_of(dir: &Path, original: &Path) -> PathBuf {
    use pm_protocol::priority::{ProxyFormat, ProxySize};
    let codecs = PathBuf::from(
        std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh before this test"),
    )
    .join("bin");
    let store = pm_proxy::ProxyStore::open(dir.join("store")).unwrap();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let hash = blake3::hash(&std::fs::read(original).unwrap())
        .to_hex()
        .to_string();
    pm_proxy::generate_with_disk_probe(
        pm_proxy::GenerationRequest {
            source: original,
            source_hash: &hash,
            format: ProxyFormat::ProResProxy,
            size: ProxySize::Full,
            ffmpeg: &codecs.join("ffmpeg"),
            ffprobe: &codecs.join("ffprobe"),
            store: &store,
            control: pm_proxy::GenerationControl {
                cancel: &cancel,
                pause: &|| false,
                on_pause: &|_| {},
                on_progress: &|_| {},
            },
        },
        &|_: &Path| -> pm_proxy::Result<(u64, u64)> { Ok((40 << 30, 100 << 30)) },
    )
    .unwrap_or_else(|e| panic!("the generator made no proxy of {}: {e}", original.display()))
    .stored
    .path
}

/// Interpret Footage describes the original file. A proxy of a Y′CbCr
/// original holds the original's codes, so a corrected range reads the proxy
/// as it reads the original, before and after the correction. A proxy of an
/// R′G′B′ original was converted to Y′CbCr by the generator, with the matrix
/// and range it tagged: the asset's matrix or range, which do nothing on the
/// original, do not misread the proxy.
#[test]
fn an_interpretation_reads_a_generated_proxy_as_it_reads_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let (w, h) = (64u32, 48u32);
    let mut r = Renderer::headless().unwrap();
    // The centre of each of the four bands.
    let bands =
        |r: &mut Renderer, original: &Path, proxy: Option<&Path>, interpret: Option<Value>| {
            let (doc, mut res) = document(proxy.unwrap_or(original), (w, h), 25.0, interpret);
            if proxy.is_some() {
                res.originals.insert("clip".into(), original.to_path_buf());
            }
            let pixels = render(r, &doc, (w, h), 0.01, &res);
            let at = |band: usize| {
                let o = ((h as usize / 2) * w as usize + band * 16 + 8) * 4;
                [pixels[o], pixels[o + 1], pixels[o + 2]].map(f64::from)
            };
            [at(0), at(1), at(2), at(3)]
        };
    let apart = |a: [[f64; 3]; 4], b: [[f64; 3]; 4]| {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .map(|(a, b)| (a - b).abs())
            .fold(0., f64::max)
    };
    // ProRes Proxy on flat bands, 10-bit codes clipped to 4..1019.
    const CODEC: f64 = 0.012;

    // Y′CbCr holding full-range codes in a file that says limited.
    let luma = [24u8, 120, 235, 250];
    let camera = mistagged_clip(dir.path(), 8, |_| luma);
    let camera_proxy = proxy_of(dir.path(), &camera);
    for interpret in [None, Some(json!({ "range": "full" })), None] {
        let original = bands(&mut r, &camera, None, interpret.clone());
        let proxy = bands(&mut r, &camera, Some(&camera_proxy), interpret.clone());
        let d = apart(original, proxy);
        eprintln!("Y′CbCr original, {interpret:?}: proxy {d:.4} from the original");
        assert!(
            d < CODEC,
            "{interpret:?}: proxy {proxy:?}, original {original:?}"
        );
        let read = |code: u8| match interpret {
            Some(_) => f64::from(code) / 255.,
            None => (f64::from(code) - 16.) / 219.,
        };
        for (band, code) in luma.into_iter().enumerate() {
            assert!(
                (original[band][1] - read(code)).abs() < 2e-5,
                "{interpret:?}: code {code} read as {}",
                original[band][1]
            );
        }
    }

    // Y′CbCr flagged full range: the generator rescales the codes to the
    // limited range ProRes holds. Without a correction the proxy reads as
    // the original; a corrected range can't be carried by rescaled codes,
    // so the clip is drawn from its original, exactly.
    let full = banded_clip(dir.path(), "full.mkv", "pc", 8, |_| luma);
    let full_proxy = proxy_of(dir.path(), &full);
    for (interpret, tolerance) in [
        (None, CODEC),
        (Some(json!({ "range": "limited" })), 1e-6),
        (Some(json!({ "matrix": "bt601" })), CODEC),
        (None, CODEC),
    ] {
        let original = bands(&mut r, &full, None, interpret.clone());
        let proxy = bands(&mut r, &full, Some(&full_proxy), interpret.clone());
        let d = apart(original, proxy);
        eprintln!("full-range Y′CbCr original, {interpret:?}: proxy {d:.4} from the original");
        assert!(
            d < tolerance,
            "{interpret:?}: proxy {proxy:?}, original {original:?}"
        );
    }

    // R′G′B′ tagged Rec.2020: saturated bands, where a wrong matrix shows.
    let colours = [
        [200u8, 40, 40],
        [40, 200, 40],
        [40, 40, 200],
        [128, 128, 128],
    ];
    let mut raw = Vec::new();
    for _ in 0..8 * h as usize {
        for x in 0..w as usize {
            raw.extend(colours[x / 16]);
        }
    }
    let input = dir.path().join("rgb.raw");
    std::fs::write(&input, raw).unwrap();
    // QuickTime Animation, tagged as such files are (a Y′CbCr matrix in the
    // tag, R′G′B′ in the samples).
    let graphics = dir.path().join("graphics.mov");
    let ffmpeg = PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    let status = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-s", "64x48", "-r", "25", "-i"])
        .arg(&input)
        .args([
            "-vf",
            "setparams=color_primaries=bt2020:color_trc=bt709:colorspace=bt2020nc",
        ])
        .args([
            "-c:v",
            "qtrle",
            "-pix_fmt",
            "rgb24",
            "-movflags",
            "+write_colr",
        ])
        .arg(&graphics)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg wrote the clip");
    let graphics_proxy = proxy_of(dir.path(), &graphics);
    let plain = bands(&mut r, &graphics, None, None);
    for interpret in [
        None,
        Some(json!({ "matrix": "identity" })),
        Some(json!({ "matrix": "bt601", "range": "full" })),
        None,
    ] {
        let original = bands(&mut r, &graphics, None, interpret.clone());
        assert!(
            apart(original, plain) < 1e-6,
            "{interpret:?}: an R′G′B′ file has no matrix or range to reread"
        );
        let proxy = bands(&mut r, &graphics, Some(&graphics_proxy), interpret.clone());
        let d = apart(original, proxy);
        eprintln!("R′G′B′ original, {interpret:?}: proxy {d:.4} from the original");
        assert!(
            d < CODEC,
            "{interpret:?}: proxy {proxy:?}, original {original:?}"
        );
    }
    // The other members still reach the proxy: a transfer chosen for the
    // asset rereads original and proxy alike.
    let log = Some(json!({ "transfer": "gamma22", "matrix": "identity" }));
    let original = bands(&mut r, &graphics, None, log.clone());
    assert!(
        apart(original, plain) > 0.01,
        "the transfer changes the reading"
    );
    let proxy = bands(&mut r, &graphics, Some(&graphics_proxy), log);
    assert!(apart(original, proxy) < CODEC, "{proxy:?} {original:?}");
}

/// An untagged 1280×720 FFV1 clip of four saturated bands (R′G′B′ codes
/// converted by FFmpeg to 4:2:0 Y′CbCr without any colour tag): HD, so the
/// native reading takes Rec.709 and a v1 project's Electron reading Rec.601.
fn untagged_hd_clip(dir: &Path) -> PathBuf {
    let (w, h) = (1280usize, 720usize);
    let colours = [
        [200u8, 40, 40],
        [40, 200, 40],
        [40, 40, 200],
        [180, 160, 60],
    ];
    let mut row = Vec::with_capacity(w * 3);
    for x in 0..w {
        row.extend(colours[x * 4 / w]);
    }
    let input = dir.join("hd.rgb");
    std::fs::write(&input, row.repeat(h * 4)).unwrap();
    let output = dir.join("untagged-hd.mkv");
    let ffmpeg = PathBuf::from(
        std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh before this test"),
    )
    .join("bin/ffmpeg");
    let status = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-s", "1280x720", "-r", "25", "-i"])
        .arg(&input)
        .args(["-c:v", "ffv1", "-pix_fmt", "yuv420p"])
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg wrote the clip");
    output
}

/// A v1 project's asset carries Electron's video reading (`videoColor:
/// "electron"`): its generated proxy, tagged with the native reading, is read
/// as the original is read, with and without an Interpret Footage choice.
#[test]
fn a_proxy_of_an_asset_read_as_in_powermove_1_reads_as_its_original() {
    let dir = tempfile::tempdir().unwrap();
    let (w, h) = (64u32, 36u32);
    let mut r = Renderer::headless().unwrap();
    let clip = untagged_hd_clip(dir.path());
    let proxy = proxy_of(dir.path(), &clip);
    let bands = |r: &mut Renderer, from: &Path, marked: bool, interpret: Option<Value>| {
        let (mut doc, mut res) = document(from, (w, h), 25.0, interpret);
        if marked {
            doc["assets"]["clip"]["videoColor"] = json!("electron");
        }
        if from != clip {
            res.originals.insert("clip".into(), clip.clone());
        }
        let pixels = render(r, &doc, (w, h), 0.01, &res);
        let at = |band: usize| {
            let o = ((h as usize / 2) * w as usize + band * 16 + 8) * 4;
            [pixels[o], pixels[o + 1], pixels[o + 2]].map(f64::from)
        };
        [at(0), at(1), at(2), at(3)]
    };
    let apart = |a: [[f64; 3]; 4], b: [[f64; 3]; 4]| {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .map(|(a, b)| (a - b).abs())
            .fold(0., f64::max)
    };
    // ProRes Proxy on flat bands (and the 8-bit upload of the v1 reading).
    const CODEC: f64 = 0.012;
    let native = bands(&mut r, &clip, false, None);
    let v1 = bands(&mut r, &clip, true, None);
    assert!(
        apart(native, v1) > 0.03,
        "the clip reads differently as in v1: {native:?} {v1:?}"
    );
    for interpret in [None, Some(json!({ "matrix": "bt709" })), None] {
        for marked in [true, false] {
            let original = bands(&mut r, &clip, marked, interpret.clone());
            let copy = bands(&mut r, &proxy, marked, interpret.clone());
            let d = apart(original, copy);
            eprintln!("marked {marked}, {interpret:?}: proxy {d:.4} from the original");
            assert!(
                d < CODEC,
                "marked {marked}, {interpret:?}: proxy {copy:?}, original {original:?}"
            );
        }
    }
    // A matrix chosen by the person wins over the v1 reading.
    let chosen = bands(&mut r, &clip, true, Some(json!({ "matrix": "bt709" })));
    assert!(apart(chosen, native) < 2e-5, "{chosen:?} {native:?}");
}

#[test]
fn a_conformed_rate_shows_frame_n_at_n_over_rate() {
    let Some(path) = formats("video/vfr.mp4") else {
        return;
    };
    let src = MediaSource::open(&path).unwrap();
    let v = src.probe.video.as_ref().unwrap();
    let (i, _) = Interpretation::from_json(&json!({ "frameRate": 24 }));
    let reading = resolve_video(v, &i);
    let timing = Timing::new(&src, &reading, pm_time::Rate::FPS_24);
    // Section 5 rule 7 on the conformed grid: frame n shows from an eighth
    // of a frame before n / rate (q / 2, q a quarter frame), as
    // `pm_render::retime::override_frame` selects it.
    let frame = pm_time::Rate::FPS_24.frame_start(1).unwrap();
    let eighth = frame / 8;
    assert_eq!(eighth * 8, frame);
    let frames = src.pts.len() as i64;
    for n in 0..frames {
        let t = pm_time::Rate::FPS_24.frame_start(n).unwrap();
        assert_eq!(timing.pick(&src, t).unwrap().frame, n as usize);
        let early = t - eighth;
        assert_eq!(timing.pick(&src, early).unwrap().frame, n as usize);
        let late = t + frame - eighth - 1;
        assert_eq!(timing.pick(&src, late).unwrap().frame, n as usize);
        if n + 1 < frames {
            assert_eq!(timing.pick(&src, late + 1).unwrap().frame, n as usize + 1);
        }
        assert_eq!(
            pm_render::retime::override_frame_at(t, pm_time::Rate::FPS_24),
            Some(n as usize)
        );
    }
    assert_eq!(timing.pick(&src, -1).unwrap().frame, 0);
    assert!(timing.pick(&src, -eighth - 1).is_none());
    let last = pm_time::Rate::FPS_24
        .frame_start(src.pts.len() as i64 + 10)
        .unwrap();
    assert_eq!(timing.pick(&src, last).unwrap().frame, src.pts.len() - 1);
    // Without an override the timestamps decide (rule 7).
    let plain = Timing::new(
        &src,
        &resolve_video(v, &Interpretation::default()),
        pm_time::Rate::FPS_30,
    );
    for n in 0..src.pts.len() {
        assert_eq!(plain.pick(&src, src.time_of(n)).unwrap().frame, n);
    }
}

#[test]
fn wide_gamut_footage_converts_in_linear_light() {
    let Some(path) = formats("video/color-p3.mov") else {
        return;
    };
    let (w, h) = (320u32, 180u32);
    let mut r = Renderer::headless().unwrap();
    // The tagged reading and a restated one are the same conversion.
    let (doc, res) = document(&path, (w, h), 25.0, None);
    let ours = render(&mut r, &doc, (w, h), 0.1, &res);
    let (doc, res) = document(
        &path,
        (w, h),
        25.0,
        Some(json!({ "primaries": "displayP3" })),
    );
    assert_eq!(render(&mut r, &doc, (w, h), 0.1, &res), ours);
    let src = MediaSource::open(&path).unwrap();
    let f = decode(&path, src.select(ticks(0.1).get()).unwrap());
    let m = pm_color::primaries::conversion(
        pm_protocol::color::Primaries::DisplayP3,
        pm_protocol::color::Primaries::Bt709,
    )
    .unwrap();
    let want: Vec<[f64; 3]> = frame_rgb(&f, ChromaSiting::Left, MatrixCoefficients::Bt709)
        .into_iter()
        .map(|rgb| {
            let lin = rgb.map(pm_color::transfer::display_decode);
            pm_color::primaries::apply(&m, lin).map(pm_color::transfer::display_encode)
        })
        .collect();
    let d = max_diff(&ours, &want);
    eprintln!("color-p3.mov: P3 → Rec.709 in linear light, max |Δ| {d:.2e}");
    assert!(d < 1e-5, "{d}");
    // Converted colours outside Rec.709 are kept (no clip until output).
    assert!(
        ours.chunks_exact(4)
            .any(|p| p[..3].iter().any(|&v| v < 0.0))
    );
}

/// A still document: `path` (w × h) stretched over the whole frame.
fn still_document(
    path: &Path,
    (w, h): (u32, u32),
    interpret: Option<Value>,
) -> (Value, MapResolver) {
    let mut doc = project(
        w,
        h,
        1.0,
        no_fill(),
        vec![layer(
            "i",
            "image",
            json!({ "asset": "img", "fit": "stretch", "w": w, "h": h }),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            1.0,
        )],
    );
    doc["colorDepth"] = json!(32);
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    asset(&mut doc, "img", "image", &format, w, h, 0.0);
    if let Some(i) = interpret {
        doc["assets"]["img"]["interpret"] = i;
        doc["requiresEngine"] = json!("native");
    }
    let mut resolver = MapResolver::default();
    resolver.paths.insert("img".into(), path.to_path_buf());
    (doc, resolver)
}

#[test]
fn interpreted_stills_convert_other_profiles_through_colorsync() {
    let (Some(adobe), Some(reference)) = (
        formats("image/icc-adobergb.png"),
        formats("image/ref-srgb-icc-adobergb.png"),
    ) else {
        return;
    };
    let (w, h) = (320u32, 180u32);
    let mut r = Renderer::headless().unwrap();
    let (doc, res) = still_document(&adobe, (w, h), None);
    let request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32);
    let compatible = r
        .render(&load(&doc), "Ptest-root", ticks(0.0), &request, &res)
        .unwrap();
    assert!(compatible.failures.is_empty(), "{:?}", compatible.failures);
    // Level 1 now draws the Chromium/skcms compatibility bitmap. Explicit
    // interpretation still takes the original, high-precision ColorSync path.
    // Any interpretation that leaves color alone (here square pixels) draws
    // it, converted by ColorSync into linear Rec.709 and re-encoded with E.
    let (doc, res) = still_document(&adobe, (w, h), Some(json!({ "pixelAspect": [1, 1] })));
    let ours = render(&mut r, &doc, (w, h), 0.0, &res);
    let pool = pm_media::pool::FramePool::new(0);
    let want = pm_media::image::read(&reference, &pool).unwrap();
    let want_px = want
        .frame
        .with_planes(|p| {
            (0..h as usize)
                .flat_map(|y| {
                    let row = unsafe {
                        std::slice::from_raw_parts(p[0].ptr.add(y * p[0].stride), w as usize * 4)
                    };
                    row.to_vec()
                })
                .collect::<Vec<u8>>()
        })
        .unwrap();
    let (mut max, mut compared) = (0f64, 0usize);
    for i in 0..(w * h) as usize {
        let px = &ours[i * 4..i * 4 + 3];
        if px.iter().any(|v| !(-0.002..=1.002).contains(v)) {
            continue; // outside sRGB: kept unclipped, ColorSync's reference clipped
        }
        compared += 1;
        for c in 0..3 {
            max = max.max((f64::from(px[c]) * 255.0 - f64::from(want_px[i * 4 + c])).abs());
        }
    }
    eprintln!("icc-adobergb.png through the renderer: max |Δ| {max:.3} CV over {compared} pixels");
    // ColorSync's 8-bit path differs from its float path by up to 1.42 CV
    // near black at the gamut edge (pm-media's `icc_stills` checks the float
    // path against the f64 matrix/TRC conversion).
    assert!(compared > 30_000 && max <= 1.5, "{max} over {compared}");
    // A primaries override sets the profile aside: the stored samples are
    // read as sRGB, exactly.
    let (doc, res) = still_document(&adobe, (w, h), Some(json!({ "primaries": "bt709" })));
    let ours = render(&mut r, &doc, (w, h), 0.0, &res);
    let stored = pm_media::image::read(&adobe, &pool).unwrap();
    let stored_px = stored
        .frame
        .with_planes(|p| {
            (0..h as usize)
                .flat_map(|y| {
                    let row = unsafe {
                        std::slice::from_raw_parts(p[0].ptr.add(y * p[0].stride), w as usize * 4)
                    };
                    row.to_vec()
                })
                .collect::<Vec<u8>>()
        })
        .unwrap();
    let max = ours
        .chunks_exact(4)
        .zip(stored_px.chunks_exact(4))
        .flat_map(|(o, s)| (0..3).map(move |c| (f64::from(o[c]) * 255.0 - f64::from(s[c])).abs()))
        .fold(0f64, f64::max);
    assert!(max < 1e-3, "profile set aside: max {max} CV");
}

#[test]
fn interpreted_stills_take_alpha_overrides() {
    let Some(exr) = formats("image/exr-half-alpha.exr") else {
        return;
    };
    let (w, h) = (256u32, 16u32);
    let mut r = Renderer::headless().unwrap();
    let (doc, res) = still_document(&exr, (w, h), Some(json!({ "alpha": "ignore" })));
    let ours = render(&mut r, &doc, (w, h), 0.0, &res);
    assert!(ours.chunks_exact(4).all(|p| p[3] == 1.0));
    let (doc, res) = still_document(&exr, (w, h), Some(json!({ "alpha": "straight" })));
    let straight = render(&mut r, &doc, (w, h), 0.0, &res);
    // Column 100 of row 8: alpha 100/255; straight color premultiplied.
    let i = (8 * w as usize + 100) * 4;
    assert!(
        (straight[i + 3] - 100.0 / 255.0).abs() < 1e-3,
        "{:?}",
        &straight[i..i + 4]
    );
}

/// Adobe RGB (1998) → linear Rec.709, analytically: gamma 563/256, its
/// chromaticities, D65 (the oracle for ColorSync's matrix/TRC conversion).
fn adobe_to_linear_709(c: [f64; 3]) -> [f64; 3] {
    use pm_color::primaries::{Chromaticities, apply, chromaticities, rgb_to_xyz};
    let adobe = rgb_to_xyz(&Chromaticities {
        r: [0.64, 0.33],
        g: [0.21, 0.71],
        b: [0.15, 0.06],
        w: [0.3127, 0.3290],
    });
    let m = rgb_to_xyz(&chromaticities(pm_protocol::color::Primaries::Bt709).unwrap());
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let inv = [
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) / det,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / det,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / det,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) / det,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / det,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / det,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) / det,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / det,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / det,
        ],
    ];
    let lin = c.map(|v| v.max(0.0).powf(563.0 / 256.0));
    apply(&inv, apply(&adobe, lin))
}

/// A 64 × 8 RGBA8 PNG tagged with macOS's Adobe RGB (1998) profile: varied
/// colors per column, alpha from 255 down to 0 per row (straight in the
/// file). None where the system profile is missing.
fn adobe_png_with_alpha() -> Option<(PathBuf, Vec<[u8; 4]>)> {
    let profile = std::fs::read("/System/Library/ColorSync/Profiles/AdobeRGB1998.icc").ok()?;
    let alphas = [255u8, 230, 180, 128, 90, 64, 20, 0];
    let (w, h) = (64usize, alphas.len());
    let mut px = Vec::with_capacity(w * h);
    for (y, &a) in alphas.iter().enumerate() {
        for x in 0..w {
            // Saturated, dark, light and mid colors without a pattern.
            let v = |k: usize| ((x * (37 + 23 * k) + 11 * k + 5 * y) % 256) as u8;
            px.push([v(0), v(1), v(2), a]);
        }
    }
    let path = std::env::temp_dir().join(format!(
        "pm-render-footage-adobe-alpha-{}.png",
        std::process::id()
    ));
    let mut info = png::Info::with_size(w as u32, h as u32);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Eight;
    info.icc_profile = Some(profile.into());
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = png::Encoder::with_info(std::io::BufWriter::new(file), info)
        .unwrap()
        .write_header()
        .unwrap();
    writer.write_image_data(px.as_flattened()).unwrap();
    writer.finish().unwrap();
    Some((path, px))
}

/// Interpret Footage on a still whose ICC profile is neither sRGB nor
/// Display P3 (section 9): alpha overrides change how ColorSync's input is
/// read, never its output; primaries and transfer overrides set the
/// profile aside and read the stored samples, as on every other still.
#[test]
fn interpreted_icc_stills_read_alpha_and_profile_overrides() {
    let Some((path, px)) = adobe_png_with_alpha() else {
        eprintln!("AdobeRGB1998.icc missing (skipped)");
        return;
    };
    let (w, h) = (64u32, 8u32);
    let mut r = Renderer::headless().unwrap();
    let e = |v: f64| f64::from(pm_color::transfer::display_encode_f32(v as f32));
    let d = |v: f64| f64::from(pm_color::transfer::display_decode_f32(v as f32));
    let unit = |p: [u8; 4]| [p[0], p[1], p[2], p[3]].map(|v| f64::from(v) / 255.0);
    // Each case: the override, and the expected premultiplied display-encoded
    // pixel (None: not comparable, e.g. premultiplied data with color above
    // alpha).
    type Want = Box<dyn Fn([f64; 4]) -> Option<[f64; 4]>>;
    let colorsync = move |s: [f64; 3], a: f64| {
        let l = adobe_to_linear_709(s);
        Some([e(l[0]) * a, e(l[1]) * a, e(l[2]) * a, a])
    };
    let matte = [1.0, 128.0 / 255.0, 0.0];
    let cases: Vec<(&str, Value, Want)> = vec![
        (
            "as the file says (straight)",
            json!({ "pixelAspect": [1, 1] }),
            Box::new(move |c| colorsync([c[0], c[1], c[2]], c[3])),
        ),
        (
            "alpha ignore",
            json!({ "alpha": "ignore" }),
            Box::new(move |c| colorsync([c[0], c[1], c[2]], 1.0)),
        ),
        (
            "alpha straight",
            json!({ "alpha": "straight" }),
            Box::new(move |c| colorsync([c[0], c[1], c[2]], c[3])),
        ),
        (
            "alpha premultiplied",
            json!({ "alpha": "premultiplied" }),
            Box::new(move |c| {
                let a = c[3];
                (a > 0.0 && c[..3].iter().all(|&v| v <= a))
                    .then(|| colorsync([c[0] / a, c[1] / a, c[2] / a], a))
                    .flatten()
            }),
        ),
        (
            "alpha premultiplied with color",
            json!({ "alpha": "premultipliedWithColor", "alphaMatte": "#FF8000" }),
            Box::new(move |c| {
                let a = c[3];
                let removed = [0, 1, 2].map(|k| c[k] - (1.0 - a) * matte[k]);
                (a > 0.0 && removed.iter().all(|&v| (0.0..=a).contains(&v)))
                    .then(|| colorsync(removed.map(|v| v / a), a))
                    .flatten()
            }),
        ),
        (
            "transfer gamma 2.2 sets the profile aside",
            json!({ "transfer": "gamma22" }),
            Box::new(move |c| {
                let l = [0, 1, 2].map(|k| pm_color::transfer::gamma22_decode(c[k]));
                Some([e(l[0]) * c[3], e(l[1]) * c[3], e(l[2]) * c[3], c[3]])
            }),
        ),
        (
            "primaries Display P3 set the profile aside",
            json!({ "primaries": "displayP3" }),
            Box::new(move |c| {
                let m = pm_color::primaries::conversion(
                    pm_protocol::color::Primaries::DisplayP3,
                    pm_protocol::color::Primaries::Bt709,
                )
                .unwrap();
                let l = pm_color::primaries::apply(&m, [d(c[0]), d(c[1]), d(c[2])]);
                Some([e(l[0]) * c[3], e(l[1]) * c[3], e(l[2]) * c[3], c[3]])
            }),
        ),
    ];
    for (name, interp, want) in cases {
        let (doc, res) = still_document(&path, (w, h), Some(interp));
        let ours = render(&mut r, &doc, (w, h), 0.0, &res);
        let (mut max, mut compared) = (0f64, 0usize);
        for (i, p) in px.iter().enumerate() {
            let Some(want) = want(unit(*p)) else {
                continue;
            };
            compared += 1;
            for k in 0..4 {
                let d = (f64::from(ours[i * 4 + k]) - want[k]).abs() * 255.0;
                assert!(
                    d <= 0.75,
                    "{name}: pixel {i} channel {k}: {:?} want {want:?} (stored {p:?})",
                    &ours[i * 4..i * 4 + 4]
                );
                max = max.max(d);
            }
        }
        eprintln!("{name}: max |Δ| {max:.3} CV over {compared} pixels");
        assert!(compared >= 64, "{name}: {compared} pixels compared");
    }
    let _ = std::fs::remove_file(&path);
}

/// A still's stored samples as 0..1 RGBA (8- or 16-bit RGBA planes).
fn half_to_f32(bits: u16) -> f32 {
    pm_gpu::readback::half_to_f32(bits)
}

fn stored_rgba(path: &Path) -> (Vec<[f64; 4]>, pm_media::image::StillImage) {
    let pool = pm_media::pool::FramePool::new(0);
    let s = pm_media::image::read(path, &pool).unwrap();
    let f = &s.frame;
    let (w, h) = (f.format.width, f.format.height);
    let px = f
        .with_planes(|p| {
            let sixteen = p[0].desc.format == pm_media::frame::PlaneFormat::Rgba16;
            let half = p[0].desc.format == pm_media::frame::PlaneFormat::Rgba16Float;
            let mut out = Vec::with_capacity(w * h);
            for y in 0..h {
                for x in 0..w {
                    out.push([0, 1, 2, 3].map(|c| unsafe {
                        let row = p[0].ptr.add(y * p[0].stride);
                        if half {
                            // Linear float: as the display-encoded codes below.
                            let b = row.add(x * 8 + c * 2);
                            let v = half_to_f32(u16::from_le_bytes([*b, *b.add(1)]));
                            if c == 3 {
                                f64::from(v)
                            } else {
                                f64::from(pm_color::transfer::display_encode_f32(v))
                            }
                        } else if sixteen {
                            let b = row.add(x * 8 + c * 2);
                            f64::from(u16::from_le_bytes([*b, *b.add(1)])) / 65535.0
                        } else {
                            f64::from(*row.add(x * 4 + c)) / 255.0
                        }
                    }));
                }
            }
            out
        })
        .unwrap();
    (px, s)
}

/// CMYK stills (section 9): ImageIO converts them to sRGB through their
/// profile once, so with Interpret Footage overrides the renderer draws
/// those samples and never applies the profile again (pm-media's
/// `converted_stills_are_srgb_once` compares the samples with ColorSync's
/// conversion); without overrides level 1 draws them through Chromium's
/// own CMYK conversion (stills2). A straight-alpha CMYK TIFF keeps its
/// alpha reading.
#[test]
fn interpreted_cmyk_stills_draw_their_converted_samples() {
    let (Some(jpg), Some(tiffa)) = (formats("image/cmyk.jpg"), formats("image/cmyka.tiff")) else {
        return;
    };
    let mut r = Renderer::headless().unwrap();
    let request = |w, h| RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32);
    let e = |v: f64| f64::from(pm_color::transfer::display_encode_f32(v as f32));
    let d = |v: f64| f64::from(pm_color::transfer::display_decode_f32(v as f32));
    let p3 = pm_color::primaries::conversion(
        pm_protocol::color::Primaries::DisplayP3,
        pm_protocol::color::Primaries::Bt709,
    )
    .unwrap();
    type Want = Box<dyn Fn([f64; 4]) -> [f64; 4]>;
    for (path, (w, h), straight) in [(&jpg, (320u32, 180u32), false), (&tiffa, (96, 64), true)] {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (doc, res) = still_document(path, (w, h), None);
        assert!(
            r.render(&load(&doc), "Ptest-root", ticks(0.0), &request(w, h), &res)
                .is_ok(),
            "{name}: level 1 draws CMYK stills without an interpretation"
        );
        let (stored, still) = stored_rgba(path);
        assert_eq!(
            still.color.icc_class,
            Some(pm_media::image::icc::IccClass::ConvertedToSrgb)
        );
        let cases: Vec<(Value, Want)> = vec![
            // As the file says: straight color times alpha (opaque: as is).
            (
                json!({ "pixelAspect": [1, 1] }),
                Box::new(|c| [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]),
            ),
            (
                json!({ "alpha": "ignore" }),
                Box::new(|c| [c[0], c[1], c[2], 1.0]),
            ),
            (
                json!({ "primaries": "displayP3" }),
                Box::new(move |c| {
                    let l = pm_color::primaries::apply(&p3, [d(c[0]), d(c[1]), d(c[2])]);
                    [e(l[0]) * c[3], e(l[1]) * c[3], e(l[2]) * c[3], c[3]]
                }),
            ),
        ];
        for (interp, want) in cases {
            let i = Interpretation::from_json(&interp).0;
            assert_eq!(
                pm_render::media::probe_interpreted_image(path, &i),
                Ok(()),
                "{name} {interp}"
            );
            let (doc, res) = still_document(path, (w, h), Some(interp.clone()));
            let ours = render(&mut r, &doc, (w, h), 0.0, &res);
            let mut max = 0f64;
            for (k, c) in stored.iter().enumerate() {
                let want = want(*c);
                for ch in 0..4 {
                    max = max.max((f64::from(ours[k * 4 + ch]) - want[ch]).abs() * 255.0);
                }
            }
            eprintln!("{name} {interp}: max |Δ| {max:.4} CV against its converted samples");
            assert!(max <= 0.05, "{name} {interp}: {max} CV");
        }
        assert_eq!(
            straight,
            still.color.alpha == pm_protocol::color::AlphaMode::Straight
        );
    }
}

/// The interpreted probes decode a still, or open and index a video, once
/// per file size and modification time; each interpretation is resolved
/// from those facts (capability routing asks at every document revision).
/// Shown by replacing a file's bytes while keeping its size and time: the
/// answers stay; a new time reads the file again.
#[test]
fn interpreted_probes_follow_the_file_stamp() {
    use pm_render::media::{ProbeIssue, probe_interpreted_image, probe_interpreted_video};
    let dir = std::env::temp_dir().join(format!("pm-render-probe-stamp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let interps = [
        json!({ "alpha": "ignore" }),
        json!({ "primaries": "displayP3" }),
        json!({ "transfer": "gamma22", "alphaMatte": "#102030" }),
    ]
    .map(|j| Interpretation::from_json(&j).0);
    // Same length, modification time restored: only the change time (and
    // the bytes) say the file is another version.
    let rewrite = |path: &Path| {
        let m = std::fs::metadata(path).unwrap();
        let modified = m.modified().unwrap();
        std::fs::write(path, vec![0u8; m.len() as usize]).unwrap();
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(modified).unwrap();
    };

    let still = dir.join("still.png");
    let mut info = png::Info::with_size(16, 8);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Sixteen;
    let file = std::fs::File::create(&still).unwrap();
    let mut writer = png::Encoder::with_info(std::io::BufWriter::new(file), info)
        .unwrap()
        .write_header()
        .unwrap();
    writer.write_image_data(&[0x40u8; 16 * 8 * 8]).unwrap();
    writer.finish().unwrap();
    for i in &interps {
        assert_eq!(probe_interpreted_image(&still, i), Ok(()));
    }
    rewrite(&still);
    for i in &interps {
        assert!(matches!(
            probe_interpreted_image(&still, i),
            Err(ProbeIssue::Unreadable(_))
        ));
    }

    if let Some(video) = formats("video/geom-base.mov") {
        let copy = dir.join("video.mov");
        std::fs::copy(&video, &copy).unwrap();
        for i in &interps {
            assert_eq!(probe_interpreted_video(&copy, i), Ok(()));
        }
        rewrite(&copy);
        for i in &interps {
            assert!(probe_interpreted_video(&copy, i).is_err());
        }
    }
}

/// A log reading through the whole video path: the same decoded codes, read
/// raw and read as camera log, agree with the vendor curve in f64.
#[test]
fn camera_log_interpretation_reads_real_video_through_the_vendor_curve() {
    use pm_reference::camera_log::Curve;
    use pm_reference::color::{Primaries as P, Transfer as T, apply, invert, mul};
    let Some(path) = formats("video/bars709-prores4444.mov") else {
        return;
    };
    let src = MediaSource::open(&path).unwrap();
    let v = src.probe.video.as_ref().unwrap();
    let size = (v.width, v.height);
    let mut r = Renderer::headless().unwrap();
    let mut read = |interpret: Value| {
        let (mut doc, res) = document(&path, size, 25.0, Some(interpret));
        doc["colorDepth"] = json!(32);
        render(&mut r, &doc, size, 0.0, &res)
    };
    let raw = read(json!({ "transfer": "linear", "primaries": "bt709" }));
    let log = read(json!({ "transfer": "sLog3", "primaries": "sGamut3Cine" }));
    let gamut = mul(&invert(&P::BT709.to_xyz()), &P::S_GAMUT3_CINE.to_xyz());
    let mut worst = 0f64;
    for (a, b) in raw.chunks_exact(4).zip(log.chunks_exact(4)) {
        // Linear codes reach the E-encoded output; undo that to get the code.
        let codes = [a[0], a[1], a[2]].map(|c| T::Srgb.to_linear(f64::from(c)));
        let light = apply(&gamut, codes.map(|c| Curve::SLog3.to_linear(c)));
        for c in 0..3 {
            let expected = T::Srgb.from_linear(light[c]);
            worst = worst.max((f64::from(b[c]) - expected).abs());
        }
    }
    assert!(worst < 5e-3, "worst {worst}");
}

/// Anamorphic video fills the quad the document authors: the coded frame is
/// stretched to it, so pixel aspect moves no authored geometry.
#[test]
fn anamorphic_video_fills_its_authored_display_size() {
    let Some(path) = formats("video/meta-pasp.mov") else {
        return;
    };
    let src = MediaSource::open(&path).unwrap();
    let v = src.probe.video.as_ref().unwrap();
    assert_ne!(v.pixel_aspect.0, v.pixel_aspect.1);
    let reading = resolve_video(v, &Interpretation::default());
    let display = reading.display_size((v.width, v.height));
    assert_ne!(display, (v.width, v.height));
    let mut r = Renderer::headless().unwrap();
    let (doc, res) = document(&path, display, 25.0, None);
    let drawn = render(&mut r, &doc, display, 0.0, &res);
    let (restated, res2) = document(
        &path,
        display,
        25.0,
        Some(json!({ "pixelAspect": [v.pixel_aspect.0, v.pixel_aspect.1] })),
    );
    assert_eq!(render(&mut r, &restated, display, 0.0, &res2), drawn);
}

/// Native has no fallback: everything with a reading draws by default, and
/// the default draws what an override restating the file would draw.
#[test]
fn every_source_with_a_reading_draws_without_an_interpretation() {
    let (restated, _) = Interpretation::from_json(&json!({ "cleanAperture": false }));
    for file in [
        "hdr/hdr10.mov",
        "hdr/hlg.mov",
        "hdr/pq-av1.mkv",
        "hdr/pq-vp9.webm",
        "hdr/dovi-8.4.mov",
        "video/interlaced-bff.mxf",
        "video/interlaced-tff.mov",
        "video/dv-pal.dv",
        "video/geom-rot90.mov",
        "video/geom-rot180.mov",
        "video/geom-rot270.mov",
        "video/geom-clap.mov",
        "video/meta-pasp.mov",
        "video/color-p3.mov",
    ] {
        let Some(path) = formats(file) else {
            return;
        };
        assert_eq!(pm_render::media::probe_video(&path), Ok(()), "{file}");
        assert_eq!(
            pm_render::media::probe_interpreted_video(&path, &restated),
            Ok(()),
            "{file} interpreted"
        );
    }
    // Progressive, unrotated, square-pixel Rec.709 and BT.601 footage is
    // drawn as before.
    for file in ["video/geom-base.mov", "video/color-601-625.mov"] {
        let Some(path) = formats(file) else {
            return;
        };
        assert_eq!(pm_render::media::probe_video(&path), Ok(()), "{file}");
    }
}
