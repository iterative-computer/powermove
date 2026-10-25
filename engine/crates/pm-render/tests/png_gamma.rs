//! Gamma-only PNG stills at compat level 2 (pm-media `png_gamma`): drawn
//! with the file's pure power curve as ImageIO reads them, never as sRGB.
//! Level 1 follows Chromium (`compat.rs`) and is checked in tests/stills.rs
//! and the app harness.

mod support;

use pm_protocol::color::{Precision, Transfer};
use pm_protocol::common::Size;
use pm_render::media::{ProbeIssue, probe_interpreted_image};
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

#[test]
fn level2_gamma_only_png_uses_the_file_curve() {
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
    let plain_rgba = render(&mut renderer, &plain, 2);
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
        let got = render(&mut renderer, &path, 2);
        let want = render(&mut renderer, &reference_path, 2);
        let diff = max_diff(&got, &want);
        let from_srgb = max_diff(&got, &plain_rgba);
        eprintln!("gAMA {file_gamma}: max {diff:.6} from reference, {from_srgb:.4} from sRGB");
        assert!(diff < 2e-4, "{name}: {diff}");
        assert!(from_srgb > 0.02, "{name}: the gamma was not applied");
    }
    // ImageIO's sRGB window: gAMA 0.45455 is sRGB, as an untagged PNG.
    let srgb = d.join("g45455.png");
    write_png(&srgb, false, Some(45_455), &stored());
    assert_eq!(render(&mut renderer, &srgb, 2), plain_rgba);
    std::fs::remove_dir_all(&d).ok();
}

/// Nonempty Interpret Footage overrides must not ignore a PNG gamma;
/// they need an explicit transfer override. Empty readings keep the curve.
#[test]
fn interpreted_gamma_png_needs_a_transfer_override() {
    let d = dir();
    let path = d.join("interp.png");
    write_png(&path, false, Some(70_000), &stored());
    let mut interp = pm_media::interpret::Interpretation::default();
    assert_eq!(probe_interpreted_image(&path, &interp), Ok(()));
    interp.alpha = Some(pm_media::interpret::AlphaChoice::Straight);
    match probe_interpreted_image(&path, &interp) {
        Err(ProbeIssue::Unsupported(f)) => assert!(f.to_string().contains("gAMA"), "{f}"),
        other => panic!("{other:?}"),
    }
    interp.transfer = Some(Transfer::Linear);
    assert_eq!(probe_interpreted_image(&path, &interp), Ok(()));
    let srgb = d.join("interp-srgb.png");
    write_png(&srgb, false, Some(45_455), &stored());
    assert_eq!(probe_interpreted_image(&srgb, &Default::default()), Ok(()));
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&srgb).ok();
}

/// 3D texture maps upload the importer's bitmap, the one a level 1 image
/// layer draws: a gamma-only PNG map has the level 1 image verdict
/// (`threed_maps.rs` `map_verdicts_are_the_image_layer_verdicts`).
#[test]
fn gamma_png_texture_maps_follow_the_level1_image_verdict() {
    let d = dir();
    let path = d.join("map.png");
    let opaque: Vec<[u16; 4]> = VALUES
        .iter()
        .map(|&v| [v.into(), v.into(), v.into(), 255])
        .collect();
    write_png(&path, false, Some(70_000), &opaque);
    let pool = pm_media::pool::FramePool::new(0);
    match (
        pm_render::media::probe_image(&path),
        pm_render::scene3d::textures::decode(&path, &pool),
    ) {
        (Ok(()), Ok((w, h, rgba))) => {
            let (bitmap, _) = pm_render::media::level1_bitmap(&path, &pool).unwrap();
            assert_eq!(
                (w as usize, h as usize),
                (bitmap.frame.format.width, bitmap.frame.format.height)
            );
            assert_eq!(rgba.len(), w as usize * h as usize * 4);
        }
        (
            Err(pm_render::media::ProbeIssue::Unsupported(a)),
            Err(pm_render::scene3d::textures::MapError::Unsupported(b)),
        ) => assert_eq!(a, b),
        (layer, map) => panic!("image layer {layer:?}, map {map:?}"),
    }
    let plain = d.join("map-plain.png");
    write_png(&plain, false, None, &opaque);
    assert!(pm_render::scene3d::textures::decode(&plain, &pool).is_ok());
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&plain).ok();
}
