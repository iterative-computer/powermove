//! HDR stills are HDR sources: an HDR HEIC in PQ or HLG, and a still with a
//! gain map read as its HDR picture (BT.709 PQ at the headroom the map
//! describes), are drawn as video of that signal is. In a Standard project
//! they are tone mapped to SDR white by the source pass's default (BT.2390
//! from the content peak); in an HDR project they keep their HDR light
//! (working values are display encoded, the curve extended above SDR
//! white).
//! Interpret Footage's `gainMap: "sdrBase"` reads the SDR base instead, an
//! ordinary SDR still. Fixtures: `engine/fixtures/hdr-stills` (left half
//! 18 % grey, right half 0.8 of SDR white in the base and 3.0 in the HDR
//! picture).

mod support;

use std::path::{Path, PathBuf};

use pm_color::tonemap::{HdrSignal, to_sdr, to_working_light};
use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{RenderRequest, Renderer};
use serde_json::{Value, json};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/hdr-stills")
        .join(name)
}

/// The still drawn alone at its size in a project of `mode`, read with
/// `interpret`: RGBA of the left and right halves' middle pixels.
fn drawn(path: &Path, mode: &str, interpret: Option<Value>) -> [[f32; 4]; 2] {
    use support::doc::*;
    let (w, h) = (64u32, 36u32);
    let mut value = project(
        w,
        h,
        1.0,
        no_fill(),
        vec![layer(
            "still",
            "image",
            json!({"asset":"clip","w":w,"h":h}),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            1.0,
        )],
    );
    value["colorDepth"] = json!(32);
    value["color"] = json!({ "mode": mode });
    let ext = path.extension().unwrap().to_str().unwrap();
    asset(&mut value, "clip", "image", ext, w, h, 1.0);
    if let Some(i) = interpret {
        value["assets"]["clip"]["interpret"] = i;
    }
    let mut resolver = pm_render::MapResolver::default();
    resolver.paths.insert("clip".into(), path.to_path_buf());
    let mut request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32);
    request.background = false;
    let mut renderer = Renderer::headless().unwrap();
    let frame = renderer
        .render(
            &load(&value),
            "Ptest-root",
            support::ticks(0.0),
            &request,
            &resolver,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let rgba = renderer.read_rgba(&frame).unwrap();
    [8usize, 56].map(|x| {
        let i = (18 * w as usize + x) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    })
}

/// The still's own codes (as pm-media reads it) at the two pixels, 0…1.
fn codes(path: &Path) -> [[f64; 3]; 2] {
    let still = pm_media::image::read(path, &pm_media::pool::FramePool::new(0)).unwrap();
    let rgba = pm_media::image::imageio::rgba_unit(&still.frame).unwrap();
    [8usize, 56].map(|x| {
        let i = (18 * 64 + x) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2]].map(f64::from)
    })
}

fn close(got: [f32; 4], want: [f64; 3], tolerance: f64) -> bool {
    got[..3]
        .iter()
        .zip(want)
        .all(|(g, w)| (f64::from(*g) - w).abs() <= tolerance)
}

#[test]
fn hdr_stills_are_tone_mapped_in_a_standard_project_and_kept_in_an_hdr_one() {
    // (file, signal, content peak in cd/m²)
    let cases = [
        ("still-gainmap.heic", HdrSignal::Pq, 609.0),
        ("still-gainmap.jpg", HdrSignal::Pq, 609.0),
        ("still-gainmap.avif", HdrSignal::Pq, 609.0),
        ("still-hdr-pq.heic", HdrSignal::Pq, 1000.0),
        ("still-hdr-hlg.heic", HdrSignal::Hlg, 1000.0),
    ];
    for (name, signal, peak) in cases {
        let path = fixture(name);
        let codes = codes(&path);
        let standard = drawn(&path, "standard", None);
        let hdr = drawn(&path, "hdr", None);
        eprintln!("MEASURE {name}: standard {standard:?}, hdr {hdr:?}");
        for (i, half) in ["left", "right"].into_iter().enumerate() {
            // Standard: BT.2390 to SDR white, then BT.2020 to Rec.709 when
            // the signal is BT.2020 (HEIC), display encoded.
            let sdr = to_sdr(signal, codes[i], peak);
            let sdr = if name.contains("gainmap") {
                sdr
            } else {
                pm_color::primaries::apply(&pm_color::tonemap::bt2020_to_709(), sdr)
            };
            let want = sdr.map(pm_color::transfer::display_encode);
            assert!(
                close(standard[i], want, 1e-3),
                "{name} {half}, standard: {:?}, {want:?} expected",
                standard[i]
            );
            // HDR: the light the signal carries, SDR white 1.0.
            let light = to_working_light(signal, codes[i]);
            let light = if name.contains("gainmap") {
                light
            } else {
                pm_color::primaries::apply(&pm_color::tonemap::bt2020_to_709(), light)
            };
            let want = light.map(pm_color::transfer::display_encode);
            assert!(
                close(hdr[i], want, 1e-3),
                "{name} {half}, hdr: {:?}, {want:?} expected",
                hdr[i]
            );
        }
        // The right half is three times SDR white in an HDR project, and is
        // the content peak, tone mapped to SDR white, in a Standard one.
        if !name.contains("hlg") {
            let three = pm_color::transfer::display_encode(3.0) as f32;
            assert!((hdr[1][0] - three).abs() < 0.01, "{name}: {:?}", hdr[1]);
        }
        if name.contains("gainmap") {
            assert!(
                (standard[1][0] - 1.0).abs() < 1e-3,
                "{name}: {:?}",
                standard[1]
            );
        }
    }
}

/// `gainMap: "sdrBase"` reads the base image only: an SDR still, the same
/// in either project, never brighter than SDR white.
#[test]
fn a_gain_map_still_read_for_its_sdr_base_is_an_sdr_still() {
    for name in [
        "still-gainmap.heic",
        "still-gainmap.jpg",
        "still-gainmap.avif",
    ] {
        let path = fixture(name);
        let base = Some(json!({ "gainMap": "sdrBase" }));
        let standard = drawn(&path, "standard", base.clone());
        let hdr = drawn(&path, "hdr", base);
        eprintln!("MEASURE {name} base: standard {standard:?}, hdr {hdr:?}");
        // 0.18 and 0.8 of SDR white (8-bit base: within two codes).
        for (i, light) in [0.18f64, 0.8].into_iter().enumerate() {
            let encoded = pm_color::transfer::display_encode(light);
            assert!(
                close(standard[i], [encoded; 3], 2.0 / 255.0),
                "{name} base {i}, standard: {:?}",
                standard[i]
            );
        }
        assert_eq!(
            hdr, standard,
            "{name}: an SDR still is the same in an HDR project"
        );
        // Explicitly HDR is the default.
        let explicit = drawn(&path, "standard", Some(json!({ "gainMap": "hdr" })));
        assert_eq!(explicit, drawn(&path, "standard", None), "{name}");
    }
}
