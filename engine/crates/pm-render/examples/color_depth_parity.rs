//! Compare a fresh app capture; fails on any RGBA8 byte difference. All
//! reference, result and difference images stay in the supplied external dir.
#[path = "../tests/support/mod.rs"]
mod support;

use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use std::{path::PathBuf, time::Instant};

fn main() {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("external capture directory"),
    );
    assert!(
        !dir.canonicalize()
            .unwrap()
            .starts_with(support::repo_root())
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let mut r = Renderer::headless().unwrap();
    assert!(
        r.set_definitions(&pm_render::all_builtin_definitions())
            .unwrap()
            .is_empty()
    );
    #[cfg(target_os = "macos")]
    r.set_text_fonts(Some(Box::new(support::text::MainThreadFonts(
        pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new()),
        None,
    ))));
    let mut results = Vec::new();
    let mut failures = 0;
    for s in manifest["samples"].as_array().unwrap() {
        r.trim(0);
        let id = s["id"].as_str().unwrap();
        let (w, h) = (
            s["width"].as_u64().unwrap() as u32,
            s["height"].as_u64().unwrap() as u32,
        );
        let doc =
            pm_model::Document::new(Some(support::read_json(&dir.join(format!("{id}.json")))), 1);
        let comp = pm_eval::node::string(pm_eval::node::member(doc.root(), "compId"))
            .unwrap()
            .to_string_lossy();
        let req = RenderRequest::sized(Size::new(w, h), Precision::Float32);
        let frame = r
            .render(
                &doc,
                &comp,
                support::ticks(s["time"].as_f64().unwrap()),
                &req,
                &MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{id}: {:?}", frame.failures);
        let pixels = r.read_rgba(&frame).unwrap();
        let bytes = std::fs::read(dir.join(format!("{id}.rgba32f"))).unwrap();
        let reference: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(reference.len(), pixels.len());
        let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let native: Vec<u8> = pixels.iter().map(|&v| to_byte(v)).collect();
        let ref_bytes: Vec<u8> = reference.iter().map(|&v| to_byte(v)).collect();
        let different = native
            .chunks_exact(4)
            .zip(ref_bytes.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        let max_cv = pixels
            .iter()
            .zip(&reference)
            .map(|(a, b)| f64::from((a - b).abs()) * 255.0)
            .fold(0.0, f64::max);
        // Pixel-exact is the DEPTH-1 bar, with no corpus exceptions.
        let pass = different == 0;
        failures += usize::from(!pass);
        support::write_png(&dir.join(format!("{id}.native.png")), w, h, &native);
        let mut diff: Vec<u8> = native
            .iter()
            .zip(&ref_bytes)
            .map(|(a, b)| a.abs_diff(*b).saturating_mul(32))
            .collect();
        for pixel in diff.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        support::write_png(&dir.join(format!("{id}.diff.png")), w, h, &diff);
        let mut timings = Vec::new();
        for _ in 0..12 {
            let started = Instant::now();
            let frame = r
                .render(
                    &doc,
                    &comp,
                    support::ticks(0.5),
                    &req,
                    &MapResolver::default(),
                )
                .unwrap();
            r.device()
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            timings.push(started.elapsed().as_secs_f64() * 1000.0);
            drop(frame);
        }
        timings.sort_by(f64::total_cmp);
        println!(
            "{id}: different={different}, max={max_cv:.6} CV, p50={:.3} ms, pool={} B",
            timings[6],
            r.target_pool_bytes()
        );
        results.push(json!({"id":id,"depth":s["depth"],"pass":pass,"differentPixels":different,"maxCv":max_cv,"p50Ms":timings[6],"targetPoolBytes":r.target_pool_bytes(),"outputBytes":frame.color.bytes()}));
    }
    std::fs::write(
        dir.join("parity.json"),
        serde_json::to_vec_pretty(&json!({"failures":failures,"samples":results})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        failures,
        0,
        "Color depth parity failures; images and parity.json are in {}",
        dir.display()
    );
}
