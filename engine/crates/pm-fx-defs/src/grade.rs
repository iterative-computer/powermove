//! The grading effects made here and not in main: Color Wheels (an ASC
//! CDL-compatible primary) and Color Curves. Both work on the project's
//! display-encoded values, as a grade in a Rec.709 timeline does, except the
//! primary's white balance and exposure, which are gains on linear light.
//! `grade: true` lists an effect in the Color panel; an extension's effect
//! that says the same is listed there too.
use crate::{
    Definition, Parameter,
    catalog::{EffectShader, MainEffects},
};
use serde_json::{Value, json};

pub const WHEELS: &str = "colorWheels";
pub const CURVES: &str = "colorCurves";
/// The most points one curve holds: with its channel and point count, the 32
/// parameters a definition may have (contract 10).
pub const CURVE_POINTS: usize = 15;
/// The four wheels, in the order a colourist reads them, with each one's
/// neutral value and the limits of its master and of one channel.
pub const WHEEL_NAMES: [(&str, &str, f64, [f64; 2]); 4] = [
    ("lift", "Lift", 0., [-1., 1.]),
    ("gamma", "Gamma", 0., [-2., 2.]),
    ("gain", "Gain", 1., [0., 8.]),
    ("offset", "Offset", 0., [-1., 1.]),
];
/// The channels one Color Curves effect bends, as its Channel choice: a full
/// RGB curves grade is a Master curve followed by one per channel.
pub const CURVE_CHANNELS: [&str; 4] = ["Master", "Red", "Green", "Blue"];

#[allow(clippy::too_many_arguments)]
fn number(
    key: &str,
    label: &str,
    group: &str,
    default: f64,
    range: [f64; 2],
    step: f64,
    unit: &str,
    advanced: bool,
) -> Parameter {
    let mut param = json!({
        "k": key, "label": label, "def": default,
        "min": range[0], "max": range[1], "step": step, "unit": unit, "advanced": advanced,
    });
    if !group.is_empty() {
        param["group"] = json!(group);
    }
    // As many decimals as the step: Pivot reads 0.435, not 0.43.
    param["precision"] = json!((-step.log10()).ceil().max(0.) as u32);
    serde_json::from_value(param).expect("a grading parameter")
}

fn definition(id: &str, label: &str, params: Vec<Parameter>, frag: &str) -> Definition {
    let mut metadata = serde_json::Map::new();
    for (key, value) in [
        ("passes", json!(1)),
        ("rawShader", json!(true)),
        ("viewportSafe", json!(true)),
        ("viewportPadding", json!([])),
        (
            "sourceProcessing",
            json!({"padding": [], "factor": 2, "pixels": 0}),
        ),
        ("version", json!(1)),
        ("grade", json!(true)),
    ] {
        metadata.insert(key.into(), value);
    }
    Definition {
        id: id.into(),
        label: label.into(),
        group: Some("Color".into()),
        params,
        frag: frag.into(),
        metadata,
    }
}

fn wheels() -> Definition {
    let mut params = vec![
        number(
            "temperature",
            "Temperature",
            "Balance",
            0.,
            [-100., 100.],
            1.,
            "",
            false,
        ),
        number("tint", "Tint", "Balance", 0., [-100., 100.], 1., "", false),
        number(
            "exposure",
            "Exposure",
            "Tone",
            0.,
            [-8., 8.],
            0.01,
            "stops",
            false,
        ),
        number(
            "contrast",
            "Contrast",
            "Tone",
            0.,
            [-100., 100.],
            1.,
            "",
            false,
        ),
        number("pivot", "Pivot", "Tone", 0.435, [0., 1.], 0.001, "", false),
        number(
            "saturation",
            "Saturation",
            "Tone",
            100.,
            [0., 200.],
            1.,
            "%",
            false,
        ),
    ];
    for (key, label, neutral, range) in WHEEL_NAMES {
        for channel in ["Master", "Red", "Green", "Blue"] {
            params.push(number(
                &format!("{key}{channel}"),
                channel,
                label,
                neutral,
                range,
                0.001,
                "",
                false,
            ));
        }
    }
    definition(
        WHEELS,
        "Color Wheels",
        params,
        include_str!("color-wheels.glsl"),
    )
}

fn curves() -> Definition {
    let channel: Parameter = serde_json::from_value(json!({
        "k": "channel", "label": "Channel", "def": 0, "min": 0, "max": 3, "step": 1,
        "advanced": false,
        "options": CURVE_CHANNELS.iter().enumerate()
            .map(|(v, label)| json!({"v": v, "label": label})).collect::<Vec<_>>(),
    }))
    .expect("the curve's channel");
    let mut params = vec![
        channel,
        number(
            "count",
            "Points",
            "",
            2.,
            [2., CURVE_POINTS as f64],
            1.,
            "",
            false,
        ),
    ];
    for point in 0..CURVE_POINTS {
        // Two points make the straight line; the rest wait at white.
        let at = if point == 0 { 0. } else { 1. };
        let group = format!("Point {}", point + 1);
        params.push(number(
            &format!("x{point}"),
            "In",
            &group,
            at,
            [0., 1.],
            0.001,
            "",
            point >= 2,
        ));
        params.push(number(
            &format!("y{point}"),
            "Out",
            &group,
            at,
            [-1., 2.],
            0.001,
            "",
            point >= 2,
        ));
    }
    definition(
        CURVES,
        "Color Curves",
        params,
        include_str!("color-curves.glsl"),
    )
}

/// Adds the grading effects after main's, each at revision 1.
pub(crate) fn add(main: &mut MainEffects) {
    for definition in [wheels(), curves()] {
        main.shaders.push(EffectShader {
            id: definition.id.clone(),
            version: 1,
            source: definition.frag.clone(),
        });
        main.effects.push(definition);
    }
}

/// Whether a registered effect definition asks to be listed in the Color
/// panel (`grade: true`).
pub fn is_grade(definition: &Value) -> bool {
    definition.get("grade").and_then(Value::as_bool) == Some(true)
}
