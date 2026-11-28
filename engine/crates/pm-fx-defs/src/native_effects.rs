//! Native built-in effects added or upgraded beyond `main-effects.json`:
//! - **Fill** (`fill`): After Effects-style solid color fill with Invert and Opacity.
//! - **Noise** (`noise`): Digital sensor/procedural noise (monochrome or color,
//!   configurable scale, clip toggle, animate and seed).
//! - **Grain** (`grain`, v3): Photographic organic film grain replacing the
//!   legacy "Film Grain" effect while preserving v1 and v2 in `legacy` so saved
//!   projects render identically.
use crate::{
    Definition, Parameter,
    catalog::{EffectShader, MainEffects, effect_version},
};
use serde_json::{Value, json};

pub const FILL: &str = "fill";
pub const NOISE: &str = "noise";
pub const GRAIN: &str = "grain";

fn number(
    key: &str,
    label: &str,
    default: f64,
    range: [f64; 2],
    step: f64,
    unit: &str,
) -> Parameter {
    serde_json::from_value(json!({
        "k": key,
        "label": label,
        "def": default,
        "min": range[0],
        "max": range[1],
        "step": step,
        "unit": unit,
        "advanced": false,
    }))
    .expect("a native effect number parameter")
}

fn color(key: &str, label: &str, default: &str) -> Parameter {
    serde_json::from_value(json!({
        "k": key,
        "label": label,
        "type": "color",
        "def": default,
        "advanced": false,
    }))
    .expect("a native effect color parameter")
}

fn toggle(key: &str, label: &str, default: bool) -> Parameter {
    serde_json::from_value(json!({
        "k": key,
        "label": label,
        "type": "toggle",
        "def": default,
        "advanced": false,
    }))
    .expect("a native effect toggle parameter")
}

fn base_metadata(version: u64) -> serde_json::Map<String, Value> {
    let mut metadata = serde_json::Map::new();
    metadata.insert("version".into(), Value::from(version));
    metadata.insert("passes".into(), json!(1));
    metadata.insert("rawShader".into(), json!(true));
    metadata.insert("viewportSafe".into(), json!(true));
    metadata.insert("viewportPadding".into(), json!([]));
    metadata.insert(
        "sourceProcessing".into(),
        json!({"padding": [], "factor": 2, "pixels": 0}),
    );
    metadata
}

const FILL_FRAG: &str = r#"#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform vec3 u_color;
uniform float u_invert;
uniform float u_opacity;
out vec4 o;
void main() {
  vec4 c = texture(u_tex, v_st);
  float op = clamp(u_opacity / 100.0, 0.0, 1.0);
  if (u_invert > 0.5) {
    float invA = clamp(1.0 - c.a, 0.0, 1.0);
    vec4 filled = vec4(u_color * invA, invA);
    o = mix(c, filled, op);
  } else {
    vec4 filled = vec4(u_color * c.a, c.a);
    o = mix(c, filled, op);
  }
}
"#;

const NOISE_FRAG: &str = r#"#version 300 es
precision highp float;
in vec2 v_st;
in vec2 v_px;
uniform sampler2D u_tex;
uniform float u_time;
uniform float u_amount;
uniform float u_colorNoise;
uniform float u_scale;
uniform float u_clipResult;
uniform float u_animate;
uniform float u_seed;
out vec4 o;

vec3 hash33(vec3 p) {
  p = fract(p * vec3(0.1031, 0.1030, 0.0973));
  p += dot(p, p.yxz + 33.33);
  return fract((p.xxy + p.yxx) * p.zyx);
}

void main() {
  vec4 c = texture(u_tex, v_st);
  if (c.a <= 0.0) {
    o = c;
    return;
  }
  float amt = max(u_amount, 0.0) / 100.0;
  if (amt <= 0.0) {
    o = c;
    return;
  }
  float cellSize = max(u_scale, 0.5);
  vec2 cell = floor(v_px / cellSize);
  float t = u_animate > 0.5 ? floor(u_time * 60.0 + 0.5) : 0.0;
  float seed = u_seed * 13.7 + t * 1.6180339;
  // Triangular zero-mean distribution in [-1, 1] without sine-hash banding.
  vec3 n1 = hash33(vec3(cell, seed));
  vec3 n2 = hash33(vec3(cell + vec2(41.0, 29.0), seed + 17.3));
  vec3 tri = n1 + n2 - 1.0;
  vec3 delta = (u_colorNoise > 0.5 ? tri : vec3(tri.x)) * amt;
  vec3 rgb = c.rgb / c.a + delta;
  if (u_clipResult > 0.5) {
    rgb = clamp(rgb, 0.0, 1.0);
  }
  o = vec4(rgb * c.a, c.a);
}
"#;

const GRAIN_FRAG: &str = r#"#version 300 es
precision highp float;
in vec2 v_st;
in vec2 v_px;
uniform sampler2D u_tex;
uniform float u_time;
uniform float u_amount;
uniform float u_size;
uniform float u_softness;
uniform float u_saturation;
uniform float u_shadows;
uniform float u_highlights;
uniform float u_animate;
uniform float u_seed;
out vec4 o;

float hash13(vec3 p) {
  p = fract(p * 0.1031);
  p += dot(p, p.zyx + 31.32);
  return fract((p.x + p.y) * p.z);
}

// Smooth organic value noise with zero mean in [-1, 1].
float smoothGrain(vec2 p, float z) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float a = hash13(vec3(i, z));
  float b = hash13(vec3(i + vec2(1.0, 0.0), z));
  float c = hash13(vec3(i + vec2(0.0, 1.0), z));
  float d = hash13(vec3(i + vec2(1.0, 1.0), z));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y) * 2.0 - 1.0;
}

// Two-octave organic film grain combining fine halide crystals and soft clumping.
float filmLayer(vec2 coord, float z, float soft) {
  float fine = smoothGrain(coord, z);
  float clump = smoothGrain(coord * 0.48 + vec2(19.3, 7.1), z + 11.7);
  float crisp = hash13(vec3(floor(coord), z + 5.3)) * 2.0 - 1.0;
  float crystal = mix(crisp, fine, 0.35 + 0.65 * soft);
  return mix(crystal, (crystal * 0.6 + clump * 0.4), 0.35 + 0.45 * soft);
}

void main() {
  vec4 c = texture(u_tex, v_st);
  if (c.a <= 0.0) {
    o = c;
    return;
  }
  float amt = max(u_amount, 0.0) / 100.0;
  if (amt <= 0.0) {
    o = c;
    return;
  }
  vec3 rgb = c.rgb / c.a;
  float luma = clamp(dot(rgb, vec3(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
  // Photographic response: strongest in midtones, tunable rolloff in shadows & highlights.
  float shWeight = clamp(u_shadows / 100.0, 0.0, 1.0);
  float hiWeight = clamp(u_highlights / 100.0, 0.0, 1.0);
  float lowMask = mix(shWeight, 1.0, smoothstep(0.0, 0.35, luma));
  float highMask = mix(1.0, hiWeight, smoothstep(0.65, 1.0, luma));
  float response = lowMask * highMask;

  float grainSize = max(u_size, 0.4);
  float soft = clamp(u_softness / 100.0, 0.0, 1.0);
  float sat = clamp(u_saturation / 100.0, 0.0, 1.0);
  float t = u_animate > 0.5 ? floor(u_time * 24.0 + 0.5) : 0.0;
  float z = u_seed * 7.31 + t * 2.39996;
  vec2 coord = v_px / grainSize;

  float mono = filmLayer(coord, z, soft);
  vec3 chroma = vec3(
    filmLayer(coord + vec2(0.37, 0.11), z + 3.1, soft),
    filmLayer(coord + vec2(-0.23, 0.41), z + 6.7, soft),
    filmLayer(coord + vec2(0.19, -0.33), z + 9.9, soft)
  );
  vec3 grain = mix(vec3(mono), chroma, sat);
  // Scale amplitude so 15% gives subtle 35mm texture and 100% is strong 16mm push.
  vec3 outRgb = rgb + grain * (amt * 0.55 * response);
  o = vec4(outRgb * c.a, c.a);
}
"#;

pub fn fill() -> Definition {
    Definition {
        id: FILL.into(),
        label: "Fill".into(),
        group: Some("Generate".into()),
        params: vec![
            color("color", "Color", "#FF0000"),
            toggle("invert", "Invert", false),
            number("opacity", "Opacity", 100., [0., 100.], 1., "%"),
        ],
        frag: FILL_FRAG.into(),
        metadata: base_metadata(1),
    }
}

pub fn noise() -> Definition {
    Definition {
        id: NOISE.into(),
        label: "Noise".into(),
        group: Some("Stylize".into()),
        params: vec![
            number("amount", "Amount of Noise", 12., [0., 100.], 0.5, "%"),
            toggle("colorNoise", "Use Color Noise", true),
            number("scale", "Scale", 1., [0.5, 16.], 0.1, "px"),
            toggle("clipResult", "Clip Result Values", true),
            toggle("animate", "Animate", true),
            number("seed", "Seed", 0., [0., 1000.], 1., ""),
        ],
        frag: NOISE_FRAG.into(),
        metadata: base_metadata(1),
    }
}

/// Upgrades `"grain"` from legacy `"Film Grain"` (v1/v2) to v3 `"Grain"`,
/// keeping v1 and v2 intact in `legacy` so saved projects keep their exact look.
fn revise_grain(main: &mut MainEffects) {
    let revision = main
        .effects
        .iter_mut()
        .find(|e| e.id == GRAIN)
        .expect("pinned grain effect");
    let version = effect_version(revision.metadata.get("version")).expect("valid grain version");
    let mut history = revision.clone();
    history
        .metadata
        .insert("version".into(), Value::from(version));
    history.metadata.remove("legacy");

    let next_version = version + 1;
    revision.label = "Grain".into();
    revision.group = Some("Stylize".into());
    revision.params = vec![
        number("amount", "Amount", 15., [0., 100.], 0.5, "%"),
        number("size", "Size", 1.2, [0.4, 6.], 0.05, ""),
        number("softness", "Softness", 35., [0., 100.], 1., "%"),
        number("saturation", "Color Saturation", 25., [0., 100.], 1., "%"),
        number("shadows", "Shadows", 50., [0., 100.], 1., "%"),
        number("highlights", "Highlights", 40., [0., 100.], 1., "%"),
        toggle("animate", "Animate", true),
        number("seed", "Seed", 0., [0., 1000.], 1., ""),
    ];
    revision.frag = GRAIN_FRAG.into();
    revision
        .metadata
        .insert("version".into(), Value::from(next_version));
    let legacy = revision
        .metadata
        .entry("legacy")
        .or_insert_with(|| Value::Array(vec![]));
    legacy
        .as_array_mut()
        .expect("grain legacy list")
        .push(serde_json::to_value(history).expect("serializable grain history"));

    main.shaders.push(EffectShader {
        id: GRAIN.into(),
        version: next_version,
        source: GRAIN_FRAG.into(),
    });
}

pub(crate) fn add(main: &mut MainEffects) {
    revise_grain(main);
    for definition in [fill(), noise()] {
        main.shaders.push(EffectShader {
            id: definition.id.clone(),
            version: 1,
            source: definition.frag.clone(),
        });
        main.effects.push(definition);
    }
}
