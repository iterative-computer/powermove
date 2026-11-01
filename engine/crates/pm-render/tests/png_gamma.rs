//! Gamma-only PNG stills at compat level 2 (pm-media `png_gamma`): drawn
//! with the file's pure power curve as ImageIO reads them, never as sRGB.
//! Level 1 follows Chromium (`compat.rs`) and is checked in tests/stills.rs
//! and the app harness.

mod support;

use pm_protocol::color::{Precision, Transfer};
use pm_protocol::common::Size;
use pm_render::media::probe_interpreted_image;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use std::path::{Path, PathBuf};
use support::doc::*;

const W: u32 = 8;
const H: u32 = 2;
/// Stored 8-bit samples (each pixel gray; every other pixel half alpha).
const VALUES: [u8; 8] = [0, 16, 48, 64, 128, 160, 192, 255];

fn alpha(i: usize) -> u8 {
    if i % 2 == 1 { 128 } else { 255 }
}

fn write_png(path: &Path, sixteen: bool, gamma: Option<u32>, samples: &[[u16; 4]]) {
    let file = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), W, H);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(if sixteen {
        png::BitDepth::Sixteen
    } else {
        png::BitDepth::Eight
    });
    if let Some(g) = gamma {
        e.set_source_gamma(png::ScaledFloat::from_scaled(g));
    }
    let mut w = e.write_header().unwrap();
    let row: Vec<u8> = samples
        .iter()
        .flat_map(|px| {
            px.iter().flat_map(|&v| {
                if sixteen {
                    v.to_be_bytes().to_vec()
                } else {
                    vec![v as u8]
                }
            })
        })
        .collect();
    let data: Vec<u8> = (0..H).flat_map(|_| row.clone()).collect();
    w.write_image_data(&data).unwrap();
}

fn stored() -> Vec<[u16; 4]> {
    VALUES
        .iter()
        .enumerate()
        .map(|(i, &v)| [v.into(), v.into(), v.into(), alpha(i).into()])
        .collect()
}

fn render(renderer: &mut Renderer, path: &Path, level: u32) -> Vec<f32> {
    let mut source = layer(
        "s",
        "image",
        json!({"asset":"img","w":W,"h":H}),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        1.0,
    );
    source["compat"] = json!(level);
    let mut doc = project(W, H, 1.0, no_fill(), vec![source]);
    asset(&mut doc, "img", "image", "png", W, H, 0.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("img".into(), path.to_path_buf());
    let document = load(&doc);
    assert_eq!(
        renderer.unsupported_with_media(&document, "Ptest-root", &resolver),
        vec![],
        "{}",
        path.display()
    );
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    let frame = renderer
        .render(
            &document,
            "Ptest-root",
            support::ticks(0.0),
            &request,
            &resolver,
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer.read_rgba(&frame).unwrap()
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("pm-render-png-gamma-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// gAMA alone is a pure power (PNG Third Edition) at every level.
#[test]
fn gamma_only_png_uses_the_file_curve() {
    let d = dir();
    let mut renderer = Renderer::headless().unwrap();
    let srgb_oetf = |l: f64| {
        if l <= 0.003_130_8 {
            12.92 * l
        } else {
            1.055 * l.powf(1.0 / 2.4) - 0.055
        }
    };
    let plain = d.join("plain.png");
    write_png(&plain, false, None, &stored());
    let plain_rgba = render(&mut renderer, &plain, 1);
    for (gamma, name) in [(70_000u32, "g70"), (100_000, "g100"), (55_555, "g55555")] {
        let file_gamma = f64::from(gamma) / 100_000.0;
        let path = d.join(format!("{name}.png"));
        write_png(&path, false, Some(gamma), &stored());
        // The same linear light written as a 16-bit sRGB PNG.
        let reference: Vec<[u16; 4]> = VALUES
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let l = (f64::from(v) / 255.0).powf(1.0 / file_gamma);
                let e = (srgb_oetf(l) * 65535.0).round() as u16;
                [e, e, e, u16::from(alpha(i)) * 257]
            })
            .collect();
        let reference_path = d.join(format!("{name}-ref.png"));
        write_png(&reference_path, true, None, &reference);
        let got = render(&mut renderer, &path, 1);
        assert_eq!(
            got,
            render(&mut renderer, &path, 2),
            "{name}: levels differ"
        );
        let want = render(&mut renderer, &reference_path, 1);
        let diff = max_diff(&got, &want);
        let from_srgb = max_diff(&got, &plain_rgba);
        eprintln!("gAMA {file_gamma}: max {diff:.6} from reference, {from_srgb:.4} from sRGB");
        assert!(diff < 2e-4, "{name}: {diff}");
        assert!(from_srgb > 0.02, "{name}: the gamma was not applied");
    }
    // gAMA 0.45455 is the pure power 2.19998, not the sRGB curve: dark
    // tones differ.
    let srgb = d.join("g45455.png");
    write_png(&srgb, false, Some(45_455), &stored());
    assert!(max_diff(&render(&mut renderer, &srgb, 1), &plain_rgba) > 1e-3);
    std::fs::remove_dir_all(&d).ok();
}

/// Interpret Footage overrides keep a PNG gamma unless a transfer override
/// replaces it.
#[test]
fn interpreted_gamma_png_keeps_its_curve() {
    let d = dir();
    let path = d.join("interp.png");
    write_png(&path, false, Some(70_000), &stored());
    let mut interp = pm_media::interpret::Interpretation::default();
    assert_eq!(probe_interpreted_image(&path, &interp), Ok(()));
    interp.alpha = Some(pm_media::interpret::AlphaChoice::Straight);
    assert_eq!(probe_interpreted_image(&path, &interp), Ok(()));
    interp.transfer = Some(Transfer::Linear);
    assert_eq!(probe_interpreted_image(&path, &interp), Ok(()));
    std::fs::remove_file(&path).ok();
}

/// A gamma-only PNG map has the image layer's verdict.
#[test]
fn gamma_png_texture_maps_follow_the_image_verdict() {
    let d = dir();
    let path = d.join("map.png");
    let opaque: Vec<[u16; 4]> = VALUES
        .iter()
        .map(|&v| [v.into(), v.into(), v.into(), 255])
        .collect();
    write_png(&path, false, Some(70_000), &opaque);
    assert_eq!(pm_render::media::probe_image(&path), Ok(()));
    assert_eq!(pm_render::scene3d::textures::Textures::probe(&path), Ok(()));
    std::fs::remove_file(&path).ok();
}
