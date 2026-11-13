//! A grade survives the built-in colour effects: light above white and
//! colours outside Rec.709 are not clipped in a 32-bit project, a partly
//! transparent pixel is graded as its straight colour, and an effect saved
//! before a fix draws as it did.
//!
//! Left alone, by definition bounded to 0..1 (their After Effects namesakes
//! are too): Color Balance (HLS), Black & White, the HLS modes of Invert,
//! Shift Channels, Channel Combiner and Blend, Duotone's two-colour map and
//! Find Edges. Levels, Levels (Individual Controls), Arithmetic and Color
//! Profile Converter clip only where their own switch says so.
mod support;

use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;

/// A constant-colour layer with `id` at `version` (the current one for
/// `None`), its parameters the definition's defaults with `overrides`.
fn graded(id: &str, version: Option<u64>, overrides: Value, rgba: [f64; 4]) -> Value {
    let catalog = pm_fx_defs::catalog::main_catalog();
    let definition = match version {
        Some(version) => catalog.get(id, version),
        None => catalog.current(id),
    }
    .unwrap_or_else(|| panic!("{id}"));
    let mut params = serde_json::Map::new();
    for (key, value) in definition.defaults() {
        params.insert(
            key.clone(),
            ch(overrides.get(&key).cloned().unwrap_or(value)),
        );
    }
    for key in overrides.as_object().unwrap().keys() {
        assert!(params.contains_key(key), "{id} has no parameter {key}");
    }
    let code = format!(
        "void main(){{fragColor=vec4({:?},{:?},{:?},{:?});}}",
        rgba[0], rgba[1], rgba[2], rgba[3]
    );
    let mut owner = layer(
        "owner",
        "shader",
        json!({"w":16,"h":16,"code":code,"uniforms":{}}),
        0.,
        0.,
        1.,
    );
    owner["fx"] = json!([{
        "id":"effect","type":id,"on":true,"p":params,"inputs":{},
        "version": definition.metadata.get("version").cloned().unwrap_or(json!(1)),
    }]);
    let mut value = project(16, 16, 1., no_fill(), vec![owner]);
    value["colorDepth"] = json!(32);
    value
}

fn pixel(renderer: &mut Renderer, value: &Value) -> [f64; 4] {
    let mut doc = load(value);
    doc.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&doc).unwrap();
    let frame = renderer
        .render(
            &doc,
            "Ptest-root",
            Ticks::ZERO,
            &RenderRequest::sized(Size::new(16, 16), Precision::Float32),
            &MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let pixels = renderer.read_rgba(&frame).unwrap();
    // The middle pixel: spatial effects see only the constant around it.
    let at = (8 * 16 + 8) * 4;
    std::array::from_fn(|c| f64::from(pixels[at + c]))
}

/// Light two and a half times white, and a colour outside Rec.709.
const WIDE: [f64; 4] = [2.5, 0.5, -0.125, 1.0];

#[test]
fn a_neutral_colour_effect_keeps_light_above_white_and_colours_outside_rec709() {
    let mut renderer = Renderer::headless().unwrap();
    for (id, neutral) in [
        ("color", json!({})),
        // Levels clips where its own switches say so; they start on.
        ("levels", json!({"clipBlack":false,"clipWhite":false})),
        (
            "levelsIndividual",
            json!({"clipBlack":false,"clipWhite":false}),
        ),
        ("exposure", json!({})),
        ("channelMixer", json!({})),
        ("brightnessContrast", json!({})),
        ("colorBalanceHLS", json!({})),
        ("colorProfileConverter", json!({})),
        ("shiftChannels", json!({})),
        ("applyColorLut", json!({})),
        ("hdrCompander", json!({"gain":1,"gamma":1})),
        ("sharpen", json!({"amount":0})),
        ("grain", json!({"amount":0})),
        ("duotone", json!({"amount":0})),
        ("tint", json!({"amountToTint":0})),
        ("tritone", json!({"blendWithOriginal":100})),
        ("invert", json!({"blendWithOriginal":100})),
    ] {
        let out = pixel(&mut renderer, &graded(id, None, neutral, WIDE));
        for c in 0..4 {
            assert!(
                (out[c] - WIDE[c]).abs() < 1e-5,
                "{id} changed {WIDE:?} to {out:?}"
            );
        }
    }
}

/// The shipped shaders clipped; an effect saved with one still does.
#[test]
fn an_effect_saved_before_the_fix_clips_as_it_did() {
    let mut renderer = Renderer::headless().unwrap();
    for (id, neutral, clipped) in [
        ("color", json!({}), [2.5, 0.5, 0.0]),
        ("channelMixer", json!({}), [1.0, 0.5, 0.0]),
        ("sharpen", json!({"amount":0}), [2.5, 0.5, 0.0]),
        ("grain", json!({"amount":0}), [2.5, 0.5, 0.0]),
    ] {
        let out = pixel(&mut renderer, &graded(id, Some(1), neutral, WIDE));
        for c in 0..3 {
            assert!(
                (out[c] - clipped[c]).abs() < 1e-5,
                "{id} revision 1: {out:?}"
            );
        }
    }
    // Revision 1's exposure doubled the encoded value and stopped at 8.
    let old = graded(
        "color",
        Some(1),
        json!({"exposure":1}),
        [6.0, 0.5, 0.18, 1.0],
    );
    let out = pixel(&mut renderer, &old);
    for (c, expected) in [8.0, 1.0, 0.36].into_iter().enumerate() {
        assert!((out[c] - expected).abs() < 1e-5, "{out:?}");
    }
}

#[test]
fn a_stop_of_exposure_doubles_the_light_and_is_not_clipped() {
    use pm_reference::color::Transfer;
    let mut renderer = Renderer::headless().unwrap();
    let input = [6.0, 0.5, 0.18, 1.0];
    for (stops, gain) in [(1.0, 2.0), (-1.0, 0.5), (2.5, 2f64.powf(2.5))] {
        let out = pixel(
            &mut renderer,
            &graded("color", None, json!({"exposure":stops}), input),
        );
        for c in 0..3 {
            let expected = Transfer::Srgb.from_linear(Transfer::Srgb.to_linear(input[c]) * gain);
            assert!(
                (out[c] - expected).abs() < 1e-4 * expected.max(1.0),
                "{stops} stops: {out:?}, channel {c} should be {expected}"
            );
        }
    }
}

/// Color's hue turns a colour about its own grey, so a colour outside
/// Rec.709 or above white keeps what makes it so: two half turns give the
/// colour back, and a colour inside 0..1 turns as the shipped effect turned
/// it.
#[test]
fn hue_keeps_colour_outside_rec709_and_above_white() {
    let mut renderer = Renderer::headless().unwrap();
    for colour in [
        [-0.125, 0.5, 0.25, 1.0],
        [2.5, 0.5, -0.125, 1.0],
        [0.25, 1.75, 4.0, 1.0],
        [-0.5, -0.25, -0.125, 1.0],
    ] {
        let mut twice = graded("color", None, json!({"hue":180}), colour);
        let mut second = twice["layers"][0]["fx"][0].clone();
        second["id"] = json!("again");
        twice["layers"][0]["fx"]
            .as_array_mut()
            .unwrap()
            .push(second);
        let out = pixel(&mut renderer, &twice);
        for c in 0..4 {
            assert!(
                (out[c] - colour[c]).abs() < 2e-5 * colour[c].abs().max(1.0),
                "two half turns of {colour:?} gave {out:?}"
            );
        }
        // One half turn keeps the colour's smallest and largest channel.
        let once = pixel(
            &mut renderer,
            &graded("color", None, json!({"hue":180}), colour),
        );
        let ends = |v: &[f64]| {
            (
                v[..3].iter().copied().fold(f64::INFINITY, f64::min),
                v[..3].iter().copied().fold(f64::NEG_INFINITY, f64::max),
            )
        };
        let (was, is) = (ends(&colour), ends(&once));
        assert!(
            (was.0 - is.0).abs() < 2e-5 && (was.1 - is.1).abs() < 2e-5,
            "a half turn of {colour:?} gave {once:?}"
        );
        assert!(
            (0..3).any(|c| (once[c] - colour[c]).abs() > 0.05),
            "{once:?}"
        );
    }
    for degrees in [-150, -60, 45, 90, 180] {
        let colour = [0.8, 0.4, 0.2, 1.0];
        let hue = json!({"hue":degrees});
        let now = pixel(&mut renderer, &graded("color", None, hue.clone(), colour));
        let shipped = pixel(&mut renderer, &graded("color", Some(1), hue, colour));
        for c in 0..3 {
            assert!(
                (now[c] - shipped[c]).abs() < 2e-5,
                "{degrees}°: {now:?}, the shipped effect {shipped:?}"
            );
        }
    }
}

/// A pixel with almost no coverage is graded like any other: its straight
/// colour takes the grade the opaque pixel takes.
#[test]
fn color_grades_a_pixel_with_faint_coverage() {
    let mut renderer = Renderer::headless().unwrap();
    let colour = [0.8, 0.4, 0.2];
    let grade = json!({"exposure":1,"saturation":30});
    let opaque = pixel(
        &mut renderer,
        &graded(
            "color",
            None,
            grade.clone(),
            [colour[0], colour[1], colour[2], 1.0],
        ),
    );
    for alpha in [0.0004, 0.00001] {
        let faint = pixel(
            &mut renderer,
            &graded(
                "color",
                None,
                grade.clone(),
                [
                    colour[0] * alpha,
                    colour[1] * alpha,
                    colour[2] * alpha,
                    alpha,
                ],
            ),
        );
        for c in 0..3 {
            assert!(
                (faint[c] / faint[3] - opaque[c]).abs() < 1e-3,
                "alpha {alpha}: straight {:?}, opaque {opaque:?}",
                [
                    faint[0] / faint[3],
                    faint[1] / faint[3],
                    faint[2] / faint[3]
                ]
            );
        }
    }
}

/// Unpremultiply, operate, premultiply: a half-transparent pixel leaves each
/// effect as the opaque pixel's result at half coverage, so soft edges keep
/// their colour (no dark or bright fringe).
#[test]
fn colour_effects_grade_the_straight_colour_of_a_transparent_pixel() {
    let mut renderer = Renderer::headless().unwrap();
    let colour = [0.8, 0.4, 0.2];
    for (id, overrides) in [
        (
            "color",
            json!({"exposure":0.5,"contrast":20,"saturation":30,"hue":40,"temperature":10}),
        ),
        ("levels", json!({"gamma":1.4,"inBlack":0.1})),
        ("levelsIndividual", json!({"redGamma":1.3})),
        ("exposure", json!({"exposure":1,"offset":0.05,"gamma":1.2})),
        ("channelMixer", json!({"redGreen":20,"blueConst":5})),
        ("brightnessContrast", json!({"brightness":20,"contrast":30})),
        (
            "colorBalanceHLS",
            json!({"hue":30,"lightness":10,"saturation":20}),
        ),
        ("colorProfileConverter", json!({"inputProfile":1})),
        ("hdrCompander", json!({})),
        ("duotone", json!({})),
        ("tint", json!({})),
        ("tritone", json!({})),
        ("invert", json!({})),
        ("posterize", json!({})),
        ("blackWhite", json!({})),
        ("grain", json!({"amount":50})),
    ] {
        let opaque = pixel(
            &mut renderer,
            &graded(
                id,
                None,
                overrides.clone(),
                [colour[0], colour[1], colour[2], 1.0],
            ),
        );
        let half = pixel(
            &mut renderer,
            &graded(
                id,
                None,
                overrides,
                [colour[0] / 2., colour[1] / 2., colour[2] / 2., 0.5],
            ),
        );
        assert!((half[3] - 0.5).abs() < 1e-6, "{id}: alpha {}", half[3]);
        assert!(
            (0..3).any(|c| (opaque[c] - colour[c]).abs() > 1e-3),
            "{id}: these settings change the colour"
        );
        for c in 0..3 {
            assert!(
                (half[c] / half[3] - opaque[c]).abs() < 2e-5,
                "{id}: half-transparent {half:?}, opaque {opaque:?}"
            );
        }
    }
}

/// A grade belongs on an adjustment layer or a group. An effect that reads
/// its layer's rectangle (Levels does) sees the composition on an adjustment
/// layer and the members' rectangle on a group, and gives what it gives on
/// the layer itself. (A text layer: `text_colors`.)
#[test]
fn an_effect_that_reads_its_layers_rectangle_grades_on_an_adjustment_layer_and_a_group() {
    let mut renderer = Renderer::headless().unwrap();
    let colour = [0.6, 0.3, 0.15, 1.0];
    for (id, overrides) in [
        (
            "levels",
            json!({"gamma":1.25,"outBlack":0.04,"outWhite":0.93}),
        ),
        ("levelsIndividual", json!({"redGamma":1.3})),
        ("invert", json!({})),
    ] {
        let direct = graded(id, None, overrides.clone(), colour);
        let expected = pixel(&mut renderer, &direct);
        assert!(
            (0..3).any(|c| (expected[c] - colour[c]).abs() > 1e-3),
            "{id}: the grade changes the colour"
        );
        for holder in ["adjustment", "group"] {
            let mut over = direct.clone();
            let fx = over["layers"][0]["fx"].take();
            over["layers"][0]["fx"] = json!([]);
            let mut grade = layer("grade", holder, json!({}), 0., 0., 1.);
            grade["fx"] = fx;
            if holder == "group" {
                over["layers"][0]["group"] = json!("grade");
            }
            over["layers"].as_array_mut().unwrap().insert(0, grade);
            let out = pixel(&mut renderer, &over);
            for c in 0..4 {
                assert!(
                    (out[c] - expected[c]).abs() < 1e-6,
                    "{id} on {holder}: {out:?}, on the layer {expected:?}"
                );
            }
        }
    }
}
