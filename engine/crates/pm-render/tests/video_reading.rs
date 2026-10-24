//! The one video reading (docs/engine/video-parity.md) against an f64
//! reference, QUALITY-1's proof for video.
//!
//! The reference shares no colour code with the engine
//! (`support/standards.rs`): each clip's reading is written by hand in
//! `cases.json` from its tags and the shared table's rules (and the probe
//! must agree); raw decoded codes are normalized per H.273; chroma is
//! reconstructed with Lanczos-3 at the tagged siting; the matrix comes from
//! Kr/Kb, primaries convert in linear light through matrices derived from
//! the published chromaticities, the sRGB curve is extended (superwhites
//! kept), and PQ/HLG follow ST 2084, BT.2100 and the BT.2390 EETF.
//!
//! A 1:1 video layer is rendered at all three project depths and compared
//! with it in display codes, as is today's viewer (its paused and playing
//! captures). "Flat" pixels have constant chroma across the reconstruction
//! footprint, where no filter or siting choice changes the answer: there
//! the engine must be within its precision bound. Everywhere, it must be at
//! least as close as today on average. The filter itself is proven against
//! 4:4:4 truth in tests/chroma_truth.rs.

#[path = "support/doc.rs"]
mod doc;
#[path = "support/standards.rs"]
mod standards;

use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::frame::{DecodedFrame, Layout};
use pm_media::interpret::{Interpretation, resolve_video};
use pm_media::vt::DecodePurpose;
use pm_protocol::color::{
    AlphaMode, ChromaSiting, MatrixCoefficients, Precision, Primaries, Range, SampleAlignment,
    Transfer,
};
use pm_protocol::common::{Size, Ticks};
use pm_render::footage::Timing;
use pm_render::sampling::Sampling;
use pm_render::{MapResolver, RenderRequest, Renderer, VideoEnd};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};

/// A clip's reading as H.273 names, written by hand in cases.json.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
struct Reading {
    matrix: String,
    primaries: String,
    transfer: String,
    range: String,
    siting: String,
    alpha: String,
}

struct Case {
    id: String,
    path: PathBuf,
    time: f64,
    reading: Reading,
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
    manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| Case {
            id: c["id"].as_str().unwrap().into(),
            path: fixtures().join(c["media"].as_str().unwrap()),
            time: c["time"].as_f64().unwrap(),
            reading: serde_json::from_value(c["reading"].clone()).unwrap(),
            today: ["paused", "playing"]
                .into_iter()
                .filter_map(|kind| c[kind].as_str().map(|f| (kind, raster(f))))
                .collect(),
        })
        .collect()
}

/// The decoded frame a composition time shows, as the renderer selects it,
/// after checking the probe reads the clip as cases.json says.
fn decoded(case: &Case) -> DecodedFrame {
    let src = MediaSource::open(&case.path).unwrap();
    let v = src.probe.video.as_ref().unwrap();
    let c = v.color.interpreted;
    let probed = Reading {
        matrix: match c.matrix {
            MatrixCoefficients::Bt601 => "bt601",
            MatrixCoefficients::Bt709 => "bt709",
            MatrixCoefficients::Bt2020Ncl => "bt2020",
            other => panic!("{other:?}"),
        }
        .into(),
        primaries: match c.primaries {
            Primaries::Bt709 => "bt709",
            Primaries::Bt601_525 => "smpte170m",
            Primaries::Bt601_625 => "bt470bg",
            Primaries::Bt2020 => "bt2020",
            Primaries::DisplayP3 => "p3",
            other => panic!("{other:?}"),
        }
        .into(),
        transfer: match c.transfer {
            Transfer::DisplayE => "srgb",
            Transfer::Pq => "pq",
            Transfer::Hlg => "hlg",
            other => panic!("{other:?}"),
        }
        .into(),
        range: if c.range == Range::Full {
            "full"
        } else {
            "limited"
        }
        .into(),
        siting: if (v.codec.log2_chroma_w, v.codec.log2_chroma_h) == (0, 0) {
            "none"
        } else {
            match c.siting {
                ChromaSiting::Left => "left",
                ChromaSiting::Center => "center",
                other => panic!("{other:?}"),
            }
        }
        .into(),
        alpha: match c.alpha {
            AlphaMode::Opaque => "none",
            AlphaMode::Straight => "straight",
            AlphaMode::Premultiplied => "premultiplied",
            other => panic!("{other:?}"),
        }
        .into(),
    };
    assert_eq!(probed, case.reading, "{}: the probe's reading", case.id);
    let reading = resolve_video(v, &Interpretation::default());
    let timing = Timing::new(&src, &reading, pm_time::Rate::FPS_30);
    let ticks = (case.time * pm_time::TICKS_PER_SECOND as f64).round() as i64;
    let pick = timing.pick(&src, ticks).unwrap();
    let service = DecodeService::new(ServiceConfig::default());
    service
        .session(&src, 1, DecodePurpose::Export)
        .unwrap()
        .frame(pick.frame)
        .unwrap()
}

/// Raw integer codes of a decoded frame: luma, chroma (its own size) and
/// alpha (0..1), read straight from memory per layout.
struct Codes {
    y: Vec<f64>,
    cb: Vec<f64>,
    cr: Vec<f64>,
    alpha: Option<Vec<f64>>,
    chroma: (usize, usize),
    factors: (usize, usize),
    bits: u32,
    full: bool,
}

fn codes(frame: &DecodedFrame) -> Codes {
    let f = &frame.format;
    let (w, h) = (f.width, f.height);
    let factors = (1usize << f.log2_chroma_w, 1usize << f.log2_chroma_h);
    let chroma = (w.div_ceil(factors.0), h.div_ceil(factors.1));
    let shift = match f.alignment {
        SampleAlignment::MsbIn16 => 16 - u32::from(f.bits),
        _ => 0,
    };
    frame
        .with_planes(|p| {
            // Sample `c` of texel x in row y of a plane of `n`-byte samples.
            let at = |plane: usize, x: usize, y: usize, per: usize, c: usize, n: usize| unsafe {
                let ptr = p[plane].ptr.add(y * p[plane].stride + (x * per + c) * n);
                if n == 1 {
                    f64::from(*ptr)
                } else {
                    f64::from(u16::from_le_bytes([*ptr, *ptr.add(1)]) >> shift)
                }
            };
            let n = if f.bits > 8 { 2 } else { 1 };
            let amax = f64::from((1u32 << f.alpha_bits) - 1).max(1.0);
            let (mut y, mut cb, mut cr) = (
                vec![0.0; w * h],
                vec![0.0; chroma.0 * chroma.1],
                vec![0.0; chroma.0 * chroma.1],
            );
            let mut alpha = f.has_alpha().then(|| vec![0.0; w * h]);
            for j in 0..h {
                for i in 0..w {
                    let k = j * w + i;
                    match f.layout {
                        Layout::Planar | Layout::Biplanar | Layout::BiplanarAlpha => {
                            y[k] = at(0, i, j, 1, 0, n)
                        }
                        Layout::Cbycry16 => y[k] = at(0, i / 2, j, 4, 1 + 2 * (i & 1), 2),
                        Layout::Ayuv16 => y[k] = at(0, i, j, 4, 1, 2),
                        other => panic!("{other:?}"),
                    }
                    if let Some(a) = &mut alpha {
                        a[k] = match f.layout {
                            Layout::Planar => {
                                at(3, i, j, 1, 0, if f.alpha_bits > 8 { 2 } else { 1 })
                            }
                            Layout::BiplanarAlpha => at(2, i, j, 1, 0, 1),
                            Layout::Ayuv16 => at(0, i, j, 4, 0, 2),
                            other => panic!("{other:?} alpha"),
                        } / amax;
                    }
                }
            }
            for j in 0..chroma.1 {
                for i in 0..chroma.0 {
                    let (b, r) = match f.layout {
                        Layout::Planar => (at(1, i, j, 1, 0, n), at(2, i, j, 1, 0, n)),
                        Layout::Biplanar | Layout::BiplanarAlpha => {
                            (at(1, i, j, 2, 0, n), at(1, i, j, 2, 1, n))
                        }
                        Layout::Cbycry16 => (at(0, i, j, 4, 0, 2), at(0, i, j, 4, 2, 2)),
                        Layout::Ayuv16 => (at(0, i, j, 4, 2, 2), at(0, i, j, 4, 3, 2)),
                        other => panic!("{other:?}"),
                    };
                    cb[j * chroma.0 + i] = b;
                    cr[j * chroma.0 + i] = r;
                }
            }
            Codes {
                y,
                cb,
                cr,
                alpha,
                chroma,
                factors,
                bits: u32::from(f.bits),
                full: f.range == Range::Full,
            }
        })
        .unwrap()
}

/// Premultiplied Standard working values in display codes (unclamped), and
/// whether each pixel's chroma is constant across the reconstruction
/// footprint.
fn reference(r: &Reading, frame: &DecodedFrame) -> (Vec<[f64; 4]>, Vec<bool>) {
    use standards::*;
    let c = codes(frame);
    let (w, h) = (frame.format.width, frame.format.height);
    let (fx, fy) = c.factors;
    // Where chroma sample 0 sits on the luma grid (H.273 chroma location
    // 0, "left": co-sited across, between rows for 4:2:0).
    let origin = match (r.siting.as_str(), fy) {
        ("none", _) => (0.0, 0.0),
        ("left", 1) => (0.0, 0.0),
        ("left", _) => (0.0, (fy as f64 - 1.0) / 2.0),
        ("center", _) => ((fx as f64 - 1.0) / 2.0, (fy as f64 - 1.0) / 2.0),
        (other, _) => panic!("siting {other}"),
    };
    let norm = |p: &[f64]| -> Vec<f64> {
        p.iter()
            .map(|&v| normalize(v, c.bits, c.full, true))
            .collect()
    };
    let (cb, cr) = (norm(&c.cb), norm(&c.cr));
    let cb = reconstruct(&cb, c.chroma, (w, h), c.factors, origin);
    let cr = reconstruct(&cr, c.chroma, (w, h), c.factors, origin);
    let k = kr_kb(&r.matrix);
    let to709 = to_709(&r.primaries);
    let at = |p: &[f64], i: i64, j: i64| {
        p[(j.clamp(0, c.chroma.1 as i64 - 1) as usize) * c.chroma.0
            + i.clamp(0, c.chroma.0 as i64 - 1) as usize]
    };
    let mut out = Vec::with_capacity(w * h);
    let mut flat = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let (i0, j0) = (
                ((x as f64 - origin.0) / fx as f64).floor() as i64,
                ((y as f64 - origin.1) / fy as f64).floor() as i64,
            );
            let (span_x, span_y) = (if fx > 1 { 3 } else { 0 }, if fy > 1 { 3 } else { 0 });
            flat.push((j0 - span_y..=j0 + span_y).all(|j| {
                (i0 - span_x..=i0 + span_x).all(|ii| {
                    at(&c.cb, ii, j) == at(&c.cb, i0, j0) && at(&c.cr, ii, j) == at(&c.cr, i0, j0)
                })
            }));
            let luma = normalize(c.y[i], c.bits, c.full, false);
            let alpha = c.alpha.as_ref().map_or(1.0, |a| a[i]);
            let rgb = ycbcr_to_rgb(luma, cb[i], cr[i], k);
            // Colour is converted straight: a file tagged premultiplied is
            // divided by alpha first, except below section 9's 1/4096
            // floor, where it is kept as stored.
            let premultiplied = r.alpha == "premultiplied";
            if premultiplied && alpha < 1.0 / 4096.0 {
                out.push([
                    rgb[0] * 255.0,
                    rgb[1] * 255.0,
                    rgb[2] * 255.0,
                    alpha * 255.0,
                ]);
                continue;
            }
            let rgb = if premultiplied {
                rgb.map(|v| v / alpha)
            } else {
                rgb
            };
            let rgb = match r.transfer.as_str() {
                "srgb" if r.primaries == "bt709" => rgb,
                "srgb" => apply(&to709, rgb.map(srgb_decode)).map(srgb_encode),
                hdr => {
                    // PQ/HLG are defined on 0..1. No fixture carries
                    // mastering metadata: a 1000 cd/m² content peak.
                    let s = rgb.map(|v| v.clamp(0.0, 1.0));
                    let nits = if hdr == "pq" {
                        s.map(pq_eotf)
                    } else {
                        hlg_display(s)
                    };
                    let sdr = nits.map(|n| pq_eotf(eetf(pq_signal(n), 1000.0, 203.0)) / 203.0);
                    apply(&to709, sdr).map(srgb_encode)
                }
            };
            let k = alpha * 255.0;
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
    assert!(
        frame.failures.is_empty(),
        "{}: {:?}",
        case.id,
        frame.failures
    );
    let rgba = renderer.read_rgba(&frame).unwrap();
    renderer.recycle(frame);
    rgba
}

/// The engine's stated distance bound from the f64 reference at a
/// project depth, in display codes. 8-bit output rounds once (half a
/// code), after half-float source storage for deep and HDR sources (half
/// an f16 step below 1.0, 0.125 codes). Half float rounds twice (source
/// and output, 2⁻¹² each). The GPU's f32 transcendentals in the PQ curve
/// (powers of 78.8 and 6.3, round trip through the EETF) add up to 0.15.
fn bound(depth: u32) -> f64 {
    match depth {
        8 => 0.625 + 1e-3,
        16 => 0.125 + 0.15,
        _ => 0.15,
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
        let frame = decoded(&case);
        let (w, h) = (frame.format.width as u32, frame.format.height as u32);
        let (reference, flat) = reference(&case.reading, &frame);
        assert!(
            flat.iter().filter(|&&f| f).count() > 1000,
            "{}: flat pixels",
            case.id
        );
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
            // How far the correction moves today's paused composition
            // (information for the quality ledger, not a gate).
            let shift = case.today.first().map(|(_, bytes)| {
                let today: Vec<[f64; 4]> = bytes
                    .chunks_exact(4)
                    .map(|p| std::array::from_fn(|c| f64::from(p[c])))
                    .collect();
                distance(
                    |p| std::array::from_fn(|c| f64::from(rgba[p * 4 + c]) * 255.0),
                    &today,
                    &flat,
                )
            });
            println!(
                "VIDEO-REFERENCE {}",
                json!({"case": case.id, "depth": depth, "engine": engine, "shiftFromToday": shift,
                    "today": today.iter().map(|(k, d)| json!({"reading": k, "distance": d}))
                        .collect::<Vec<_>>(),
                    "flatPixels": flat.iter().filter(|&&f| f).count(), "pixels": flat.len()})
            );
            if engine.flat_max > bound(depth) {
                failures.push(format!(
                    "{} {depth}-bit: flat max {:.4} codes over the {:.3} bound",
                    case.id,
                    engine.flat_max,
                    bound(depth)
                ));
            }
            for (kind, t) in &today {
                if engine.mean > t.mean + TIE || engine.flat_max > t.flat_max + TIE {
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
    // A renderer per surface: each decodes and converts the frame itself,
    // so no surface is answered from another's converted-frame cache.
    let fresh = || Renderer::headless().unwrap();
    for case in cases() {
        let frame = decoded(&case);
        let size = (frame.format.width as u32, frame.format.height as u32);
        drop(frame);
        for depth in [8, 16, 32] {
            let export = render(&mut fresh(), &case, size, depth, true, Sampling::Final);
            let paused = render(&mut fresh(), &case, size, depth, false, Sampling::Final);
            let playing = render(&mut fresh(), &case, size, depth, false, Sampling::Playback);
            assert_eq!(paused, export, "{} {depth}-bit: paused vs export", case.id);
            assert_eq!(
                playing, export,
                "{} {depth}-bit: playing vs export",
                case.id
            );
        }
    }
}

/// Superwhites and sub-blacks survive every reading into effects: on an
/// untagged SD clip (BT.601, SMPTE 170M primaries converted) exactly as on
/// a Rec.709 HD clip (no conversion), the layer keeps the 8-code band below
/// black and the 250-code band above white, and an exposure pull of one
/// stop recovers the superwhite band (the effect's own formula then clamps
/// below 0). Bands are Y′ 8, 16, 128, 235, 250 with neutral chroma,
/// lossless.
#[test]
fn superwhites_reach_effects_the_same_way_for_every_reading() {
    let mut renderer = Renderer::headless().unwrap();
    assert!(
        renderer
            .set_definitions(&pm_render::builtin_definitions())
            .unwrap()
            .is_empty()
    );
    let bands = [8.0f64, 16.0, 128.0, 235.0, 250.0];
    let mut seen = Vec::new();
    for (clip, (w, h)) in [
        ("superwhite-sd.mp4", (256u32, 144u32)),
        ("superwhite-hd.mp4", (1280, 720)),
    ] {
        for exposure in [0.0f64, -1.0] {
            let dur = 1.0;
            let mut video = doc::layer(
                "v",
                "video",
                json!({"asset":"clip","fit":"stretch","w":w,"h":h,"trim":0,"speed":1}),
                f64::from(w) / 2.0,
                f64::from(h) / 2.0,
                dur,
            );
            if exposure != 0.0 {
                video["fx"] = json!([{ "id": "e", "type": "color", "on": true,
                "p": { "exposure": doc::ch(json!(exposure)) } }]);
            }
            let mut p = doc::project(w, h, dur, doc::no_fill(), vec![video]);
            doc::asset(&mut p, "clip", "video", "mp4", w, h, dur);
            p["colorDepth"] = json!(32);
            let mut assets = MapResolver::default();
            assets.paths.insert("clip".into(), fixtures().join(clip));
            let request = RenderRequest {
                export: true,
                video_end: VideoEnd::Final,
                ..RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32)
            };
            let frame = renderer
                .render(&doc::load(&p), "Ptest-root", Ticks::ZERO, &request, &assets)
                .unwrap();
            assert!(frame.failures.is_empty(), "{clip}: {:?}", frame.failures);
            let rgba = renderer.read_rgba(&frame).unwrap();
            let values: Vec<f64> = (0..bands.len())
                .map(|b| {
                    let y = (b as u32 * h + h / 2) / bands.len() as u32;
                    f64::from(rgba[((y * w + w / 2) * 4) as usize])
                })
                .collect();
            for (v, code) in values.iter().zip(bands) {
                // Grey keeps its value through any D65 primaries conversion; the
                // exposure scales the extended E-encoded value.
                let mut want = exposure.exp2() * (code - 16.0) / 219.0;
                if exposure != 0.0 {
                    want = want.max(0.0);
                }
                assert!(
                    (v - want).abs() < 1e-4,
                    "{clip} exposure {exposure} band {code}: {v} != {want}"
                );
            }
            seen.push(values);
        }
    }
    // seen: SD at 0 and −1 stops, then HD at 0 and −1 stops.
    for (sd, hd) in [(&seen[0], &seen[2]), (&seen[1], &seen[3])] {
        for (a, b) in sd.iter().zip(hd) {
            assert!((a - b).abs() < 1e-5, "SD {a} vs HD {b}");
        }
    }
    assert!(
        seen[0][0] < 0.0 && seen[2][0] < 0.0,
        "the sub-black band is kept"
    );
    assert!(
        seen[1][4] > seen[1][3] + 0.03,
        "the superwhite band is recovered"
    );
}

/// The engine's colour constants against the standards: pm-color's
/// primaries conversions against matrices derived from the published
/// chromaticities (and BT.2087's published BT.2020 → BT.709 matrix), its
/// EETF against BT.2390's equations, and the range normalization against
/// H.273 at 8, 10, 12 and 16 bits.
#[test]
fn engine_colour_constants_follow_the_standards() {
    use standards::*;
    let bt2087 = [
        [1.6605, -0.5876, -0.0728],
        [-0.1246, 1.1329, -0.0083],
        [-0.0182, -0.1006, 1.1187],
    ];
    let derived = to_709("bt2020");
    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (derived[i][j] - bt2087[i][j]).abs() < 1e-4,
                "BT.2087 [{i}][{j}]"
            );
        }
    }
    for (name, p) in [
        ("bt709", Primaries::Bt709),
        ("smpte170m", Primaries::Bt601_525),
        ("bt470bg", Primaries::Bt601_625),
        ("bt2020", Primaries::Bt2020),
        ("p3", Primaries::DisplayP3),
    ] {
        let ours = pm_color::primaries::conversion(p, Primaries::Bt709).unwrap();
        let want = to_709(name);
        for i in 0..3 {
            for j in 0..3 {
                assert!((ours[i][j] - want[i][j]).abs() < 1e-6, "{name} [{i}][{j}]");
            }
        }
    }
    // BT.2390: identity below the knee, the source peak onto the target.
    let (pw, pt) = (pq_signal(1000.0), pq_signal(203.0));
    assert!((eetf(pw, 1000.0, 203.0) - pt).abs() < 1e-12);
    assert_eq!(eetf(0.3, 1000.0, 203.0), 0.3);
    for i in 0..=1000 {
        let e = f64::from(i) / 1000.0;
        let ours = pm_color::tonemap::eetf(e, 1000.0);
        assert!((ours - eetf(e, 1000.0, 203.0)).abs() < 1e-12, "EETF at {e}");
    }
    for bits in [8u8, 10, 12, 16] {
        for range in [Range::Limited, Range::Full] {
            let alignment = if bits == 16 {
                SampleAlignment::Full16
            } else {
                SampleAlignment::Lsb
            };
            let n = pm_protocol::color::normalization(bits, alignment, range);
            let full = range == Range::Full;
            for code in [0.0, 16.0, 64.0, 128.0, 235.0, 240.0, 255.0]
                .map(|c| c * f64::from(1u32 << (bits - 8)))
            {
                let y = (code - n.luma_offset) / n.luma_scale;
                let c = (code - n.chroma_offset) / n.chroma_scale;
                assert!(
                    (y - normalize(code, u32::from(bits), full, false)).abs() < 1e-12,
                    "{bits} {range:?} Y′ {code}"
                );
                assert!(
                    (c - normalize(code, u32::from(bits), full, true)).abs() < 1e-12,
                    "{bits} {range:?} C {code}"
                );
            }
        }
    }
}
