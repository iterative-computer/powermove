//! The one video reading (docs/engine/video-parity.md) against an f64
//! reference, QUALITY-1's proof for video.
//!
//! The reference converts each case's decoded Y′CbCr in f64 with the
//! stream's reading (pm-media's untagged-video table): range from the
//! decoder's format, section 9's siting-aware chroma filters, the matrix
//! from its Kr/Kb, primaries in linear light, and today's RWTMO curve for
//! HDR. Like a display (and the source monitor's gate), it clips the video
//! signal to 0..1 before linearising it. A 1:1 video layer is rendered at all three project depths and
//! compared with it in display code units, as is today's viewer: its paused
//! (Canvas) and playing (direct upload) captures of the same frame.
//!
//! "Flat" pixels are those whose chroma neighbourhood is constant, where no
//! chroma filter or siting choice changes the answer: there the comparison
//! is pure arithmetic and storage precision.

#[path = "support/doc.rs"]
mod doc;

use pm_color::chroma::{Factors, filters, sample_origin, taps};
use pm_color::primaries::{apply, conversion};
use pm_color::tonemap::{HdrSignal, rwtmo};
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::frame::DecodedFrame;
use pm_media::interpret::{Interpretation, resolve_video};
use pm_media::vt::DecodePurpose;
use pm_protocol::color::{AlphaMode, ColorDescription, MatrixCoefficients, Precision, Primaries, Transfer};
use pm_protocol::common::{Size, Ticks};
use pm_render::footage::Timing;
use pm_render::sampling::Sampling;
use pm_render::{MapResolver, RenderRequest, Renderer, VideoEnd};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};

struct Case {
    id: String,
    path: PathBuf,
    time: f64,
    /// Today's viewer: (reading, premultiplied RGBA8 rows).
    today: Vec<(&'static str, Vec<u8>)>,
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video")
}

fn raster(file: &str) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(&std::fs::read(fixtures().join(file)).unwrap()[..])
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn cases() -> Vec<Case> {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(fixtures().join("cases.json")).unwrap()).unwrap();
    let mut cases: Vec<Case> = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| Case {
            id: c["id"].as_str().unwrap().into(),
            path: fixtures().join(c["media"].as_str().unwrap()),
            time: c["time"].as_f64().unwrap(),
            today: ["paused", "playing"]
                .into_iter()
                .map(|kind| (kind, raster(c[kind].as_str().unwrap())))
                .collect(),
        })
        .collect();
    // Today's viewer cannot decode ProRes (it plays a VP9 rendition): the
    // reference bound alone applies.
    let e2e = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures");
    for id in ["prores422", "prores4444-alpha"] {
        cases.push(Case {
            id: id.into(),
            path: e2e.join(format!("{id}.mov")),
            time: 0.25,
            today: vec![],
        });
    }
    cases
}

/// The decoded frame a composition time shows, as the renderer selects it.
fn decoded(case: &Case) -> (ColorDescription, DecodedFrame) {
    let src = MediaSource::open(&case.path).unwrap();
    let v = src.probe.video.as_ref().unwrap();
    let reading = resolve_video(v, &Interpretation::default());
    let timing = Timing::new(&src, &reading, pm_time::Rate::FPS_30);
    let ticks = (case.time * pm_time::TICKS_PER_SECOND as f64).round() as i64;
    let pick = timing.pick(&src, ticks).unwrap();
    let service = DecodeService::new(ServiceConfig::default());
    let frame = service
        .session(&src, 1, DecodePurpose::Export)
        .unwrap()
        .frame(pick.frame)
        .unwrap();
    (v.color.interpreted, frame)
}

fn e_encode(l: f64) -> f64 {
    let m = l.abs();
    let e = if m <= 0.003_130_8 {
        12.92 * m
    } else {
        1.055 * m.powf(1.0 / 2.4) - 0.055
    };
    e.copysign(l)
}

fn e_decode(e: f64) -> f64 {
    let m = e.abs();
    let l = if m <= 0.040_45 {
        m / 12.92
    } else {
        ((m + 0.055) / 1.055).powf(2.4)
    };
    l.copysign(e)
}

/// Premultiplied Standard working values in code units (unclamped), and
/// whether each pixel's chroma neighbourhood is flat.
fn reference(c: &ColorDescription, frame: &DecodedFrame) -> (Vec<[f64; 4]>, Vec<bool>) {
    let (ya, cc) = frame.reference().unwrap();
    let f = &frame.format;
    let (w, h) = (f.width as u32, f.height as u32);
    let factors = Factors::from_log2(f.log2_chroma_w, f.log2_chroma_h);
    let (cw, ch) = factors.chroma_size(w, h);
    let (ox, oy) = sample_origin(c.siting, factors);
    let (fh, fv) = filters(factors);
    let (kr, kb) = match c.matrix {
        MatrixCoefficients::Bt601 => (0.299, 0.114),
        MatrixCoefficients::Bt709 => (0.2126, 0.0722),
        MatrixCoefficients::Bt2020Ncl => (0.2627, 0.0593),
        other => panic!("no fixture reads {other:?}"),
    };
    // Tagged BT.601 primaries draw as Rec.709 (VIDEO-SD-1).
    let primaries = match c.primaries {
        Primaries::Bt601_525 | Primaries::Bt601_625 => Primaries::Bt709,
        other => other,
    };
    let chroma = |i: i64, j: i64| {
        cc[(j.clamp(0, i64::from(ch) - 1) * i64::from(cw) + i.clamp(0, i64::from(cw) - 1))
            as usize]
    };
    let mut out = Vec::with_capacity((w * h) as usize);
    let mut flat = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let ty = taps(y, oy, factors.y, ch, fv);
        for x in 0..w {
            let tx = taps(x, ox, factors.x, cw, fh);
            let mut cbcr = [0.0f64; 2];
            for &(j, wy) in &ty {
                for &(i, wx) in &tx {
                    let s = cc[(j * cw + i) as usize];
                    cbcr[0] += wx * wy * f64::from(s[0]);
                    cbcr[1] += wx * wy * f64::from(s[1]);
                }
            }
            let (i0, j0) = (
                ((f64::from(x) - ox) / f64::from(factors.x)).floor() as i64,
                ((f64::from(y) - oy) / f64::from(factors.y)).floor() as i64,
            );
            let centre = chroma(i0, j0);
            flat.push(
                (j0 - 2..=j0 + 2).all(|j| (i0 - 2..=i0 + 2).all(|i| chroma(i, j) == centre)),
            );
            let [luma, alpha] = ya[(y * w + x) as usize].map(f64::from);
            let r = luma + 2.0 * (1.0 - kr) * cbcr[1];
            let b = luma + 2.0 * (1.0 - kb) * cbcr[0];
            let rgb = [r, (luma - kr * r - kb * b) / (1.0 - kr - kb), b];
            let rgb = match c.transfer {
                Transfer::Pq | Transfer::Hlg => {
                    let signal = if c.transfer == Transfer::Pq {
                        HdrSignal::Pq
                    } else {
                        HdrSignal::Hlg
                    };
                    let to_2020 = conversion(primaries, Primaries::Bt2020).unwrap();
                    rwtmo::to_sdr(signal, rgb.map(|v| v.clamp(0.0, 1.0)), &to_2020).map(e_encode)
                }
                _ if primaries == Primaries::Bt709 => rgb,
                _ => {
                    let m = conversion(primaries, Primaries::Bt709).unwrap();
                    apply(&m, rgb.map(|v| e_decode(v.clamp(0.0, 1.0)))).map(e_encode)
                }
            };
            // A file tagged premultiplied (HEVC's AlphaChannelMode) already
            // holds premultiplied colour; straight colour is multiplied here.
            let k = match c.alpha {
                AlphaMode::Premultiplied => 255.0,
                _ => alpha * 255.0,
            };
            out.push([rgb[0] * k, rgb[1] * k, rgb[2] * k, alpha * 255.0]);
        }
    }
    (out, flat)
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
struct Distance {
    max: f64,
    mean: f64,
    flat_max: f64,
    flat_mean: f64,
}

/// Display code distance (both sides clamped to the codable range).
fn distance(actual: impl Fn(usize) -> [f64; 4], reference: &[[f64; 4]], flat: &[bool]) -> Distance {
    let (mut d, mut n, mut flat_n) = (Distance::default(), 0.0, 0.0);
    for (p, (want, &flat)) in reference.iter().zip(flat).enumerate() {
        let got = actual(p);
        for c in 0..4 {
            let e = (got[c].clamp(0.0, 255.0) - want[c].clamp(0.0, 255.0)).abs();
            d.max = d.max.max(e);
            d.mean += e;
            n += 1.0;
            if flat {
                d.flat_max = d.flat_max.max(e);
                d.flat_mean += e;
                flat_n += 1.0;
            }
        }
    }
    d.mean /= n;
    d.flat_mean /= f64::max(flat_n, 1.0);
    d
}

fn project(case: &Case, w: u32, h: u32, depth: u32) -> pm_model::Document {
    let dur = 4.0;
    let mut p = doc::project(
        w,
        h,
        dur,
        doc::no_fill(),
        vec![doc::layer(
            "v",
            "video",
            json!({"asset":"clip","fit":"stretch","w":w,"h":h,"trim":0,"speed":1}),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            dur,
        )],
    );
    let ext = case.path.extension().unwrap().to_str().unwrap();
    doc::asset(&mut p, "clip", "video", ext, w, h, dur);
    p["colorDepth"] = depth.into();
    doc::load(&p)
}

/// The rendered frame at `time` in working values (×255).
fn render(
    renderer: &mut Renderer,
    case: &Case,
    (w, h): (u32, u32),
    depth: u32,
    export: bool,
    sampling: Sampling,
) -> Vec<f32> {
    let document = project(case, w, h, depth);
    let mut assets = MapResolver::default();
    assets.paths.insert("clip".into(), case.path.clone());
    let request = RenderRequest {
        export,
        sampling,
        video_end: VideoEnd::Final,
        ..RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32)
    };
    let tick = Ticks::new((case.time * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap();
    let frame = renderer
        .render(&document, "Ptest-root", tick, &request, &assets)
        .unwrap();
    assert!(frame.failures.is_empty(), "{}: {:?}", case.id, frame.failures);
    let rgba = renderer.read_rgba(&frame).unwrap();
    renderer.recycle(frame);
    rgba
}

/// The engine's stated distance bound from the f64 reference at a
/// project depth, in display codes. 8-bit output rounds once (half a
/// code), after half-float source storage for deep and HDR sources (half
/// an f16 step below 1.0, 0.125 codes). Half float rounds twice (source
/// and output, 2⁻¹² each). Single float is bounded by the GPU's f32
/// transcendentals in the PQ curve.
fn bound(depth: u32) -> f64 {
    match depth {
        8 => 0.625 + 1e-3,
        16 => 0.125 + 0.05,
        _ => 0.05,
    }
}

/// Today's 8-bit captures and an 8-bit engine output can both be the
/// correctly rounded code: their distances then differ by f32 noise only.
const TIE: f64 = 1e-3;

#[test]
fn video_reading_is_at_least_as_close_to_the_f64_reference_as_today() {
    let mut renderer = Renderer::headless().unwrap();
    let mut failures = Vec::new();
    for case in cases() {
        let (color, frame) = decoded(&case);
        let (w, h) = (frame.format.width as u32, frame.format.height as u32);
        let (reference, flat) = reference(&color, &frame);
        drop(frame);
        let today: Vec<(&str, Distance)> = case
            .today
            .iter()
            .map(|(kind, bytes)| {
                assert_eq!(bytes.len(), reference.len() * 4, "{}", case.id);
                let d = distance(
                    |p| std::array::from_fn(|c| f64::from(bytes[p * 4 + c])),
                    &reference,
                    &flat,
                );
                (*kind, d)
            })
            .collect();
        for depth in [8, 16, 32] {
            let rgba = render(&mut renderer, &case, (w, h), depth, true, Sampling::Final);
            let engine = distance(
                |p| std::array::from_fn(|c| f64::from(rgba[p * 4 + c]) * 255.0),
                &reference,
                &flat,
            );
            println!(
                "VIDEO-REFERENCE {}",
                json!({"case": case.id, "depth": depth, "engine": engine,
                    "today": today.iter().map(|(k, d)| json!({"reading": k, "distance": d}))
                        .collect::<Vec<_>>(),
                    "flatPixels": flat.iter().filter(|&&f| f).count(), "pixels": flat.len()})
            );
            if engine.max > bound(depth) {
                failures.push(format!(
                    "{} {depth}-bit: max {:.4} codes over the {:.3} bound",
                    case.id,
                    engine.max,
                    bound(depth)
                ));
            }
            for (kind, t) in &today {
                if engine.max > t.max + TIE
                    || engine.mean > t.mean + TIE
                    || engine.flat_max > t.flat_max + TIE
                {
                    failures.push(format!(
                        "{} {depth}-bit: farther than today's {kind} reading: {engine:?} vs {t:?}",
                        case.id
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn playback_pause_and_export_draw_the_same_video_frame() {
    let mut renderer = Renderer::headless().unwrap();
    for case in cases() {
        let (_, frame) = decoded(&case);
        let size = (frame.format.width as u32, frame.format.height as u32);
        drop(frame);
        for depth in [8, 16, 32] {
            let export = render(&mut renderer, &case, size, depth, true, Sampling::Final);
            let paused = render(&mut renderer, &case, size, depth, false, Sampling::Final);
            let playing = render(&mut renderer, &case, size, depth, false, Sampling::Playback);
            assert_eq!(paused, export, "{} {depth}-bit: paused vs export", case.id);
            assert_eq!(playing, export, "{} {depth}-bit: playing vs export", case.id);
        }
    }
}
