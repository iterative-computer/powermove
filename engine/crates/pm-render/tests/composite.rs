//! The compositor on generated documents against f64 references of
//! today's formulas (`legacy/gl/compositor.ts`, `shaders.ts`), in both
//! working encodings and both precisions: the nine blend modes
//! (`FRAG_COMPOSITE`), parametric masks (`FRAG_MASK`), track mattes
//! (`applyTrackMatte`, with today's visibility rules), styled groups as
//! compositing boundaries, adjustment layers and layer effects through an
//! effect host (`compositeAdjustment`, `runEffects`), isolation of failing
//! nodes, regions of interest with effect padding, determinism, and the
//! project color settings. Parity against the legacy renderer itself is in
//! `tests/corpus.rs` (the `composite/*` corpus cases).

mod support;

use std::sync::Arc;

use pm_color::linear::{
    composite as reference_composite, over, premultiplied_e_to_linear, premultiplied_linear_to_e,
};
use pm_protocol::color::{BlendDomain, BlendMode, Precision, WorkingEncoding};
use pm_protocol::common::{PixelRect, Size};
use pm_render::composite::effects::{Covered, EffectContext, EffectFrame};
use pm_render::composite::{
    EffectHost, EffectPasses, Node as CompositeNode, Padding, PreparedEffects,
};
use pm_render::plan::Content;
use pm_render::{Feature, MapResolver, RenderError, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const MODES: [(&str, BlendMode); 9] = [
    ("normal", BlendMode::Normal),
    ("add", BlendMode::Add),
    ("screen", BlendMode::Screen),
    ("multiply", BlendMode::Multiply),
    ("overlay", BlendMode::Overlay),
    ("softlight", BlendMode::SoftLight),
    ("difference", BlendMode::Difference),
    ("lighten", BlendMode::Lighten),
    ("darken", BlendMode::Darken),
];

fn hex(c: &str) -> [f64; 4] {
    let p = pm_color::standard::parse_hex(c);
    let a = p.alpha;
    [p.rgb[0] * a, p.rgb[1] * a, p.rgb[2] * a, a]
}

fn with_color(mut doc: Value, linear: bool, domain: &str, precision: &str) -> Value {
    doc["color"] = json!({ "mode": "standard", "linearBlending": linear, "precision": precision, "blendDomain": domain });
    doc
}

fn render_with(
    r: &mut Renderer,
    doc: &Value,
    t: f64,
    precision: Precision,
    region: Option<PixelRect>,
) -> (Vec<f32>, Vec<pm_render::LayerFailure>) {
    let w = doc["w"].as_u64().unwrap() as u32;
    let h = doc["h"].as_u64().unwrap() as u32;
    let mut request = RenderRequest::at_scale(Size::new(w, h), 1.0, precision);
    request.region = region;
    let frame = r
        .render(
            &load(doc),
            "Ptest-root",
            ticks(t),
            &request,
            &MapResolver::default(),
        )
        .unwrap_or_else(|e| panic!("render: {e}"));
    let rgba = r.read_rgba(&frame).unwrap();
    (rgba, frame.failures)
}

fn render(r: &mut Renderer, doc: &Value, precision: Precision) -> Vec<f32> {
    let (rgba, failures) = render_with(r, doc, 0.0, precision, None);
    assert!(failures.is_empty(), "{failures:?}");
    rgba
}

fn px(rgba: &[f32], w: u32, x: u32, y: u32) -> [f64; 4] {
    let i = ((y * w + x) * 4) as usize;
    [0, 1, 2, 3].map(|k| f64::from(rgba[i + k]))
}

fn close(a: [f64; 4], b: [f64; 4], tol: f64) -> bool {
    (0..4).all(|k| (a[k] - b[k]).abs() <= tol)
}

/// A working-encoding reference pixel to the frame's output (premultiplied E).
fn output(v: [f64; 4], working: WorkingEncoding) -> [f64; 4] {
    match working {
        WorkingEncoding::Standard => v,
        WorkingEncoding::LinearLight => premultiplied_linear_to_e(v),
    }
}

fn input(v: [f64; 4], working: WorkingEncoding) -> [f64; 4] {
    match working {
        WorkingEncoding::Standard => v,
        WorkingEncoding::LinearLight => premultiplied_e_to_linear(v),
    }
}

/// Tolerance of a pixel computed through half-float storage (each pass
/// rounds to 11 significant bits) or single-precision storage.
fn tolerance(precision: Precision) -> f64 {
    match precision {
        Precision::Float16 => 4e-3,
        Precision::Float32 => 2e-5,
    }
}

const SETUPS: [(bool, &str, WorkingEncoding, BlendDomain); 3] = [
    (
        false,
        "hdrSafe",
        WorkingEncoding::Standard,
        BlendDomain::HdrSafe,
    ),
    (
        true,
        "hdrSafe",
        WorkingEncoding::LinearLight,
        BlendDomain::HdrSafe,
    ),
    (
        true,
        "displayEncoded",
        WorkingEncoding::LinearLight,
        BlendDomain::DisplayEncoded,
    ),
];

/// Every mode, over an opaque and a translucent destination, at full and
/// partial opacity, in Standard and both linear-light domains, at 16 and
/// 32 bits: `FRAG_COMPOSITE` exactly.
#[test]
fn blend_modes_match_the_reference() {
    let mut r = Renderer::headless().unwrap();
    let pairs = [
        ("#336699", "#CC5588"),
        ("#E0E0E0", "#202020"),
        ("#808080", "#FF7F00"),
        ("#33669980", "#CCAA22"),
        ("#F0C02040", "#2050F0C0"),
    ];
    let mut checked = 0;
    for (lin, domain, working, blend_domain) in SETUPS {
        for precision in [Precision::Float16, Precision::Float32] {
            let prec = if precision == Precision::Float32 {
                "float32"
            } else {
                "float16"
            };
            for (dst, src) in pairs {
                for (name, mode) in MODES {
                    for opacity in [100.0, 60.0] {
                        let mut top = layer(
                            "top",
                            "solid",
                            json!({ "color": src, "w": 8, "h": 8 }),
                            0.0,
                            0.0,
                            1.0,
                        );
                        top["blend"] = json!(name);
                        set(&mut top, "opacity", json!(opacity));
                        let bottom = layer(
                            "bottom",
                            "solid",
                            json!({ "color": dst, "w": 16, "h": 8 }),
                            0.0,
                            0.0,
                            1.0,
                        );
                        let doc = with_color(
                            project(16, 8, 1.0, no_fill(), vec![top, bottom]),
                            lin,
                            domain,
                            prec,
                        );
                        let rgba = render(&mut r, &doc, Precision::Float16);
                        let d = input(hex(dst), working);
                        let s = input(hex(src), working).map(|v| v * opacity / 100.0);
                        let expected = if mode == BlendMode::Normal {
                            over(s, d)
                        } else {
                            reference_composite(mode, working, blend_domain, s, d, 1.0)
                        };
                        let got = px(&rgba, 16, 4, 4);
                        let want = output(expected, working);
                        assert!(
                            close(got, want, tolerance(precision)),
                            "{name} {src} over {dst} at {opacity}% ({working:?}, {blend_domain:?}, {precision:?}): {got:?} != {want:?}"
                        );
                        // Outside the top layer the destination is untouched.
                        assert!(close(
                            px(&rgba, 16, 12, 4),
                            output(d, working),
                            tolerance(precision)
                        ));
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, 3 * 2 * 5 * 9 * 2);
}

/// A styled group (blend mode, opacity) composites its members first, then
/// blends the group image (`compositeAlpha` = the group's opacity).
#[test]
fn styled_groups_are_compositing_boundaries() {
    let mut r = Renderer::headless().unwrap();
    for (lin, domain, working, blend_domain) in SETUPS {
        let mut a = layer(
            "a",
            "solid",
            json!({ "color": "#FF6B1A", "w": 12, "h": 8 }),
            0.0,
            0.0,
            1.0,
        );
        a["group"] = json!("g");
        let mut b = layer(
            "b",
            "solid",
            json!({ "color": "#4C8DFFC0", "w": 12, "h": 8 }),
            4.0,
            0.0,
            1.0,
        );
        b["group"] = json!("g");
        let mut g = layer("g", "group", json!({}), 0.0, 0.0, 1.0);
        g["blend"] = json!("multiply");
        set(&mut g, "opacity", json!(70));
        let back = layer(
            "back",
            "solid",
            json!({ "color": "#A0B0C0", "w": 16, "h": 8 }),
            0.0,
            0.0,
            1.0,
        );
        let doc = with_color(
            project(16, 8, 1.0, no_fill(), vec![b, a, g, back]),
            lin,
            domain,
            "float32",
        );
        let rgba = render(&mut r, &doc, Precision::Float32);
        let d = input(hex("#A0B0C0"), working);
        let sa = input(hex("#FF6B1A"), working);
        let sb = input(hex("#4C8DFFC0"), working);
        for (x, image) in [(2, sa), (8, over(sb, sa)), (14, sb)] {
            let want = output(
                reference_composite(BlendMode::Multiply, working, blend_domain, image, d, 0.7),
                working,
            );
            let got = px(&rgba, 16, x, 4);
            assert!(
                close(got, want, 2e-5),
                "x={x} {working:?}: {got:?} != {want:?}"
            );
        }
    }
}

fn masked_doc(precision: &str, linear: bool) -> Value {
    let mut l = layer(
        "m",
        "solid",
        json!({ "color": "#E8E2CF", "w": 64, "h": 48 }),
        30.0,
        20.0,
        1.0,
    );
    set(&mut l, "rotation", json!(17));
    set(&mut l, "scale.x", json!(130));
    set(&mut l, "skew", json!(-9));
    set(&mut l, "opacity", json!(85));
    l["masks"] = json!([
        { "id": "k1", "shape": "ellipse", "mode": "add", "on": true,
          "p": { "x": ch(json!(30)), "y": ch(json!(24)), "w": ch(json!(50)), "h": ch(json!(30)), "rotation": ch(json!(20)), "feather": ch(json!(12)) } },
        { "id": "k2", "shape": "rect", "mode": "subtract", "on": true,
          "p": { "x": ch(json!(40)), "y": ch(json!(20)), "w": ch(json!(14)), "h": ch(json!(-10)), "rotation": ch(json!(0)), "feather": ch(json!(0)) } },
        { "id": "k3", "shape": "rect", "mode": "add", "on": false,
          "p": { "x": ch(json!(0)), "y": ch(json!(0)), "w": ch(json!(500)), "h": ch(json!(500)), "rotation": ch(json!(0)), "feather": ch(json!(0)) } },
        { "id": "k4", "shape": "rect", "mode": "add", "on": true,
          "p": { "x": ch(json!(5)), "y": ch(json!(40)), "w": ch(json!(10)), "h": ch(json!(10)), "rotation": ch(json!(45)), "feather": ch(json!(0.005)) } }
    ]);
    let back = layer(
        "back",
        "solid",
        json!({ "color": "#202436", "w": 128, "h": 96 }),
        0.0,
        0.0,
        1.0,
    );
    with_color(
        project(128, 96, 1.0, no_fill(), vec![l, back]),
        linear,
        "hdrSafe",
        precision,
    )
}

/// Mask coverage on the GPU equals `FRAG_MASK` in single precision
/// (`pm_vector::ParametricMasks::coverage`) at every pixel center, through
/// the layer's inverse world matrix, rounded to the storage precision.
#[test]
fn masks_match_the_coverage_reference() {
    let mut r = Renderer::headless().unwrap();
    for (precision, name) in [
        (Precision::Float16, "float16"),
        (Precision::Float32, "float32"),
    ] {
        for linear in [false, true] {
            let doc = masked_doc(name, linear);
            let working = if linear {
                WorkingEncoding::LinearLight
            } else {
                WorkingEncoding::Standard
            };
            let request = RenderRequest::at_scale(Size::new(128, 96), 1.0, precision);
            let (plan, _) = r
                .plan(
                    &load(&doc),
                    "Ptest-root",
                    ticks(0.0),
                    &request,
                    &MapResolver::default(),
                )
                .unwrap();
            let node = plan
                .draws
                .iter()
                .find_map(|d| match &d.content {
                    Content::Composite(n) => Some(n.as_ref().clone()),
                    _ => None,
                })
                .expect("a layer node");
            let CompositeNode::Layer(node) = node else {
                panic!()
            };
            let masks = node.masks.expect("masks");
            assert_eq!(masks.masks.masks.len(), 3, "the disabled mask is filtered");
            let rgba = render(&mut r, &doc, precision);
            // The layer alone (a second render without masks) gives the
            // unmasked layer pixel to scale.
            let mut plain = doc.clone();
            plain["layers"][0]["masks"] = json!([]);
            let unmasked = render(&mut r, &plain, precision);
            let back = input(hex("#202436"), working);
            let inv = masks.inverse;
            let (mut checked, mut off) = (0, 0);
            for y in 0..96 {
                for x in 0..128 {
                    let (qx, qy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let m: Vec<f32> = inv.iter().map(|v| *v as f32).collect();
                    let lx = m[0] * qx + m[3] * qy + m[6];
                    let ly = m[1] * qx + m[4] * qy + m[7];
                    let lz = m[2] * qx + m[5] * qy + m[8];
                    let mut cov = masks.masks.coverage(pm_vector::kurbo::Point::new(
                        f64::from(lx / lz),
                        f64::from(ly / lz),
                    ));
                    if precision == Precision::Float16 {
                        cov = f16_round(cov);
                    }
                    // Layer over the background: unmasked = L + B(1 − La).
                    let u = input(px(&unmasked, 128, x, y), working);
                    let la = [
                        (u[0] - back[0] * (1.0 - u[3])) / 1.0,
                        (u[1] - back[1] * (1.0 - u[3])) / 1.0,
                        (u[2] - back[2] * (1.0 - u[3])) / 1.0,
                        u[3],
                    ];
                    let layer_px = la.map(|v| v * f64::from(cov));
                    let want = output(over(layer_px, back), working);
                    let got = px(&rgba, 128, x, y);
                    checked += 1;
                    if !close(got, want, 4.0 * tolerance(precision)) {
                        off += 1;
                    }
                }
            }
            // Pixel centers exactly on a hard edge may round either way.
            assert!(
                off * 1000 <= checked,
                "{precision:?} linear={linear}: {off} of {checked} pixels off"
            );
        }
    }
}

/// Round to binary16 (nearest even), as a half-float target stores.
fn f16_round(v: f32) -> f32 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let e = v.abs().log2().floor().max(-14.0);
    let step = 2f32.powf(e - 10.0);
    (v / step).round_ties_even() * step
}

fn matte_doc(mode: &str, source_on: bool, source_from: f64) -> Value {
    let mut target = layer(
        "t",
        "solid",
        json!({ "color": "#3FCF8E", "w": 16, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    target["matteSource"] = json!("s");
    target["matteMode"] = ch(json!(mode));
    let mut source = layer(
        "s",
        "solid",
        json!({ "color": "#C0804099", "w": 8, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    source["on"] = json!(source_on);
    source["from"] = json!(source_from);
    set(&mut source, "opacity", json!(80));
    project(16, 8, 1.0, no_fill(), vec![target, source])
}

/// The four matte modes: alpha or Rec.709 luma of the premultiplied E
/// matte, inverted or not, clamped; the source draws even when switched off
/// and never on its own; a source outside its time span gives an empty
/// matte.
#[test]
fn track_mattes_follow_todays_rules() {
    let mut r = Renderer::headless().unwrap();
    let t = hex("#3FCF8E");
    let s = hex("#C0804099").map(|v| v * 0.8);
    for linear in [false, true] {
        let working = if linear {
            WorkingEncoding::LinearLight
        } else {
            WorkingEncoding::Standard
        };
        for (mode, luma, inverted) in [
            ("alpha", false, false),
            ("alpha-inverted", false, true),
            ("luma", true, false),
            ("luma-inverted", true, true),
        ] {
            for on in [true, false] {
                let doc = with_color(matte_doc(mode, on, 0.0), linear, "hdrSafe", "float32");
                let rgba = render(&mut r, &doc, Precision::Float32);
                let m = pm_color::linear::matte_value(input(s, working), luma, inverted, working);
                let outside = pm_color::linear::matte_value([0.0; 4], luma, inverted, working);
                let tl = input(t, working);
                let left = output(tl.map(|v| v * m), working);
                let right = output(tl.map(|v| v * outside), working);
                assert!(
                    close(px(&rgba, 16, 3, 4), left, 1e-4),
                    "{mode} on={on} linear={linear}: {:?} != {left:?}",
                    px(&rgba, 16, 3, 4)
                );
                assert!(close(px(&rgba, 16, 12, 4), right, 1e-4), "{mode} outside");
            }
            // A source outside its time span: an empty matte.
            let doc = with_color(matte_doc(mode, true, 0.5), linear, "hdrSafe", "float32");
            let rgba = render(&mut r, &doc, Precision::Float32);
            let empty = pm_color::linear::matte_value([0.0; 4], luma, inverted, working);
            let want = output(input(t, working).map(|v| v * empty), working);
            assert!(close(px(&rgba, 16, 3, 4), want, 1e-4), "{mode} late source");
        }
    }
}

/// A matte chain deeper than today's limit (8) stops applying mattes, and a
/// cycle renders like today's depth-limited recursion; a group used as a
/// matte source fails that matte (reported) and mattes with nothing.
#[test]
fn matte_depth_cycles_and_group_sources() {
    let mut r = Renderer::headless().unwrap();
    // Both cycle members are matte sources, so neither draws on its own;
    // a third layer matted by one of them sees the depth-limited chain:
    // passes alternate a, b, a, … and the eighth (b, alpha 0.5) draws
    // unmatted, so the alphas multiply to 0.5⁴ at the top.
    let mut t = layer(
        "t",
        "solid",
        json!({ "color": "#FF0000", "w": 8, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    t["matteSource"] = json!("a");
    let mut a = layer(
        "a",
        "solid",
        json!({ "color": "#00FF00", "w": 8, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    a["matteSource"] = json!("b");
    let mut b = layer(
        "b",
        "solid",
        json!({ "color": "#0000FF80", "w": 8, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    b["matteSource"] = json!("a");
    let alpha_b = hex("#0000FF80")[3];
    let doc = project(8, 8, 1.0, no_fill(), vec![t, a, b]);
    let (rgba, failures) = render_with(&mut r, &doc, 0.0, Precision::Float32, None);
    assert!(failures.is_empty());
    let k = alpha_b.powi(4);
    let got = px(&rgba, 8, 4, 4);
    assert!(close(got, [k, 0.0, 0.0, k], 1e-6), "{got:?}");
    // A group source.
    let mut t = layer(
        "t",
        "solid",
        json!({ "color": "#FF0000", "w": 8, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    t["matteSource"] = json!("g");
    let mut m = layer(
        "m",
        "solid",
        json!({ "color": "#FFFFFF", "w": 8, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    m["group"] = json!("g");
    let g = layer("g", "group", json!({}), 0.0, 0.0, 1.0);
    let doc = project(8, 8, 1.0, no_fill(), vec![t, m, g]);
    let (rgba, failures) = render_with(&mut r, &doc, 0.0, Precision::Float32, None);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].layer, "g");
    // The target is matted to nothing; the group's member draws below it.
    assert!(close(px(&rgba, 8, 4, 4), [1.0, 1.0, 1.0, 1.0], 1e-6));
}

// ── Effects through a test host ──────────────────────────────────────

const EFFECT_WGSL: &str = r#"
struct P { shift: vec4<i32>, mode: vec4<i32> };
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var src: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u)) * 4.0 - 1.0;
    return vec4<f32>(a, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(pos.xy) - p.shift.xy;
    let size = vec2<i32>(textureDimensions(src));
    if (at.x < 0 || at.y < 0 || at.x >= size.x || at.y >= size.y) { return vec4<f32>(0.0); }
    let c = textureLoad(src, at, 0);
    if (p.mode.x == 1) { return vec4<f32>(c.a - c.rgb, c.a); }
    return c;
}
"#;

/// A test effect: invert (premultiplied `a − rgb`) or shift right.
#[derive(Clone, Copy, PartialEq, Debug)]
enum TestEffect {
    Invert,
    ShiftRight(i32),
    Fail,
    Nothing,
}

struct Passes(TestEffect);

impl EffectPasses for Passes {
    fn key(&self) -> Vec<u8> {
        format!("{:?}", self.0).into_bytes()
    }

    fn record(
        &self,
        cx: &mut EffectContext<'_>,
        input: Covered<'_>,
        _backdrop: Option<Covered<'_>>,
    ) -> Result<Option<pm_gpu::Target>, String> {
        let (mode, shift) = match self.0 {
            TestEffect::Invert => (1, 0),
            TestEffect::ShiftRight(s) => (0, s),
            TestEffect::Fail => return Err("the test effect failed".into()),
            TestEffect::Nothing => return Ok(None),
        };
        let device = cx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(EFFECT_WGSL.into()),
        });
        let out = cx
            .pool
            .take_working(device, input.rect.width, input.rect.height, cx.precision);
        cx.taken.push(out.clone());
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: out.format(),
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let data: [i32; 8] = [shift, 0, 0, 0, mode, 0, 0, 0];
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        cx.queue.write_buffer(&buffer, 0, &bytes);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&input.target.view),
                },
            ],
        });
        let mut pass = cx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &out.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..4, 0..1);
        drop(pass);
        Ok(Some(out))
    }
}

/// Reads `fx[0].type` as the test effect.
struct Host;

impl EffectHost for Host {
    fn prepare<'d>(
        &self,
        scope: &mut pm_eval::Scope<'_, 'd>,
        layer: &'d pm_model::Node,
        _frame: EffectFrame,
    ) -> pm_eval::JsResult<Result<Option<PreparedEffects>, Feature>> {
        let _ = scope;
        let fx = pm_eval::node::array(pm_eval::node::member(Some(layer), "fx")).unwrap_or(&[]);
        let Some(first) = fx.first() else {
            return Ok(Ok(None));
        };
        let ty = pm_eval::node::string(pm_eval::node::member(Some(first), "type"))
            .unwrap()
            .to_string_lossy();
        let (effect, padding) = match ty.as_str() {
            "invert" => (TestEffect::Invert, Padding::NONE),
            "shift" => (TestEffect::ShiftRight(6), Padding::Pixels(6)),
            "shift-full" => (TestEffect::ShiftRight(6), Padding::Full),
            "fail" => (TestEffect::Fail, Padding::NONE),
            "nothing" => (TestEffect::Nothing, Padding::NONE),
            _ => return Ok(Err(Feature::Effects)),
        };
        Ok(Ok(Some(PreparedEffects {
            padding,
            backdrop: false,
            passes: Arc::new(Passes(effect)),
        })))
    }
}

fn effect(ty: &str) -> Value {
    json!([{ "id": "f1", "type": ty, "on": true, "p": {} }])
}

/// `compositeAdjustment`: the stack beneath through the effects, masked,
/// composited over the stack with the layer's opacity and blend through
/// `FRAG_COMPOSITE` (normal included); layers above are untouched; an
/// adjustment whose effects do not run is a no-op.
#[test]
fn adjustment_layers_process_the_stack_beneath() {
    let mut r = Renderer::headless().unwrap();
    r.set_effect_host(Some(Arc::new(Host)));
    for (lin, domain, working, blend_domain) in SETUPS {
        for (blend_name, mode) in [("normal", BlendMode::Normal), ("screen", BlendMode::Screen)] {
            let above = layer(
                "above",
                "solid",
                json!({ "color": "#FF6B1A", "w": 4, "h": 4 }),
                0.0,
                0.0,
                1.0,
            );
            let mut adj = layer("adj", "adjustment", json!({}), 0.0, 0.0, 1.0);
            adj["fx"] = effect("invert");
            adj["blend"] = json!(blend_name);
            set(&mut adj, "opacity", json!(60));
            adj["masks"] = json!([{ "id": "k", "shape": "rect", "mode": "add", "on": true,
                "p": { "x": ch(json!(8)), "y": ch(json!(8)), "w": ch(json!(8)), "h": ch(json!(16)), "rotation": ch(json!(0)), "feather": ch(json!(0)) } }]);
            let back = layer(
                "back",
                "solid",
                json!({ "color": "#4080C0B0", "w": 16, "h": 16 }),
                0.0,
                0.0,
                1.0,
            );
            let doc = with_color(
                project(16, 16, 1.0, no_fill(), vec![above, adj, back]),
                lin,
                domain,
                "float32",
            );
            let rgba = render(&mut r, &doc, Precision::Float32);
            let acc = input(hex("#4080C0B0"), working);
            let inverted = [acc[3] - acc[0], acc[3] - acc[1], acc[3] - acc[2], acc[3]];
            // Inside the mask (x < 12).
            let want = output(
                reference_composite(mode, working, blend_domain, inverted, acc, 0.6),
                working,
            );
            assert!(
                close(px(&rgba, 16, 8, 8), want, 2e-5),
                "{blend_name} {working:?}: {:?} != {want:?}",
                px(&rgba, 16, 8, 8)
            );
            // Outside the mask: the contribution is transparent.
            let outside = output(
                reference_composite(mode, working, blend_domain, [0.0; 4], acc, 0.6),
                working,
            );
            assert!(close(px(&rgba, 16, 14, 8), outside, 2e-5));
            // Above the adjustment.
            assert!(close(
                px(&rgba, 16, 1, 1),
                output(input(hex("#FF6B1A"), working), working),
                2e-5
            ));
        }
    }
    // An effect that does not run leaves the stack as it is.
    let mut adj = layer("adj", "adjustment", json!({}), 0.0, 0.0, 1.0);
    adj["fx"] = effect("nothing");
    let back = layer(
        "back",
        "solid",
        json!({ "color": "#4080C0B0", "w": 16, "h": 16 }),
        0.0,
        0.0,
        1.0,
    );
    let doc = project(16, 16, 1.0, no_fill(), vec![adj, back]);
    let rgba = render(&mut r, &doc, Precision::Float32);
    assert!(close(px(&rgba, 16, 8, 8), hex("#4080C0B0"), 1e-6));
}

/// Section 8 isolation: a node that fails reports its layer and draws
/// nothing; every other layer renders. Without an effect host, effects are
/// routed (unsupported), as before.
#[test]
fn a_failing_node_outputs_transparency() {
    let mut r = Renderer::headless().unwrap();
    let mut bad = layer(
        "bad",
        "solid",
        json!({ "color": "#FFFFFF", "w": 8, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    bad["fx"] = effect("fail");
    let back = layer(
        "back",
        "solid",
        json!({ "color": "#336699", "w": 8, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    let doc = project(8, 8, 1.0, no_fill(), vec![bad, back]);
    match r.render(
        &load(&doc),
        "Ptest-root",
        ticks(0.0),
        &RenderRequest::at_scale(Size::new(8, 8), 1.0, Precision::Float32),
        &MapResolver::default(),
    ) {
        Err(RenderError::Unsupported(u)) => {
            assert!(u.iter().any(|u| u.feature == Feature::Effects))
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
    r.set_effect_host(Some(Arc::new(Host)));
    let (rgba, failures) = render_with(&mut r, &doc, 0.0, Precision::Float32, None);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].layer, "bad");
    assert!(close(px(&rgba, 8, 4, 4), hex("#336699"), 1e-6));
}

/// `masks` that is truthy but not an array fails the layer as today's
/// `(L.masks || []).some(…)` throws: a named layer failure, not a route;
/// the rest of the frame draws.
#[test]
fn masks_that_are_not_an_array_fail_the_layer() {
    let mut r = Renderer::headless().unwrap();
    let mut bad = layer(
        "bad",
        "solid",
        json!({ "color": "#FFFFFF", "w": 8, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    bad["masks"] = json!({ "v": [], "kf": [], "expr": null });
    let back = layer(
        "back",
        "solid",
        json!({ "color": "#336699", "w": 8, "h": 8 }),
        0.0,
        0.0,
        1.0,
    );
    let doc = project(8, 8, 1.0, no_fill(), vec![bad, back]);
    assert!(pm_render::unsupported(&load(&doc), "Ptest-root").is_empty());
    let (rgba, failures) = render_with(&mut r, &doc, 0.0, Precision::Float32, None);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].layer, "bad");
    assert!(close(px(&rgba, 8, 4, 4), hex("#336699"), 1e-6));
}

/// A region of interest equals the same pixels of the full frame, bit for
/// bit, with masks, mattes, blend modes, groups, and effects that read
/// beyond the region (grown by their padding, or the whole frame).
#[test]
fn regions_equal_the_full_frame_with_effect_padding() {
    let mut r = Renderer::headless().unwrap();
    r.set_effect_host(Some(Arc::new(Host)));
    for shift in ["shift", "shift-full"] {
        let mut doc = masked_doc("float16", false);
        let layers = doc["layers"].as_array_mut().unwrap();
        let mut shifted = layer(
            "sh",
            "solid",
            json!({ "color": "#3FCF8EC0", "w": 30, "h": 20 }),
            40.0,
            50.0,
            1.0,
        );
        shifted["fx"] = effect(shift);
        shifted["blend"] = json!("overlay");
        let mut adj = layer("adj", "adjustment", json!({}), 0.0, 0.0, 1.0);
        adj["fx"] = effect(shift);
        set(&mut adj, "opacity", json!(50));
        let mut t = layer(
            "t",
            "solid",
            json!({ "color": "#F2C14E", "w": 50, "h": 30 }),
            60.0,
            30.0,
            1.0,
        );
        t["matteSource"] = json!("src");
        t["matteMode"] = ch(json!("luma"));
        let mut src = layer(
            "src",
            "solid",
            json!({ "color": "#808080", "w": 30, "h": 30 }),
            70.0,
            35.0,
            1.0,
        );
        src["on"] = json!(false);
        layers.insert(0, adj);
        layers.insert(0, shifted);
        layers.insert(0, t);
        layers.insert(0, src);
        let (full, _) = render_with(&mut r, &doc, 0.0, Precision::Float16, None);
        for region in [
            PixelRect {
                x: 0,
                y: 0,
                width: 128,
                height: 96,
            },
            PixelRect {
                x: 43,
                y: 51,
                width: 20,
                height: 13,
            },
            PixelRect {
                x: 0,
                y: 40,
                width: 9,
                height: 56,
            },
            PixelRect {
                x: 100,
                y: 0,
                width: 28,
                height: 30,
            },
        ] {
            let (part, failures) = render_with(&mut r, &doc, 0.0, Precision::Float16, Some(region));
            assert!(failures.is_empty(), "{failures:?}");
            for y in 0..region.height {
                for x in 0..region.width {
                    let a = px(&part, region.width, x, y);
                    let b = px(&full, 128, region.x + x, region.y + y);
                    assert_eq!(a, b, "{shift} region {region:?} at ({x}, {y})");
                }
            }
        }
    }
}

/// Section 2 determinism: the same frame twice is bit-identical, and so is
/// its plan.
#[test]
fn compositing_is_deterministic() {
    let mut r = Renderer::headless().unwrap();
    r.set_effect_host(Some(Arc::new(Host)));
    for linear in [false, true] {
        let mut doc = masked_doc("float16", linear);
        let mut t = layer(
            "t",
            "solid",
            json!({ "color": "#F2C14E", "w": 50, "h": 30 }),
            60.0,
            30.0,
            1.0,
        );
        t["matteSource"] = json!("src");
        t["blend"] = json!("softlight");
        let mut src = layer(
            "src",
            "solid",
            json!({ "color": "#80808080", "w": 30, "h": 30 }),
            70.0,
            35.0,
            1.0,
        );
        src["on"] = json!(false);
        let mut adj = layer("adj", "adjustment", json!({}), 0.0, 0.0, 1.0);
        adj["fx"] = effect("invert");
        let layers = doc["layers"].as_array_mut().unwrap();
        layers.insert(0, adj);
        layers.insert(0, t);
        layers.insert(0, src);
        let a = render(&mut r, &doc, Precision::Float16);
        let b = render(&mut r, &doc, Precision::Float16);
        assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
        let request = RenderRequest::at_scale(Size::new(128, 96), 1.0, Precision::Float16);
        let p1 = r
            .plan(
                &load(&doc),
                "Ptest-root",
                ticks(0.0),
                &request,
                &MapResolver::default(),
            )
            .unwrap();
        let p2 = r
            .plan(
                &load(&doc),
                "Ptest-root",
                ticks(0.0),
                &request,
                &MapResolver::default(),
            )
            .unwrap();
        assert_eq!(p1, p2);
    }
}

/// Settings › Project › Color: the precision setting is a floor for
/// intermediates, unknown settings are routed, and linear blending is
/// rendered (not routed).
#[test]
fn project_color_settings() {
    let mut r = Renderer::headless().unwrap();
    let doc = with_color(
        project(8, 8, 1.0, solid_fill("#336699"), vec![]),
        false,
        "hdrSafe",
        "float32",
    );
    let frame = r
        .render(
            &load(&doc),
            "Ptest-root",
            ticks(0.0),
            &RenderRequest::at_scale(Size::new(8, 8), 1.0, Precision::Float16),
            &MapResolver::default(),
        )
        .unwrap();
    assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba32Float);
    let mut bad = doc.clone();
    bad["color"]["mode"] = json!("aces");
    match r.render(
        &load(&bad),
        "Ptest-root",
        ticks(0.0),
        &RenderRequest::at_scale(Size::new(8, 8), 1.0, Precision::Float16),
        &MapResolver::default(),
    ) {
        Err(RenderError::Unsupported(u)) => assert_eq!(u[0].feature, Feature::ProjectColor),
        other => panic!("{:?}", other.map(|_| ())),
    }
    let linear = with_color(
        project(8, 8, 1.0, solid_fill("#336699"), vec![]),
        true,
        "hdrSafe",
        "float16",
    );
    assert!(pm_render::unsupported(&load(&linear), "Ptest-root").is_empty());
    let rgba = render(&mut r, &linear, Precision::Float16);
    assert!(
        close(px(&rgba, 8, 2, 2), hex("#336699"), 2e-3),
        "{:?}",
        px(&rgba, 8, 2, 2)
    );
}
