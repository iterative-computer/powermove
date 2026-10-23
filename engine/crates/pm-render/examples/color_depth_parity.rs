//! Compare a fresh app capture; fails on any RGBA8 byte difference. All
//! reference, result and difference images stay in the supplied external dir.
#[path = "../tests/support/mod.rs"]
mod support;

use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

fn export_depth8(
    r: &mut Renderer,
    doc: Arc<pm_model::Document>,
    comp: &str,
    sample: &serde_json::Value,
    destination: PathBuf,
    reference: &[u8],
) -> usize {
    use pm_export::render::{
        ExportJob, MediaPaths, Unrestricted, plan_export_for, run_export_into, sink_for,
    };
    use pm_protocol::{
        color::OutputColor,
        common::{FrameRate, TICKS_PER_SECOND, TickRange, Ticks},
        priority::{AlphaOutput, DeliveryOptions, ExportFormat, ExportSettings, QualityTier},
    };
    let start = support::ticks(sample["time"].as_f64().unwrap());
    let job = ExportJob {
        document: doc,
        composition: comp.to_owned(),
        settings: ExportSettings {
            format: ExportFormat::PngStill { sixteen_bit: false },
            alpha: if sample["exportReference"].is_string() {
                AlphaOutput::Straight
            } else {
                AlphaOutput::None
            },
            quality: QualityTier::Max,
            size: Size::new(
                sample["width"].as_u64().unwrap() as u32,
                sample["height"].as_u64().unwrap() as u32,
            ),
            frame_rate: FrameRate::new(30, 1),
            range: TickRange {
                start,
                end: Ticks::new(start.get() + TICKS_PER_SECOND / 30).unwrap(),
            },
            audio: false,
            motion_blur: false,
            color: OutputColor::SrgbImage,
            precision: Precision::Float32,
            dither: true,
            draft_from_proxies: false,
            delivery: DeliveryOptions::default(),
        },
        media: Arc::new(MediaPaths::default()),
        destination: destination.clone(),
        overwrite: pm_export::Overwrite::Replace,
        mode: Some(0o600),
        seed: 17,
    };
    let plan = plan_export_for(&job, false, r).unwrap();
    let sink = sink_for(&plan).unwrap();
    run_export_into(
        r,
        &job,
        plan,
        sink,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let (info, decoded) = pm_export::png::read(&destination, true).unwrap();
    assert_eq!(info.bit_depth, 8);
    let decoded = decoded.unwrap();
    let actual = decoded.planes[0].u8().unwrap();
    assert_eq!(actual.len(), reference.len());
    let exported_reference;
    let reference = if let Some(name) = sample["exportReference"].as_str() {
        let (_, decoded) =
            pm_export::png::read(&destination.parent().unwrap().join(name), true).unwrap();
        exported_reference = decoded.unwrap();
        exported_reference.planes[0].u8().unwrap()
    } else {
        // Opaque PNG pixels coincide with the premultiplied app readback.
        assert!(reference.chunks_exact(4).all(|p| p[3] == 255));
        reference
    };
    actual
        .chunks_exact(4)
        .zip(reference.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

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
        let doc = Arc::new(pm_model::Document::new(
            Some(support::read_json(&dir.join(format!("{id}.json")))),
            1,
        ));
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
        let export_different = if s["depth"] == 8 {
            assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba8Unorm);
            let different = export_depth8(
                &mut r,
                Arc::clone(&doc),
                &comp,
                s,
                dir.join(format!("{id}.export.png")),
                &ref_bytes,
            );
            println!("{id} exported PNG: different={different}");
            Some(different)
        } else {
            None
        };
        let max_cv = pixels
            .iter()
            .zip(&reference)
            .map(|(a, b)| f64::from((a - b).abs()) * 255.0)
            .fold(0.0, f64::max);
        // Pixel-exact is the DEPTH-1 bar, with no corpus exceptions.
        let mut pass = different == 0 && export_different.is_none_or(|n| n == 0);
        let mut source_result = serde_json::Value::Null;
        if let Some(source) = s.get("source") {
            let source_w = source["width"].as_u64().unwrap() as usize;
            let source_h = source["height"].as_u64().unwrap() as usize;
            let raster = r
                .text_raster(
                    &doc,
                    &comp,
                    source["layer"].as_str().unwrap(),
                    s["time"].as_f64().unwrap(),
                    source["scale"].as_f64().unwrap(),
                    Precision::Float32,
                )
                .unwrap()
                .unwrap();
            assert_eq!(raster.canvas, (source_w as u32, source_h as u32));
            let mut native_source = vec![0; source_w * source_h * 4];
            let [x0, y0, x1, y1] = raster.plan.crop.map(|v| v as usize);
            for y in y0..y1 {
                for x in x0..x1 {
                    for c in 0..4 {
                        native_source[(y * source_w + x) * 4 + c] =
                            to_byte(raster.pixels[((y - y0) * (x1 - x0) + x - x0) * 4 + c]);
                    }
                }
            }
            let reference_source = std::fs::read(dir.join(format!("{id}.source.rgba"))).unwrap();
            assert_eq!(native_source.len(), reference_source.len());
            let source_different = native_source
                .chunks_exact(4)
                .zip(reference_source.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            let source_max = native_source
                .iter()
                .zip(&reference_source)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            support::write_png(
                &dir.join(format!("{id}.source.native.png")),
                source_w as u32,
                source_h as u32,
                &native_source,
            );
            support::write_png(
                &dir.join(format!("{id}.source.reference.png")),
                source_w as u32,
                source_h as u32,
                &reference_source,
            );
            println!("{id} source: different={source_different}, max={source_max} CV");
            pass &= source_different == 0;
            source_result = json!({"differentPixels":source_different,"maxCv":source_max});
        }
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
        failures += usize::from(!pass);
        results.push(json!({"id":id,"depth":s["depth"],"pass":pass,"differentPixels":different,"exportDifferentPixels":export_different,"maxCv":max_cv,"source":source_result,"p50Ms":timings[6],"targetPoolBytes":r.target_pool_bytes(),"outputBytes":frame.color.bytes()}));
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
