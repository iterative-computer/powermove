//! Color Wheels and Color Curves against plain f64 references written here
//! from their stated definitions: the ASC CDL (slope, offset, power with
//! negatives passed through as OpenColorIO's no-clamp style, then Rec.709
//! saturation), white balance and exposure as gains on linear light, and
//! monotone cubic curves (Fritsch-Carlson). A 32-bit project, a field that
//! includes light above white and colours outside Rec.709.
mod support;

use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Map, Value, json};
use support::doc::*;

const W: u32 = 48;
const H: u32 = 48;

/// Every colour of the field: 0..1 stretched to -0.25..1.5.
const FIELD: &str = "void main(){vec2 p=gl_FragCoord.xy/vec2(48.0,48.0);\
     vec3 c=vec3(p.x,p.y,fract(p.x*5.0+p.y*3.0))*1.75-0.25;fragColor=vec4(c,1.0);}";

fn effect(id: &str, overrides: &Value) -> Value {
    effect_named("grade", id, overrides)
}

fn effect_named(name: &str, id: &str, overrides: &Value) -> Value {
    let definition = pm_fx_defs::catalog::main_catalog().current(id).unwrap();
    let mut params = Map::new();
    for (key, value) in definition.defaults() {
        params.insert(
            key.clone(),
            ch(overrides.get(&key).cloned().unwrap_or(value)),
        );
    }
    for key in overrides.as_object().unwrap().keys() {
        assert!(params.contains_key(key), "{id} has no parameter {key}");
    }
    json!({"id":name,"type":id,"on":true,"p":params,"inputs":{},
           "version":definition.metadata["version"]})
}

fn draw(renderer: &mut Renderer, fx: Option<Value>) -> Vec<f32> {
    draw_chain(renderer, fx.into_iter().collect())
}

fn draw_chain(renderer: &mut Renderer, fx: Vec<Value>) -> Vec<f32> {
    let mut source = layer(
        "owner",
        "shader",
        json!({"w":W,"h":H,"code":FIELD,"uniforms":{}}),
        0.,
        0.,
        1.,
    );
    source["fx"] = Value::Array(fx);
    let mut value = project(W, H, 1., no_fill(), vec![source]);
    value["colorDepth"] = json!(32);
    let mut doc = load(&value);
    doc.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&doc).unwrap();
    let frame = renderer
        .render(
            &doc,
            "Ptest-root",
            Ticks::ZERO,
            &RenderRequest::sized(Size::new(W, H), Precision::Float32),
            &MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer.read_rgba(&frame).unwrap()
}

/// The largest difference from `reference` over the field (NaN fails).
fn worst(input: &[f32], output: &[f32], reference: impl Fn([f64; 3]) -> [f64; 3]) -> f64 {
    let mut worst = 0f64;
    for (i, o) in input.chunks_exact(4).zip(output.chunks_exact(4)) {
        let x = [i[0], i[1], i[2]].map(f64::from);
        let want = reference(x);
        for c in 0..3 {
            assert!(
                o[c].is_finite() && want[c].is_finite(),
                "{x:?}: {o:?} {want:?}"
            );
            worst = worst.max((f64::from(o[c]) - want[c]).abs());
        }
        assert_eq!(o[3], i[3]);
    }
    worst
}

fn e_decode(v: f64) -> f64 {
    let m = v.abs();
    let l = if m <= 0.04045 {
        m / 12.92
    } else {
        ((m + 0.055) / 1.055).powf(2.4)
    };
    l.copysign(v)
}

fn e_encode(l: f64) -> f64 {
    let m = l.abs();
    let e = if m <= 0.0031308 {
        12.92 * m
    } else {
        1.055 * m.powf(1.0 / 2.4) - 0.055
    };
    e.copysign(l)
}

const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

/// ASC CDL v1.2 SOP and saturation; a negative power base passes through.
fn cdl(x: [f64; 3], slope: [f64; 3], offset: [f64; 3], power: [f64; 3], sat: f64) -> [f64; 3] {
    let sop: [f64; 3] = std::array::from_fn(|c| {
        let v = x[c] * slope[c] + offset[c];
        if v > 0.0 { v.powf(power[c]) } else { v }
    });
    let luma: f64 = (0..3).map(|c| LUMA[c] * sop[c]).sum();
    sop.map(|v| luma + sat * (v - luma))
}

#[test]
fn neutral_grading_tools_return_the_same_bits() {
    let mut renderer = Renderer::headless().unwrap();
    let input = draw(&mut renderer, None);
    for id in ["colorWheels", "colorCurves"] {
        let output = draw(&mut renderer, Some(effect(id, &json!({}))));
        let changed = input
            .iter()
            .zip(&output)
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        assert_eq!(changed, 0, "{id} changed {changed} values");
    }
}

/// A CDL read from a file is the wheels' gain, offset and gamma: slope =
/// gain, offset = offset, power = 2^-gamma (lift 0), saturation in percent.
#[test]
fn color_wheels_are_an_asc_cdl() {
    let mut renderer = Renderer::headless().unwrap();
    let input = draw(&mut renderer, None);
    let (slope, offset, power, sat) = (
        [1.2, 0.95, 0.8],
        [0.02, -0.01, 0.05],
        [0.9, 1.1, 1.25],
        0.85,
    );
    let mut p = json!({"saturation": sat * 100.0});
    for (c, name) in ["Red", "Green", "Blue"].into_iter().enumerate() {
        p[format!("gain{name}")] = json!(slope[c]);
        p[format!("offset{name}")] = json!(offset[c]);
        p[format!("gamma{name}")] = json!(-f64::log2(power[c]));
    }
    let output = draw(&mut renderer, Some(effect("colorWheels", &p)));
    let d = worst(&input, &output, |x| cdl(x, slope, offset, power, sat));
    eprintln!("CDL: {d:e} from the reference");
    assert!(d < 1e-5, "{d:e}");
}

/// Every control at once: white balance and exposure on linear light
/// (balance keeps grey's luminance), contrast about the pivot, lift, gamma,
/// gain and offset with their masters, then saturation.
#[test]
fn color_wheels_match_their_stated_definition() {
    let mut renderer = Renderer::headless().unwrap();
    let input = draw(&mut renderer, None);
    let p = json!({
        "temperature": 30, "tint": -20, "exposure": 0.5, "contrast": 25, "pivot": 0.4,
        "saturation": 120,
        "liftMaster": 0.05, "liftRed": -0.02, "liftGreen": 0.0, "liftBlue": 0.03,
        "gammaMaster": 0.2, "gammaRed": 0.1, "gammaGreen": -0.05, "gammaBlue": 0.0,
        "gainMaster": 1.1, "gainRed": 0.95, "gainGreen": 1.0, "gainBlue": 1.05,
        "offsetMaster": -0.01, "offsetRed": 0.0, "offsetGreen": 0.02, "offsetBlue": 0.0,
    });
    let output = draw(&mut renderer, Some(effect("colorWheels", &p)));
    let n = |k: &str| p[k].as_f64().unwrap();
    let ch = |w: &str| {
        [
            n(&format!("{w}Red")),
            n(&format!("{w}Green")),
            n(&format!("{w}Blue")),
        ]
    };
    let lift = ch("lift").map(|c| n("liftMaster") + c);
    let gamma = ch("gamma").map(|c| n("gammaMaster") + c);
    let gain = ch("gain").map(|c| n("gainMaster") * c);
    let offset = ch("offset").map(|c| n("offsetMaster") + c);
    let reference = |x: [f64; 3]| {
        let balance = [n("temperature"), -n("tint"), -n("temperature")].map(|v| (v * 0.005).exp2());
        let grey: f64 = (0..3).map(|c| LUMA[c] * balance[c]).sum();
        let g = balance.map(|b| b * n("exposure").exp2() / grey);
        let y: [f64; 3] = std::array::from_fn(|c| e_encode(e_decode(x[c]) * g[c]));
        let k = 1.0 + n("contrast") / 100.0;
        let y = y.map(|v| (v - n("pivot")) * k + n("pivot"));
        let slope: [f64; 3] = std::array::from_fn(|c| gain[c] * (1.0 - lift[c]));
        let off: [f64; 3] = std::array::from_fn(|c| gain[c] * lift[c] + offset[c]);
        cdl(
            y,
            slope,
            off,
            gamma.map(|g| (-g).exp2()),
            n("saturation") / 100.0,
        )
    };
    let d = worst(&input, &output, reference);
    eprintln!("wheels: {d:e} from the reference");
    assert!(d < 2e-5, "{d:e}");
}

/// Monotone cubic Hermite through the points sorted by In (Fritsch-Carlson
/// tangents, end tangents the end secants), straight beyond the ends. Points
/// whose In lie within 1e-6 of the one before are one point: the one written
/// last survives. A single point left is the line of slope 1 through it.
fn monotone(points: &[(f64, f64)], v: f64) -> f64 {
    let mut sorted: Vec<(usize, (f64, f64))> = points.iter().copied().enumerate().collect();
    sorted.sort_by(|a, b| a.1.0.total_cmp(&b.1.0));
    let mut p: Vec<(usize, (f64, f64))> = Vec::new();
    for (i, point) in sorted.iter().enumerate() {
        match p.last_mut() {
            Some(last) if i > 0 && point.1.0 - sorted[i - 1].1.0 < 1e-6 => {
                if point.0 > last.0 {
                    *last = *point;
                }
            }
            _ => p.push(*point),
        }
    }
    let p: Vec<(f64, f64)> = p.into_iter().map(|(_, point)| point).collect();
    let n = p.len();
    if n == 1 {
        return p[0].1 + (v - p[0].0);
    }
    let secant = |i: usize| (p[i + 1].1 - p[i].1) / (p[i + 1].0 - p[i].0);
    let tangent = |i: usize| {
        if i == 0 {
            return secant(0);
        }
        if i == n - 1 {
            return secant(n - 2);
        }
        let (a, b) = (secant(i - 1), secant(i));
        if a * b <= 0.0 {
            return 0.0;
        }
        let (ha, hb) = (p[i].0 - p[i - 1].0, p[i + 1].0 - p[i].0);
        3.0 * (ha + hb) / ((2.0 * hb + ha) / a + (hb + 2.0 * ha) / b)
    };
    if v <= p[0].0 {
        return p[0].1 + tangent(0) * (v - p[0].0);
    }
    if v >= p[n - 1].0 {
        return p[n - 1].1 + tangent(n - 1) * (v - p[n - 1].0);
    }
    let i = (0..n - 1).rev().find(|&i| v >= p[i].0).unwrap();
    let h = p[i + 1].0 - p[i].0;
    let t = (v - p[i].0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * p[i].1
        + (t3 - 2.0 * t2 + t) * h * tangent(i)
        + (-2.0 * t3 + 3.0 * t2) * p[i + 1].1
        + (t3 - t2) * h * tangent(i + 1)
}

fn curve(channel: u32, points: &[(f64, f64)]) -> Value {
    let mut p = json!({"channel": channel, "count": points.len()});
    for (i, (x, y)) in points.iter().enumerate() {
        p[format!("x{i}")] = json!(x);
        p[format!("y{i}")] = json!(y);
    }
    effect_named(&format!("curve{channel}"), "colorCurves", &p)
}

/// A Master curve, then red, green and blue ones, on display-encoded
/// values; points written out of order are read in input order.
#[test]
fn color_curves_are_monotone_cubics_through_their_points() {
    let mut renderer = Renderer::headless().unwrap();
    let input = draw(&mut renderer, None);
    let master = [
        (0.0, 0.05),
        (0.25, 0.2),
        (0.5, 0.55),
        (0.8, 0.85),
        (1.0, 1.0),
    ];
    let red = [(0.0, 0.0), (0.6, 0.7), (1.0, 1.1)];
    // Green's points out of order, as an agent may write them.
    let green = [(1.0, 0.95), (0.0, 0.02), (0.4, 0.35)];
    let blue: Vec<(f64, f64)> = (0..15)
        .map(|i| {
            let x = f64::from(i) / 14.0;
            (x, x * x * (3.0 - 2.0 * x))
        })
        .collect();
    let chain = vec![
        curve(0, &master),
        curve(1, &red),
        curve(2, &green),
        curve(3, &blue),
    ];
    let output = draw_chain(&mut renderer, chain);
    let d = worst(&input, &output, |x| {
        let m = x.map(|v| monotone(&master, v));
        [
            monotone(&red, m[0]),
            monotone(&green, m[1]),
            monotone(&blue, m[2]),
        ]
    });
    eprintln!("curves: {d:e} from the reference");
    assert!(d < 2e-5, "{d:e}");
}

/// Adding a point leaves it at white, on the end point already there: the
/// picture must not change (it bent a straight curve and clipped superwhites).
#[test]
fn an_unused_point_at_white_changes_nothing() {
    let mut renderer = Renderer::headless().unwrap();
    let input = draw(&mut renderer, None);
    for count in [3, 15] {
        let output = draw(
            &mut renderer,
            Some(effect("colorCurves", &json!({"count": count}))),
        );
        let changed = input
            .iter()
            .zip(&output)
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        assert_eq!(changed, 0, "{count} points changed {changed} values");
    }
}

/// Points with the same In are one point, the one written last; at either
/// end and in the middle the curve stays finite and follows that point.
#[test]
fn coincident_points_are_one_point_the_last_written() {
    let mut renderer = Renderer::headless().unwrap();
    let input = draw(&mut renderer, None);
    let cases: [&[(f64, f64)]; 5] = [
        // The new point's Out edited while its In stays at white.
        &[(0.0, 0.0), (1.0, 1.0), (1.0, 0.9)],
        // Moved onto black.
        &[(0.0, 0.0), (1.0, 1.0), (0.0, 0.1)],
        // Two points disagreeing in the middle.
        &[(0.0, 0.0), (0.5, 0.6), (1.0, 1.0), (0.5, 0.4)],
        // Every point on one In: the line of slope 1 through the survivor.
        &[(0.3, 0.2), (0.3, 0.5)],
        // Within a millionth: still one point.
        &[(0.0, 0.0), (0.6, 0.7), (0.6000004, 0.3), (1.0, 1.0)],
    ];
    for points in cases {
        let output = draw(&mut renderer, Some(curve(0, points)));
        let d = worst(&input, &output, |x| x.map(|v| monotone(points, v)));
        eprintln!("{points:?}: {d:e} from the reference");
        assert!(d < 2e-5, "{points:?}: {d:e}");
    }
    // The stated survivor, read off the reference: a straight line to 0.9.
    let end = [(0.0, 0.0), (1.0, 1.0), (1.0, 0.9)];
    assert!((monotone(&end, 1.1) - 0.99).abs() < 1e-12);
    assert!((monotone(&end, 0.5) - 0.45).abs() < 1e-12);
}

/// A keyed point that lands on another at a key is the same coincidence.
#[test]
fn a_keyed_point_landing_on_another_is_one_point() {
    let mut renderer = Renderer::headless().unwrap();
    let input = draw(&mut renderer, None);
    let mut fx = curve(0, &[(0.0, 0.0), (0.5, 0.6), (1.0, 1.0), (0.2, 0.3)]);
    fx["p"]["x3"] = keys(json!(0.5), &[(0.0, json!(0.5)), (1.0, json!(0.9))]);
    let output = draw(&mut renderer, Some(fx));
    let at_key = [(0.0, 0.0), (0.5, 0.6), (1.0, 1.0), (0.5, 0.3)];
    let d = worst(&input, &output, |x| x.map(|v| monotone(&at_key, v)));
    eprintln!("keyed: {d:e} from the reference");
    assert!(d < 2e-5, "{d:e}");
}

/// Values the controls can't hold (an expression's, or written by an older
/// agent) are held to each control's range: the picture stays finite and is
/// the one the range's end gives. 1e39 is a finite number that becomes an
/// infinity at GPU precision.
#[test]
fn values_outside_the_controls_are_held_to_their_range() {
    let mut renderer = Renderer::headless().unwrap();
    let mut bits = |fx: Value| -> Vec<u32> {
        let out = draw(&mut renderer, Some(fx));
        assert!(out.iter().all(|v| v.is_finite()), "a value is not finite");
        out.iter().map(|v| v.to_bits()).collect()
    };
    for (outside, held) in [
        (json!({"exposure": 1024}), json!({"exposure": 8})),
        (json!({"exposure": -1e39}), json!({"exposure": -8})),
        (json!({"gainMaster": 1e39}), json!({"gainMaster": 8})),
        (
            json!({"saturation": 1e39, "contrast": -1e39}),
            json!({"saturation": 200, "contrast": -100}),
        ),
    ] {
        assert_eq!(
            bits(effect("colorWheels", &outside)),
            bits(effect("colorWheels", &held)),
            "{outside}"
        );
    }
    assert_eq!(
        bits(curve(0, &[(0.0, 0.0), (0.5, 1e39), (1.0, 1.0)])),
        bits(curve(0, &[(0.0, 0.0), (0.5, 2.0), (1.0, 1.0)])),
    );
}
