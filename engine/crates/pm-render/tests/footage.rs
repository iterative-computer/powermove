//! Footage as interpreted, through the renderer (plan sections 9 and 13):
//!
//! - HDR10 and HLG sources tone mapped with BT.2446 Method A equal the f64
//!   oracle (`pm_color::tonemap`) applied to the same decode;
//! - interlaced sources at field rate equal FFmpeg's bwdif
//!   (`mode=send_field`) converted with the same matrix and chroma filter,
//!   and at frame rate show each frame's first field;
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
use pm_color::tonemap::{HdrSignal, hdr_to_sdr};
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

fn max_diff(ours: &[f32], want: &[[f64; 3]]) -> f64 {
    want.iter()
        .enumerate()
        .flat_map(|(i, w)| (0..3).map(move |c| (i, c, w[c])))
        .map(|(i, c, w)| (f64::from(ours[i * 4 + c]) - w).abs())
        .fold(0.0, f64::max)
}

#[test]
fn hdr_sources_are_tone_mapped_with_method_a() {
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
        let (doc, res) = document(&path, (w, h), 25.0, None);
        let mut r = Renderer::headless().unwrap();
        // Drawn as probed; an override that restates the file's own reading
        // changes nothing.
        let (doc_i, _) = document(
            &path,
            (w, h),
            25.0,
            Some(json!({ "transfer": if signal == HdrSignal::Pq { "pq" } else { "hlg" } })),
        );
        let ours = render(&mut r, &doc, (w, h), 0.13, &res);
        assert_eq!(render(&mut r, &doc_i, (w, h), 0.13, &res), ours);
        let n = src.select(ticks(0.13).get()).unwrap();
        let f = decode(&path, n);
        let want: Vec<[f64; 3]> = frame_rgb(&f, reading.siting.value, reading.matrix.value)
            .into_iter()
            .map(|rgb| hdr_to_sdr(signal, rgb, tm.peak_nits))
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
        // f32 on the GPU against f64: the largest errors sit on channels near
        // zero after the BT.2020 → Rec.709 matrix (cancellation, then the
        // 1/2.4 power): 6.8e-4 on HDR10, a quarter of an 8-bit code value.
        assert!(d < 1e-3, "{file}: {d}");
    }
}

#[test]
fn interlaced_sources_deinterlace_like_ffmpeg_bwdif() {
    let Some(path) = formats("video/interlaced-bff.mxf") else {
        return;
    };
    let src = MediaSource::open(&path).unwrap();
    let v = src.probe.video.as_ref().unwrap();
    let reading = resolve_video(v, &Interpretation::default());
    let (w, h) = (v.width as usize, v.height as usize);
    let (cw, ch) = (w / 2, h);
    // FFmpeg's bwdif, one picture per field.
    let raw = std::env::temp_dir().join(format!("pm-render-bwdif-{}.raw", std::process::id()));
    let ffmpeg = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Caches/powermove-engine/ffmpeg/out/bin/ffmpeg");
    let ok = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-i"])
        .arg(&path)
        .args([
            "-map",
            "0:v:0",
            "-vf",
            "bwdif=mode=send_field:parity=auto:deint=all",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv422p",
        ])
        .arg(&raw)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let reference = std::fs::read(&raw).unwrap();
    let _ = std::fs::remove_file(&raw);
    let frame_bytes = w * h + 2 * cw * ch;
    let mut r = Renderer::headless().unwrap();
    let (doc50, res) = document(&path, (w as u32, h as u32), 50.0, None);
    let (doc25, _) = document(&path, (w as u32, h as u32), 25.0, None);
    let mut worst = 0f64;
    for field in [0usize, 1, 2, 3, 20, 47, 48, 49] {
        let t = (field as f64 + 0.25) / 50.0;
        let ours = render(&mut r, &doc50, (w as u32, h as u32), t, &res);
        let p = &reference[field * frame_bytes..][..frame_bytes];
        let y: Vec<f32> = p[..w * h]
            .iter()
            .map(|&v| (f32::from(v) - 16.0) / 219.0)
            .collect();
        let cb: Vec<f32> = p[w * h..][..cw * ch]
            .iter()
            .map(|&v| (f32::from(v) - 128.0) / 224.0)
            .collect();
        let cr: Vec<f32> = p[w * h + cw * ch..][..cw * ch]
            .iter()
            .map(|&v| (f32::from(v) - 128.0) / 224.0)
            .collect();
        let want = rgb_of(
            (w, h),
            &y,
            &cb,
            &cr,
            (cw, ch),
            Factors { x: 2, y: 1 },
            reading.siting.value,
            reading.matrix.value,
        );
        let d = max_diff(&ours, &want);
        worst = worst.max(d);
        assert!(d < 1e-5, "field {field}: max |Δ| {d}");
        // At 25 fps the frame shows its first field.
        if field % 2 == 0 {
            let frame = render(&mut r, &doc25, (w as u32, h as u32), t, &res);
            assert_eq!(frame, ours, "frame-rate output keeps the first field");
        }
    }
    eprintln!("interlaced-bff.mxf at 50 fps: max |Δ| {worst:.2e} against FFmpeg's bwdif");
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
        // The renderer's picture is the same permutation of the unrotated
        // picture, exactly.
        let (doc_r, res_r) = document(&path, (ow, oh), 25.0, None);
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
    // The clean aperture: the full frame by default, the crop on request.
    let clap = formats("video/geom-clap.mov").unwrap();
    let (doc_c, res_c) = document(&clap, (w as u32, h as u32), 25.0, None);
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
    for (interp, matrix) in [
        (None, MatrixCoefficients::Bt601),
        (
            Some(json!({ "matrix": "bt709" })),
            MatrixCoefficients::Bt709,
        ),
    ] {
        let (doc, res) = document(&path, (w, h), 25.0, interp.clone());
        let ours = render(&mut r, &doc, (w, h), 0.1, &res);
        let want = frame_rgb(&f, ChromaSiting::Left, matrix);
        let d = max_diff(&ours, &want);
        eprintln!("color-601-625.mov read with {matrix:?}: max |Δ| {d:.2e}");
        assert!(d < 1e-5, "{interp:?}: {d}");
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
    for n in 0..src.pts.len() as i64 {
        let t = pm_time::Rate::FPS_24.frame_start(n).unwrap();
        assert_eq!(timing.pick(&src, t).unwrap().frame, n as usize);
        let late = t + pm_time::Rate::FPS_24.frame_start(1).unwrap() - 1;
        assert_eq!(timing.pick(&src, late).unwrap().frame, n as usize);
    }
    assert!(timing.pick(&src, -1).is_none());
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
fn wide_gamut_footage_converts_in_linear_light_when_interpreted() {
    let Some(path) = formats("video/color-p3.mov") else {
        return;
    };
    let (w, h) = (320u32, 180u32);
    let mut r = Renderer::headless().unwrap();
    let (doc, res) = document(&path, (w, h), 25.0, None);
    let request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32);
    assert!(
        r.render(&load(&doc), "Ptest-root", ticks(0.1), &request, &res)
            .is_err(),
        "Display P3 footage is routed without an interpretation"
    );
    let (doc, res) = document(
        &path,
        (w, h),
        25.0,
        Some(json!({ "primaries": "displayP3" })),
    );
    let ours = render(&mut r, &doc, (w, h), 0.1, &res);
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
    // Out-of-gamut values are kept (no clip until output).
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
    asset(&mut doc, "img", "image", "png", w, h, 0.0);
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
    assert!(
        r.render(&load(&doc), "Ptest-root", ticks(0.0), &request, &res)
            .is_err(),
        "Adobe RGB stills are routed without an interpretation"
    );
    // Any interpretation (here the file's own primaries) draws it, converted
    // by ColorSync into linear Rec.709 and re-encoded with E.
    let (doc, res) = still_document(&adobe, (w, h), Some(json!({ "primaries": "bt709" })));
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
    assert!(compared > 30_000 && max <= 1.0, "{max} over {compared}");
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
