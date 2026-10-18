//! Section 2's third linear-blending line, measured for the Phase 3a gate
//! (docs/engine/phase3a-gate.md): "A Gaussian blur of a white line conserves
//! energy within 0.5%" (computed at 32-bit, sRGB output).
//!
//! A one-pixel white line on black, blurred by the built-in `blur` effect
//! (the definition the kernel pushes), rendered at 32-bit with linear
//! blending on and off, in two forms:
//!
//! - **layer**: the effect on the line's own layer (transparent around it),
//!   composited over the black background;
//! - **stack**: an adjustment layer blurring the line already composited
//!   over an opaque black solid (the effect sees opaque pixels, so the
//!   section 11 rule that legacy GLSL reads E-encoded values decides the
//!   result).
//!
//! Energy is the sum over the frame of the linear-light value (the sRGB
//! decode of the output), against the same frame without the blur. The line
//! is far from every edge, so nothing leaves the frame at the radii measured.
//!
//! A measurement of the gate, not a regression test: run it with
//! `cargo test --release -p pm-cli --test phase3a_bars -- --ignored --nocapture`.

use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_render::{MapResolver, RenderRequest, Renderer, all_builtin_definitions};
use serde_json::{Value, json};

const W: u32 = 512;
const H: u32 = 64;

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn transform(x: f64, y: f64) -> Value {
    json!({
        "perspective": ch(json!(50)), "position.z": ch(json!(0)), "anchor.z": ch(json!(0)),
        "scale.z": ch(json!(100)), "rotation.x": ch(json!(0)), "rotation.y": ch(json!(0)),
        "orientation.x": ch(json!(0)), "orientation.y": ch(json!(0)), "orientation.z": ch(json!(0)),
        "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(x)),
        "position.y": ch(json!(y)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
        "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
    })
}

fn layer(id: &str, ty: &str, x: f64, d: Value, fx: Value) -> Value {
    json!({
        "id": id, "type": ty, "name": id, "from": 0, "dur": 1, "on": true,
        "lock": false, "shy": false, "collapsed": true, "threeD": false, "solo": false,
        "color": "#4C8DFF", "blend": "normal", "mblur": false, "parent": null, "group": null,
        "matteSource": null, "p": transform(x, 0.0), "fx": fx,
        "transitionIn": null, "transitionOut": null, "masks": [],
        "d": d, "locked_intent": {}
    })
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Form {
    /// The blur on the line's own layer.
    Layer,
    /// An adjustment layer blurring the line over an opaque black solid.
    Stack,
}

/// A white line one pixel wide across the frame's height, in the middle,
/// with the blur effect at `amount` (none at 0), linear blending on or off.
fn document(form: Form, amount: f64, linear: bool) -> pm_model::Document {
    let fx = if amount > 0.0 {
        json!([{ "id": "f1", "type": "blur", "on": true, "p": { "amount": ch(json!(amount)) } }])
    } else {
        json!([])
    };
    // Effects run on composition-sized buffers, so the blur spreads past
    // the line's pixel.
    let line_d = json!({ "color": "#FFFFFF", "w": 1, "h": H });
    let layers = match form {
        Form::Layer => vec![layer("line", "solid", f64::from(W / 2), line_d, fx)],
        Form::Stack => vec![
            layer("adj", "adjustment", 0.0, json!({}), fx),
            layer("line", "solid", f64::from(W / 2), line_d, json!([])),
            layer(
                "black",
                "solid",
                0.0,
                json!({ "color": "#000000", "w": W, "h": H }),
                json!([]),
            ),
        ],
    };
    let doc = json!({
        "id": "Pbars", "name": "bars", "w": W, "h": H, "fps": 30, "dur": 1, "bg": "#000000",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "compId": "Pbars-root", "compName": "bars", "markers": [], "work": [0, 1], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": layers,
        "assets": {}, "comps": {},
        "color": { "mode": "standard", "linearBlending": linear, "precision": "float32", "blendDomain": "hdrSafe" }
    });
    let value = pm_model::json::parse(&doc.to_string()).expect("document");
    pm_model::Document::new(Some(value), 1)
}

/// The sRGB decode (the display encoding E of Standard color).
fn linear(e: f64) -> f64 {
    if e <= 0.04045 {
        e / 12.92
    } else {
        ((e + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear-light energy of the red channel (the line is neutral white).
fn energy(rgba: &[f32]) -> f64 {
    rgba.chunks_exact(4).map(|p| linear(f64::from(p[0]))).sum()
}

fn render(r: &mut Renderer, doc: &pm_model::Document) -> Vec<f32> {
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    let frame = r
        .render(
            doc,
            "Pbars-root",
            Ticks::ZERO,
            &request,
            &MapResolver::default(),
        )
        .expect("render");
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    r.read_rgba(&frame).expect("readback")
}

#[test]
#[ignore = "gate measurement (docs/engine/phase3a-gate.md); run with --ignored"]
fn gaussian_blur_of_a_white_line_conserves_energy() {
    let mut r = Renderer::headless().expect("a Metal device");
    r.set_definitions(&all_builtin_definitions())
        .expect("built-in definitions");
    let mut worst_linear: f64 = 0.0;
    let mut worst_stack: f64 = 0.0;
    for (form, on) in [
        (Form::Layer, true),
        (Form::Layer, false),
        (Form::Stack, true),
        (Form::Stack, false),
    ] {
        let sharp = energy(&render(&mut r, &document(form, 0.0, on)));
        assert!(sharp > 0.0, "the line draws");
        for amount in [2.0, 8.0, 24.0, 64.0] {
            let blurred = render(&mut r, &document(form, amount, on));
            let ratio = energy(&blurred) / sharp;
            // Where the blur reaches: the widest column still above 1/1024 of the peak.
            let row: Vec<f64> = (0..W as usize)
                .map(|x| linear(f64::from(blurred[4 * x])))
                .collect();
            let peak = row.iter().copied().fold(0.0, f64::max);
            let reach = row.iter().filter(|v| **v > peak / 1024.0).count();
            println!(
                "{form:?}, linear blending {}: blur amount {amount:>4}: energy {:.6} of the sharp line ({:+.3}%), peak {:.5}, {} columns lit",
                if on { "on " } else { "off" },
                ratio,
                (ratio - 1.0) * 100.0,
                peak,
                reach
            );
            match (form, on) {
                (Form::Layer, true) => worst_linear = worst_linear.max((ratio - 1.0).abs()),
                (Form::Stack, true) => worst_stack = worst_stack.max((ratio - 1.0).abs()),
                _ => {}
            }
        }
    }
    println!(
        "worst energy error with linear blending on: layer blur {:.3}%, stack blur (adjustment layer) {:.3}% (bar 0.5%)",
        worst_linear * 100.0,
        worst_stack * 100.0
    );
    // The bar holds for both forms or not at all: a blurred white line on
    // black is the same picture whether the black is the background or
    // opaque pixels beneath the effect.
    assert!(
        worst_linear <= 0.005 && worst_stack <= 0.005,
        "energy not conserved within 0.5%: layer {:.3}%, stack {:.3}%",
        worst_linear * 100.0,
        worst_stack * 100.0
    );
}
