//! Section 2's linear-blending bar through the renderer: computed at 32-bit,
//! round half up, sRGB output, 50% white over black is 188 (0.735357;
//! boundary 187.5), and a 1-pixel checkerboard downscaled 2× is 188. With
//! linear blending off both are today's 128. (The third line, a Gaussian
//! blur of a white line conserving energy, needs the blur effect; see
//! docs/engine/composite.md.)

mod support;

use pm_color::transfer::unit_to_code;
use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

fn with_linear(mut doc: Value, on: bool) -> Value {
    doc["color"] = json!({ "mode": "standard", "linearBlending": on });
    doc
}

fn render(r: &mut Renderer, doc: &Value, size: (u32, u32), resolver: &MapResolver) -> Vec<f32> {
    let request = RenderRequest::at_scale(Size::new(size.0, size.1), 1.0, Precision::Float32);
    let frame = r
        .render(&load(doc), "Ptest-root", ticks(0.0), &request, resolver)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    r.read_rgba(&frame).unwrap()
}

#[test]
fn half_white_over_black_is_188() {
    let mut r = Renderer::headless().unwrap();
    for (on, code) in [(true, 188), (false, 128)] {
        let mut white = layer(
            "w",
            "solid",
            json!({ "color": "#FFFFFF", "w": 8, "h": 8 }),
            0.0,
            0.0,
            1.0,
        );
        set(&mut white, "opacity", json!(50));
        let doc = with_linear(project(8, 8, 1.0, solid_fill("#000000"), vec![white]), on);
        let rgba = render(&mut r, &doc, (8, 8), &MapResolver::default());
        for px in rgba.chunks_exact(4) {
            for c in &px[..3] {
                assert_eq!(unit_to_code(f64::from(*c), 8), code, "linear {on}: {c}");
            }
        }
        if on {
            assert!((f64::from(rgba[0]) - 0.735357).abs() < 1e-5, "{}", rgba[0]);
        }
    }
}

/// A 64 × 64 one-pixel black and white checkerboard (8-bit RGB PNG).
fn checkerboard(path: &std::path::Path) {
    let n = 64u32;
    let mut data = Vec::with_capacity((n * n * 3) as usize);
    for y in 0..n {
        for x in 0..n {
            let v = if (x + y) % 2 == 0 { 255 } else { 0 };
            data.extend_from_slice(&[v, v, v]);
        }
    }
    let file = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), n, n);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&data).unwrap();
}

#[test]
fn checkerboard_downscaled_2x_is_188() {
    let dir = std::env::temp_dir().join(format!("pm-render-linear-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("checker.png");
    checkerboard(&path);
    let mut r = Renderer::headless().unwrap();
    let mut resolver = MapResolver::default();
    resolver.paths.insert("checker".into(), path.clone());
    for (on, code) in [(true, 188), (false, 128)] {
        // The 64-pixel image centered at (16, 16) at 50%: each output pixel
        // center lands on a texel corner, so bilinear sampling averages
        // two white and two black texels.
        let mut image = layer(
            "img",
            "image",
            json!({ "asset": "checker", "fit": "stretch", "w": 64, "h": 64 }),
            16.0,
            16.0,
            1.0,
        );
        set(&mut image, "scale.x", json!(50));
        set(&mut image, "scale.y", json!(50));
        let mut doc = project(32, 32, 1.0, solid_fill("#000000"), vec![image]);
        asset(&mut doc, "checker", "image", "png", 64, 64, 1.0);
        let doc = with_linear(doc, on);
        let rgba = render(&mut r, &doc, (32, 32), &resolver);
        for (i, px) in rgba.chunks_exact(4).enumerate() {
            for c in &px[..3] {
                assert_eq!(
                    unit_to_code(f64::from(*c), 8),
                    code,
                    "linear {on}: pixel {i} = {c}"
                );
            }
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// One renderer drawing the same still for a Standard and a linear-light
/// project keeps each project's converted texture (working values differ).
#[test]
fn converted_media_follow_the_working_encoding() {
    let dir = std::env::temp_dir().join(format!("pm-render-linear-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("checker.png");
    checkerboard(&path);
    let mut r = Renderer::headless().unwrap();
    let mut resolver = MapResolver::default();
    resolver.paths.insert("checker".into(), path.clone());
    let doc = |on: bool| {
        let mut image = layer(
            "img",
            "image",
            json!({ "asset": "checker", "fit": "stretch", "w": 64, "h": 64 }),
            16.0,
            16.0,
            1.0,
        );
        set(&mut image, "scale.x", json!(50));
        set(&mut image, "scale.y", json!(50));
        let mut d = project(32, 32, 1.0, solid_fill("#000000"), vec![image]);
        asset(&mut d, "checker", "image", "png", 64, 64, 1.0);
        with_linear(d, on)
    };
    for (on, code) in [(false, 128), (true, 188), (false, 128), (true, 188)] {
        let rgba = render(&mut r, &doc(on), (32, 32), &resolver);
        assert_eq!(unit_to_code(f64::from(rgba[0]), 8), code, "linear {on}");
    }
    std::fs::remove_dir_all(&dir).ok();
}
