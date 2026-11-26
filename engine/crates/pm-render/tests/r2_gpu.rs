//! R2 comparisons of native attachment bytes, including half-float zeros.
mod support;

use pm_gpu::readback::read_raw;
use pm_protocol::color::Precision;
use pm_protocol::common::{PixelRect, Size};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::{ch, layer, load, no_fill, project, solid_fill};
use support::ticks;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn renderer() -> Renderer {
    let gpu = pm_gpu::Gpu::new("R2 raw attachment differential").unwrap();
    Renderer::new(gpu.device, gpu.queue)
}

fn document(w: u32, h: u32, linear: bool, layers: Vec<Value>) -> pm_model::Document {
    load_project(project(w, h, 2.0, no_fill(), layers), linear)
}

fn load_project(mut value: Value, linear: bool) -> pm_model::Document {
    value["colorDepth"] = json!(16);
    value["color"] = json!({"mode":"standard","linearBlending":linear});
    let mut doc = load(&value);
    doc.set_saving_app_version(Some("1.3.0".into()));
    doc
}

fn render(
    r: &mut Renderer,
    doc: &pm_model::Document,
    time: f64,
    request: &RenderRequest,
) -> Vec<u8> {
    let frame = r
        .render(
            doc,
            "Ptest-root",
            ticks(time),
            request,
            &MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba16Float);
    let pixels = read_raw(r.device(), r.queue(), &frame.color.texture).unwrap();
    r.recycle(frame);
    pixels
}

fn assert_raw_equal(reference: &[u8], candidate: &[u8], width: u32, context: &str) {
    assert_eq!(reference.len(), candidate.len(), "{context}");
    for (i, (before, after)) in reference
        .chunks_exact(2)
        .zip(candidate.chunks_exact(2))
        .enumerate()
    {
        if before != after {
            let pixel = i / 4;
            panic!(
                "{context}: pixel ({}, {}), channel {}: {:#06x} vs {:#06x}",
                pixel % width as usize,
                pixel / width as usize,
                i % 4,
                u16::from_le_bytes(before.try_into().unwrap()),
                u16::from_le_bytes(after.try_into().unwrap()),
            );
        }
    }
}

fn shader(id: &str, source: &str, w: u32, h: u32, x: f64, y: f64) -> Value {
    layer(
        id,
        "shader",
        json!({"code":source,"uniforms":{},"w":w,"h":h}),
        x,
        y,
        2.0,
    )
}

fn blur(id: &str, amount: f64) -> Value {
    json!({"id": id, "type": "blur", "version": 1, "on": true,
        "p": {"amount": ch(json!(amount))}})
}

/// The reference still uses the effect scissor. Only its composite uses
/// the original full-quad sampler. Offset and overlap exercise GL rows.
#[test]
fn bounded_source_blending_matches_original_attachment_bits() {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    for linear in [false, true] {
        let (w, h) = (257, 145);
        let mut upper = layer(
            "upper",
            "solid",
            json!({"color":"#FF60B080","w":31,"h":19}),
            52.0,
            105.0,
            2.0,
        );
        upper["fx"] = json!([blur("upper-blur", 5.0)]);
        let mut lower = layer(
            "lower",
            "solid",
            json!({"color":"#6070FF80","w":57,"h":55}),
            66.0,
            84.0,
            2.0,
        );
        lower["fx"] = json!([blur("lower-blur", 7.0)]);
        let edge = layer(
            "edge",
            "solid",
            json!({"color":"#20EF80","w":21,"h":9}),
            3.0,
            141.0,
            2.0,
        );
        let doc = document(w, h, linear, vec![upper, lower, edge]);
        r.set_main_effects_for_document(&doc).unwrap();
        r.set_effect_scissor(true);
        for region in [
            None,
            Some(PixelRect {
                x: 11,
                y: 37,
                width: 93,
                height: 79,
            }),
        ] {
            let mut request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16);
            request.background = false;
            request.region = region;
            r.set_effect_output_bounds(false);
            let before = render(&mut r, &doc, 0.5, &request);
            r.set_effect_output_bounds(true);
            let after = render(&mut r, &doc, 0.5, &request);
            assert_raw_equal(
                &before,
                &after,
                region.map_or(w, |r| r.width),
                "bounded effects",
            );
        }
    }
}

#[test]
fn shader_cache_ignores_unused_time_and_preserves_animated_output() {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let (w, h) = (97, 51);
    let static_source =
        "void mainImage(out vec4 c, in vec2 p) { c = vec4(0.125, 0.5, p.y / 128.0, 1.0); }";
    let moving_source =
        "void mainImage(out vec4 c, in vec2 p) { c = vec4(iTime / 2.0, p.y / 128.0, 0.5, 1.0); }";
    let doc = document(
        w,
        h,
        false,
        vec![
            shader("static", static_source, 19, 17, 1.0, 1.0),
            shader("moving", moving_source, 19, 17, 28.0, 1.0),
            shader("second", moving_source, 19, 17, 55.0, 1.0),
        ],
    );
    let mut request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16);
    request.background = false;
    let times = [0.25, 0.5, 0.75, 0.5];
    r.set_playback_retention(0, 0);
    r.set_early_target_release(false);
    let reference: Vec<_> = times
        .map(|time| render(&mut r, &doc, time, &request))
        .into();
    assert_ne!(reference[0], reference[1], "the active uniform animates");
    r.set_playback_retention(0, 1 << 20);
    r.set_early_target_release(true);
    for (time, before) in times.into_iter().zip(reference) {
        let after = render(&mut r, &doc, time, &request);
        assert_raw_equal(&before, &after, w, "shader cache and scoped leases");
    }
    let (hits, admissions, _, _) = r.shader_output_cache_stats();
    assert!(hits >= 2, "{hits} cache hits");
    assert!(admissions >= 1);
    assert!(r.shader_output_cache_bytes() <= 1 << 20);
}

const ADVERSARIAL: &str = r#"
void mainImage(out vec4 c, in vec2 p) {
    ivec2 xy = ivec2(floor(p));
    float alternating = ((xy.x ^ xy.y) & 1) != 0 ? 12.0 : -0.75;
    float marker = xy == ivec2(3, 7) ? 7.0 : 0.0;
    float tiny = uintBitsToFloat(0x33800000u);
    float minusZero = uintBitsToFloat(0x80000000u);
    c = vec4(alternating, marker, xy.y % 2 == 0 ? tiny : minusZero, 1.0);
}
"#;

#[test]
fn a_padded_effect_keeps_the_same_bits_in_a_viewport_accumulator() {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let (w, h) = (512, 288);
    let mut front = layer(
        "front",
        "solid",
        json!({"color":"#FF60B080","w":145,"h":250}),
        4.25,
        51.75,
        2.0,
    );
    front["fx"] = json!([blur("soften", 8.0)]);
    front["group"] = json!("inner");
    let mut inner = layer("inner", "group", json!({}), 0., 0., 2.0);
    inner["group"] = json!("outer");
    inner["fx"] = json!([blur("inner-soften", 4.0)]);
    let mut outer = layer("outer", "group", json!({}), 0., 0., 2.0);
    outer["fx"] = json!([blur("outer-soften", 5.0)]);
    let back = layer(
        "back",
        "solid",
        json!({"color":"#2060EF80","w":197,"h":122}),
        170.75,
        86.25,
        2.0,
    );
    let crops = [
        PixelRect {
            x: 103,
            y: 57,
            width: 121,
            height: 87,
        },
        PixelRect {
            x: 0,
            y: 57,
            width: 121,
            height: 87,
        },
        PixelRect {
            x: 24,
            y: h - 87,
            width: 121,
            height: 87,
        },
    ];
    for checker in [false, true] {
        let mut layers = vec![front.clone(), inner.clone(), outer.clone(), back.clone()];
        if checker {
            // A checker behind the nested effects makes a displaced raster
            // origin visible even where the foreground is transparent.
            let tile = 32;
            for y in (0..h).step_by(tile as usize) {
                for x in (0..w).step_by(tile as usize) {
                    if (x / tile + y / tile) % 2 == 1 {
                        layers.push(layer(
                            &format!("tile-{x}-{y}"),
                            "solid",
                            json!({"color":"#94A0AC","w":tile,"h":tile}),
                            f64::from(x),
                            f64::from(y),
                            2.0,
                        ));
                    }
                }
            }
        }
        let doc = load_project(project(w, h, 2.0, solid_fill("#343840"), layers), true);
        r.set_main_effects_for_document(&doc).unwrap();
        let mut request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16);
        r.set_early_target_release(false);
        let full = render(&mut r, &doc, 0.5, &request);
        for crop in crops {
            const PIXEL_BYTES: usize = 8; // RGBA16F
            let left = crop.x as usize * PIXEL_BYTES;
            let right = left + crop.width as usize * PIXEL_BYTES;
            // Renderer::render returns top-down rows.
            let reference: Vec<_> = full
                .chunks_exact(w as usize * PIXEL_BYTES)
                .skip(crop.y as usize)
                .take(crop.height as usize)
                .flat_map(|row| row[left..right].iter().copied())
                .collect();
            request.region = Some(crop);
            r.set_early_target_release(true);
            let candidate = render(&mut r, &doc, 0.5, &request);
            assert_raw_equal(
                &reference,
                &candidate,
                crop.width,
                &format!("nested viewport, checker={checker}, crop={crop:?}"),
            );
        }
    }
}

/// A certification probe for the default-off wrapper elision. Run the
/// ignored test explicitly on each candidate GPU. A difference means the
/// corresponding format/device/size cannot use the direct path.
#[test]
#[ignore = "experimental direct group path requires per-device bit certification"]
fn direct_group_target_bit_identity_probe() {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    r.set_playback_retention(0, 0);
    for (w, h) in [(79, 43), (2880, 1620)] {
        for linear in [false, true] {
            let mut group = layer("group", "group", json!({}), 0.0, 0.0, 2.0);
            // Make this a layer node without changing the expected RGB
            // arithmetic (zero blur); the checker reaches the group target.
            group["fx"] = json!([blur("group-blur", 0.0)]);
            let mut member = shader("member", ADVERSARIAL, w, h, 0.0, 0.0);
            member["group"] = json!("group");
            let doc = document(w, h, linear, vec![member, group]);
            r.set_main_effects_for_document(&doc).unwrap();
            let mut request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16);
            request.background = false;
            let (plan, _) = r
                .plan(
                    &doc,
                    "Ptest-root",
                    ticks(0.5),
                    &request,
                    &MapResolver::default(),
                )
                .unwrap();
            assert!(
                plan.draws
                    .iter()
                    .any(|draw| matches!(&draw.content, pm_render::plan::Content::Composite(_)))
            );
            r.set_direct_group_target(false);
            let before = render(&mut r, &doc, 0.5, &request);
            r.set_direct_group_target(true);
            let count = r.direct_group_target_draws();
            let after = render(&mut r, &doc, 0.5, &request);
            assert_eq!(
                r.direct_group_target_draws(),
                count + 1,
                "probe must use the direct path"
            );
            assert_raw_equal(
                &before,
                &after,
                w,
                &format!("direct group {w}×{h}, linear={linear}"),
            );
        }
    }
}
