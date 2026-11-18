//! A colour grade on a video layer: Color Wheels (and an imported CDL, which
//! lands on them), Color Curves and Apply Color LUT change a video clip's
//! picture as they change a still's, and the graded video frame equals the
//! same grade on a still of that frame; a camera log reading changes it too.
mod support;
use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use support::doc::*;
use support::ticks;

const W: u32 = 256;
const H: u32 = 144;
const AT: f64 = 0.625;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/video")
        .join(name)
}
fn effect(id: &str, overrides: &Value) -> Value {
    effect_with(id, overrides, json!({}))
}
fn effect_with(id: &str, overrides: &Value, inputs: Value) -> Value {
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
    json!({"id":"grade","type":id,"on":true,"p":params,"inputs":inputs,
           "version":definition.metadata["version"]})
}
/// One layer of `kind` (`video` or `image`) showing `path` over the whole
/// frame, with `fx`; `lut` is the `.cube` an Apply Color LUT reads.
fn document(kind: &str, path: &Path, fx: Vec<Value>, lut: &Path) -> (Value, MapResolver) {
    let format = if kind == "video" { "mp4" } else { "png" };
    let mut source = layer(
        "clip",
        kind,
        json!({ "asset": "a", "fit": "stretch", "w": W, "h": H, "trim": 0, "speed": 1 }),
        f64::from(W) / 2.,
        f64::from(H) / 2.,
        2.,
    );
    source["fx"] = Value::Array(fx);
    let mut doc = project(W, H, 2., no_fill(), vec![source]);
    doc["colorDepth"] = json!(32);
    asset(
        &mut doc,
        "a",
        kind,
        format,
        W,
        H,
        if kind == "video" { 2. } else { 0. },
    );
    asset(&mut doc, "lut", "lut", "cube", 0, 0, 0.);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("a".into(), path.to_path_buf());
    resolver.paths.insert("lut".into(), lut.to_path_buf());
    (doc, resolver)
}
fn render(renderer: &mut Renderer, (doc, resolver): (Value, MapResolver)) -> Vec<f32> {
    let mut doc = load(&doc);
    doc.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&doc).unwrap();
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    let frame = renderer
        .render(&doc, "Ptest-root", ticks(AT), &request, &resolver)
        .unwrap_or_else(|e| panic!("render: {e:?}"));
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer.read_rgba(&frame).unwrap()
}
fn mean_abs(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| f64::from((x - y).abs()))
        .sum::<f64>()
        / a.len() as f64
}
/// `pixels` (display-encoded RGBA, 0..1) as a 16-bit PNG, for a still.
fn write_png(path: &Path, pixels: &[f32]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), W, H);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header().unwrap();
    let data: Vec<u8> = pixels
        .iter()
        .flat_map(|v| ((v.clamp(0., 1.) * 65535.).round() as u16).to_be_bytes())
        .collect();
    writer.write_image_data(&data).unwrap();
}

/// Each per-layer colour control changes a video frame, and the graded
/// frame equals the same grade on a still of the ungraded frame.
#[test]
fn a_graded_video_frame_differs_and_matches_the_grade_on_its_still() {
    let clip = fixture("h264-709-limited.mp4");
    let dir = tempfile::tempdir().unwrap();
    // A 3D LUT that inverts each channel.
    let lut = dir.path().join("invert.cube");
    let cube: String = (0..8)
        .map(|i| {
            format!(
                "{} {} {}\n",
                1 - (i & 1),
                1 - ((i >> 1) & 1),
                1 - ((i >> 2) & 1)
            )
        })
        .collect();
    std::fs::write(&lut, format!("LUT_3D_SIZE 2\n{cube}")).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let plain = render(&mut renderer, document("video", &clip, vec![], &lut));
    let still = dir.path().join("frame.png");
    write_png(&still, &plain);
    let still_plain = render(&mut renderer, document("image", &still, vec![], &lut));
    assert!(
        mean_abs(&plain, &still_plain) < 1e-3,
        "the still is the frame: {}",
        mean_abs(&plain, &still_plain)
    );
    for (name, fx) in [
        (
            "Color Wheels gamma",
            effect(
                "colorWheels",
                &json!({"gammaRed": 0.21, "gammaGreen": -0.10, "gammaBlue": 0.35}),
            ),
        ),
        (
            "Color Wheels lift",
            effect("colorWheels", &json!({"liftMaster": 0.2})),
        ),
        (
            "Color Wheels as a CDL",
            effect(
                "colorWheels",
                &json!({"gainRed": 1.2, "offsetBlue": 0.05, "saturation": 80}),
            ),
        ),
        (
            "Color Curves",
            effect(
                "colorCurves",
                &json!({"count": 3, "x1": 0.5, "y1": 0.7, "x2": 1.0, "y2": 1.0}),
            ),
        ),
        (
            "Apply Color LUT",
            effect_with(
                "applyColorLut",
                &json!({"amount": 100}),
                json!({"lut": {"kind": "asset", "assetId": "lut"}}),
            ),
        ),
    ] {
        let graded = render(
            &mut renderer,
            document("video", &clip, vec![fx.clone()], &lut),
        );
        let still_graded = render(&mut renderer, document("image", &still, vec![fx], &lut));
        let change = mean_abs(&plain, &graded);
        assert!(change > 0.01, "{name} changed the video frame by {change}");
        let apart = mean_abs(&graded, &still_graded);
        assert!(
            apart < 2e-3,
            "{name}: video {apart} from the same grade on its still"
        );
    }
    // A camera log reading of the clip (Interpret Footage) changes its picture.
    let (mut doc, resolver) = document("video", &clip, vec![], &lut);
    doc["assets"]["a"]["interpret"] = json!({"transfer": "sLog3", "primaries": "sGamut3Cine"});
    let read_as_log = render(&mut renderer, (doc, resolver));
    let change = mean_abs(&plain, &read_as_log);
    assert!(change > 0.01, "S-Log3 changed the video frame by {change}");
}

/// A grade is a stack (PM-504): Color Wheels, then a curve, then a LUT on one
/// video clip equal the three applied in turn, each on a still of the frame
/// the one before made; and the order is the stack's (reversed, the picture
/// differs).
#[test]
fn wheels_curve_and_lut_on_one_clip_apply_in_their_order() {
    let named = |id: &str, mut fx: Value| {
        fx["id"] = json!(id);
        fx
    };
    let clip = fixture("h264-709-limited.mp4");
    let dir = tempfile::tempdir().unwrap();
    // A LUT that keeps red, swaps green and blue and darkens a little: not
    // its own inverse, so its place in the stack shows.
    let lut = dir.path().join("swap.cube");
    let cube: String = (0..8)
        .map(|i| {
            let (r, g, b) = (i & 1, (i >> 1) & 1, (i >> 2) & 1);
            format!("{} {} {}\n", 0.9 * r as f64, 0.9 * b as f64, 0.9 * g as f64)
        })
        .collect();
    std::fs::write(&lut, format!("LUT_3D_SIZE 2\n{cube}")).unwrap();
    let wheels = named(
        "wheels",
        effect("colorWheels", &json!({"gammaRed": 0.12, "gammaBlue": -0.08})),
    );
    let curve = named(
        "curve",
        effect(
            "colorCurves",
            &json!({"count": 3, "x1": 0.5, "y1": 0.62, "x2": 1.0, "y2": 1.0}),
        ),
    );
    let look = named(
        "lut",
        effect_with(
            "applyColorLut",
            &json!({"amount": 100}),
            json!({"lut": {"kind": "asset", "assetId": "lut"}}),
        ),
    );
    let mut renderer = Renderer::headless().unwrap();
    let stacked = render(
        &mut renderer,
        document(
            "video",
            &clip,
            vec![wheels.clone(), curve.clone(), look.clone()],
            &lut,
        ),
    );
    // In turn: each block on a still of what the one before it made.
    let mut step = render(&mut renderer, document("video", &clip, vec![wheels.clone()], &lut));
    for (i, fx) in [curve.clone(), look.clone()].into_iter().enumerate() {
        let still = dir.path().join(format!("step{i}.png"));
        write_png(&still, &step);
        step = render(&mut renderer, document("image", &still, vec![fx], &lut));
    }
    let apart = mean_abs(&stacked, &step);
    assert!(apart < 4e-3, "the stack is {apart} from its blocks in turn");
    let plain = render(&mut renderer, document("video", &clip, vec![], &lut));
    let change = mean_abs(&plain, &stacked);
    assert!(change > 0.01, "the stack changed the frame by {change}");
    let reversed = render(
        &mut renderer,
        document("video", &clip, vec![look, curve, wheels], &lut),
    );
    let moved = mean_abs(&stacked, &reversed);
    assert!(moved > 4. * apart.max(1e-3), "reordering moved the picture by {moved}");
}
