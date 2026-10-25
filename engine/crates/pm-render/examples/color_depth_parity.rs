//! Project-depth quality gate: authored values and high-precision reference
//! closeness, plus the same-size preview/export contract. Evidence stays in
//! the supplied external directory. Text source placement has its own reference.
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

fn export_png(
    r: &mut Renderer,
    doc: Arc<pm_model::Document>,
    comp: &str,
    sample: &serde_json::Value,
    destination: PathBuf,
    working: &[f32],
    media: &MapResolver,
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
            audio_index_playhead: None,
            motion_blur: false,
            color: OutputColor::SrgbImage,
            precision: Precision::Float32,
            dither: true,
            draft_from_proxies: false,
            delivery: DeliveryOptions::default(),
        },
        media: Arc::new(MediaPaths {
            paths: media.paths.clone(),
        }),
        destination: destination.clone(),
        overwrite: pm_export::Overwrite::Replace,
        mode: Some(0o600),
        seed: 17,
    };
    let plan = plan_export_for(&job, r).unwrap();
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
    assert_eq!(actual.len(), working.len());
    let straight = sample["exportReference"].is_string();
    // PNG is straight alpha; compare its API conversion with f64 arithmetic
    // on the exact working frame. Today's early 8-bit unpremultiplication
    // is not a reference. RGB allows only quantization plus half-code dither;
    // alpha is never dithered and its quantized code must be exact.
    actual
        .chunks_exact(4)
        .zip(working.chunks_exact(4))
        .filter(|(actual, pixel)| {
            let alpha = if straight {
                f64::from(pixel[3]).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let alpha_code = (alpha * 255.0).round() as u8;
            if actual[3] != alpha_code {
                return true;
            }
            (0..3).any(|channel| {
                if !straight && sample["depth"] == 8 {
                    return actual[channel]
                        != (pixel[channel].clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                let color = if straight {
                    if alpha < 1.0 / 4096.0 {
                        0.0
                    } else {
                        f64::from(pixel[channel]) / alpha
                    }
                } else {
                    f64::from(pixel[channel])
                };
                (f64::from(actual[channel]) - color.clamp(0.0, 1.0) * 255.0).abs() > 1.00001
            })
        })
        .count()
}

/// QUALITY-1's reference for a text sample: the uploaded canvas (`source`,
/// premultiplied RGBA8, top-down; the native raster must equal it) sampled
/// bilinearly in f64 at every output pixel centre through the layer's quad
/// (texel centres at i + 0.5, nothing outside the canvas, as the canvas
/// upload samples it), times the layer's opacity, over the same project's
/// background (the gradient capture at this depth). Returns RGBA per pixel
/// and whether the text reaches the pixel (any tap with ink).
fn text_reference(
    doc: &pm_model::Document,
    comp: &str,
    raster: &pm_render::TextRaster,
    source_layer: &str,
    time: f64,
    source: &[u8],
    background: &[f64],
    (w, h): (u32, u32),
) -> (Vec<f64>, Vec<bool>) {
    use pm_eval::transform::{apply, invert};
    let c = pm_eval::Comp::resolve(doc, comp).unwrap();
    let mut ev = pm_eval::Evaluator::new();
    let mut scope = pm_eval::Scope::new(&mut ev, &c, &c);
    let node = scope
        .layers()
        .iter()
        .find(|l| {
            pm_eval::node::string(pm_eval::node::member(Some(l), "id"))
                .is_some_and(|id| id.to_string_lossy() == source_layer)
        })
        .unwrap();
    let world = scope.world_matrix(node, time).unwrap();
    let opacity = scope.world_opacity(node, time).unwrap();
    let inverse = invert(&world).unwrap();
    let (cw, ch) = (raster.canvas.0 as i64, raster.canvas.1 as i64);
    let [x0, y0, x1, y1] = raster.plan.crop.map(f64::from);
    // Layer px -> canvas px: the crop spans the quad's box.
    let (sx, sy) = ((x1 - x0) / raster.box_size.0, (y1 - y0) / raster.box_size.1);
    let texel = |x: i64, y: i64| -> [f64; 4] {
        if x < 0 || y < 0 || x >= cw || y >= ch {
            return [0.0; 4];
        }
        let i = ((y * cw + x) * 4) as usize;
        std::array::from_fn(|k| f64::from(source[i + k]) / 255.0)
    };
    let mut out = vec![0.0; (w * h * 4) as usize];
    let mut reached = vec![false; (w * h) as usize];
    for py in 0..h {
        for px in 0..w {
            let (lx, ly) = apply(&inverse, f64::from(px) + 0.5, f64::from(py) + 0.5);
            let u = x0 + (lx + raster.anchor.0) * sx - 0.5;
            let v = y0 + (ly + raster.anchor.1) * sy - 0.5;
            let (iu, iv) = (u.floor(), v.floor());
            let (fu, fv) = (u - iu, v - iv);
            let (iu, iv) = (iu as i64, iv as i64);
            let taps = [
                (texel(iu, iv), (1.0 - fu) * (1.0 - fv)),
                (texel(iu + 1, iv), fu * (1.0 - fv)),
                (texel(iu, iv + 1), (1.0 - fu) * fv),
                (texel(iu + 1, iv + 1), fu * fv),
            ];
            let n = (py * w + px) as usize;
            reached[n] = taps.iter().any(|(t, _)| t[3] > 0.0);
            let s: [f64; 4] =
                std::array::from_fn(|k| taps.iter().map(|(t, f)| t[k] * f).sum::<f64>() * opacity);
            for k in 0..4 {
                out[n * 4 + k] = s[k] + background[n * 4 + k] * (1.0 - s[3]);
            }
        }
    }
    (out, reached)
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
        let mut media = MapResolver::default();
        if let Some(pm_model::NodeKind::Object(assets)) = doc
            .root()
            .and_then(|root| root.get("assets"))
            .map(|n| n.kind())
        {
            for (id, record) in assets.iter() {
                if let Some(path) = pm_eval::node::string(record.get("sourcePath")) {
                    media
                        .paths
                        .insert(id.to_string_lossy(), PathBuf::from(path.to_string_lossy()));
                }
            }
        }
        let mut req = RenderRequest::sized(Size::new(w, h), Precision::Float32);
        req.background = !s["transparent"].as_bool().unwrap_or(false);
        if let Some(samples) = s["mbSamples"].as_u64().filter(|n| *n > 0) {
            req.motion_blur = pm_render::motion::MotionBlur::Samples(samples.try_into().unwrap());
        }
        let frame = r
            .render(
                &doc,
                &comp,
                support::ticks(s["time"].as_f64().unwrap()),
                &req,
                &media,
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{id}: {:?}", frame.failures);
        assert_eq!(frame.color.format(), r.device_target_format(&doc, &req));
        if s["depth"] == 8 {
            assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba8Unorm);
        }
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
        // Text sampling is judged against the f64 reference below; its
        // export must equal the engine's own preview.
        let text = s["source"]["layer"].is_string() && s["source"]["scale"].is_number();
        let export_different =
            if s["exportReference"].is_string() || (s["depth"] == 8 && s["exportPng"] != false) {
                let different = export_png(
                    &mut r,
                    Arc::clone(&doc),
                    &comp,
                    s,
                    dir.join(format!("{id}.export.png")),
                    &pixels,
                    &media,
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
        let at = support::ticks(s["time"].as_f64().unwrap());
        let ideal = support::reference::render(&doc, &media, &comp, at, w, h, req.background);
        let wide = |v: &[f32]| v.iter().map(|&c| f64::from(c)).collect::<Vec<_>>();
        let report = pm_reference::compare::compare(
            &wide(&pixels),
            &wide(&reference),
            &ideal.rgba,
            &ideal.classes(),
        );
        let bounds = if s["depth"] == 8 {
            pm_reference::compare::Bounds::EIGHT_BIT
        } else {
            pm_reference::compare::Bounds::HALF_FLOAT
        };
        let quality_failures = report.failures(&bounds);
        let (preview, export) = support::reference::preview_and_export(
            &mut r,
            &doc,
            &comp,
            at,
            Size::new(w, h),
            req.motion_blur,
            &media,
        );
        let equality = pm_reference::compare::identical(&preview, &export);
        println!("{id} reference:{report}; preview/export {equality:?}");
        let mut pass = (text || quality_failures.is_empty())
            && equality.is_ok()
            && export_different.is_none_or(|n| n == 0);
        let mut source_result = serde_json::Value::Null;
        if let Some(source) = s.get("source").filter(|_| text) {
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
            // The composed text against the f64 reference (gate-policy.md):
            // pixels within 2 px of where it samples ink are Resampled; every
            // other pixel stays byte-exact.
            let background_document = support::text::without_text(&doc);
            let background = support::reference::render(
                &background_document,
                &media,
                &comp,
                at,
                w,
                h,
                req.background,
            );
            assert_eq!(background.modelled(), 1.0, "{id}: reference background");
            let (ideal, reached) = text_reference(
                &doc,
                &comp,
                &raster,
                source["layer"].as_str().unwrap(),
                s["time"].as_f64().unwrap(),
                &reference_source,
                &background.rgba,
                (w, h),
            );
            let classes = pm_reference::compare::classify(
                &vec![false; reached.len()],
                &reached,
                w as usize,
                h as usize,
                2,
            );
            let outside = classes
                .iter()
                .enumerate()
                .filter(|(i, c)| {
                    **c != pm_reference::compare::Class::Resampled
                        && native[i * 4..i * 4 + 4] != ref_bytes[i * 4..i * 4 + 4]
                })
                .count();
            let wide = |v: &[f32]| v.iter().map(|&x| f64::from(x)).collect::<Vec<_>>();
            let report =
                pm_reference::compare::compare(&wide(&pixels), &wide(&reference), &ideal, &classes);
            let bounds = if s["depth"] == 8 {
                pm_reference::compare::Bounds::EIGHT_BIT
            } else {
                pm_reference::compare::Bounds::HALF_FLOAT
            };
            let mut why = report.failures(&bounds);
            if outside > 0 {
                why.push(format!(
                    "{outside} pixels away from the text differ from today"
                ));
            }
            println!("{id} vs the f64 reference:{report}");
            for line in &why {
                println!("{id} FAIL: {line}");
            }
            pass &= why.is_empty();
            let r = report.class(pm_reference::compare::Class::Resampled);
            source_result = json!({"differentPixels":source_different,"maxCv":source_max,
                "reference":{"outsideDifferent":outside,"failures":why,
                "engine":{"max":r.engine_vs_reference.max,"p999":r.engine_vs_reference.p999,"mean":r.engine_vs_reference.mean},
                "today":{"max":r.today_vs_reference.max,"p999":r.today_vs_reference.p999,"mean":r.today_vs_reference.mean}}});
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
        for _ in 0..s["warmFrames"].as_u64().unwrap_or(12) {
            let started = Instant::now();
            let frame = r
                .render(
                    &doc,
                    &comp,
                    support::ticks(s["time"].as_f64().unwrap()),
                    &req,
                    &media,
                )
                .unwrap();
            r.device()
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            timings.push(started.elapsed().as_secs_f64() * 1000.0);
            drop(frame);
        }
        timings.sort_by(f64::total_cmp);
        let p50 = timings.get(timings.len() / 2).copied();
        println!(
            "{id}: different={different}, max={max_cv:.6} CV, p50={p50:?} ms, pool={} B",
            r.target_pool_bytes()
        );
        failures += usize::from(!pass);
        results.push(json!({"id":id,"depth":s["depth"],"pass":pass,"differentPixels":different,"exportDifferentPixels":export_different,"maxCv":max_cv,"modelled":ideal.modelled(),"qualityFailures":quality_failures,"previewEqualsExport":equality.is_ok(),"source":source_result,"p50Ms":p50,"targetPoolBytes":r.target_pool_bytes(),"outputBytes":frame.color.bytes()}));
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
