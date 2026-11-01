//! Shipped caption style semantics. Existing authored presets are data; this
//! module introduces no native visual design choices.
use crate::model::{clamp, finite};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionStyle {
    pub preset: String,
    pub font: String,
    pub weight: f64,
    pub italic: bool,
    pub size: f64,
    pub tracking: f64,
    pub leading: f64,
    pub fill: String,
    pub stroke: f64,
    pub stroke_color: String,
    #[serde(rename = "box")]
    pub boxed: bool,
    pub box_color: String,
    pub box_opacity: f64,
    pub box_padding: f64,
    pub box_radius: f64,
    pub placement: String,
    pub align: String,
    pub offset_x: f64,
    pub offset_y: f64,
    pub safe_margin: f64,
    pub max_width: f64,
    pub max_lines: f64,
    pub text_case: String,
    pub highlight: bool,
    pub highlight_color: String,
    pub highlight_scale: f64,
    pub word_animation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptionPreset {
    pub id: String,
    pub label: String,
    pub description: String,
    pub style: Value,
}
#[derive(Deserialize)]
struct Styles {
    default: CaptionStyle,
    presets: Vec<CaptionPreset>,
}
fn styles() -> &'static Styles {
    static STYLES: OnceLock<Styles> = OnceLock::new();
    STYLES.get_or_init(|| {
        serde_json::from_str(include_str!("styles.json")).expect("generated caption styles")
    })
}
impl Default for CaptionStyle {
    fn default() -> Self {
        styles().default.clone()
    }
}
pub fn caption_presets() -> &'static [CaptionPreset] {
    &styles().presets
}
pub fn caption_preset(id: &str) -> Option<&'static CaptionPreset> {
    caption_presets().iter().find(|p| p.id == id)
}
pub fn preset_style(id: &str, composition_height: f64) -> CaptionStyle {
    let preset = caption_preset(id).unwrap_or(&caption_presets()[0]);
    let mut v = serde_json::to_value(CaptionStyle::default()).unwrap();
    for (k, val) in preset.style.as_object().unwrap() {
        v[k] = val.clone();
    }
    v["preset"] = Value::from(preset.id.clone());
    let scale = if composition_height > 0.0 && composition_height.is_finite() {
        composition_height / 1080.0
    } else {
        1.0
    };
    for key in [
        "size",
        "stroke",
        "boxPadding",
        "boxRadius",
        "offsetX",
        "offsetY",
    ] {
        v[key] = Value::from((v[key].as_f64().unwrap() * scale * 100.0 + 0.5).floor() / 100.0);
    }
    serde_json::from_value(v).unwrap()
}
pub fn normalize_caption_style(raw: &Value) -> CaptionStyle {
    let mut v = serde_json::to_value(CaptionStyle::default()).unwrap();
    v["weight"] = Value::from(clamp(
        (finite(raw.get("weight"), v["weight"].as_f64().unwrap()) + 0.5).floor(),
        100.0,
        1000.0,
    ));
    v["size"] = Value::from(clamp(
        finite(raw.get("size"), v["size"].as_f64().unwrap()),
        4.0,
        1000.0,
    ));
    v["tracking"] = Value::from(clamp(
        finite(raw.get("tracking"), v["tracking"].as_f64().unwrap()),
        -50.0,
        200.0,
    ));
    v["leading"] = Value::from(clamp(
        finite(raw.get("leading"), v["leading"].as_f64().unwrap()),
        0.5,
        4.0,
    ));
    v["stroke"] = Value::from(clamp(
        finite(raw.get("stroke"), v["stroke"].as_f64().unwrap()),
        0.0,
        100.0,
    ));
    v["boxOpacity"] = Value::from(clamp(
        finite(raw.get("boxOpacity"), v["boxOpacity"].as_f64().unwrap()),
        0.0,
        100.0,
    ));
    v["boxPadding"] = Value::from(clamp(
        finite(raw.get("boxPadding"), v["boxPadding"].as_f64().unwrap()),
        0.0,
        400.0,
    ));
    v["boxRadius"] = Value::from(clamp(
        finite(raw.get("boxRadius"), v["boxRadius"].as_f64().unwrap()),
        0.0,
        400.0,
    ));
    v["offsetX"] = Value::from(clamp(
        finite(raw.get("offsetX"), v["offsetX"].as_f64().unwrap()),
        -100000.0,
        100000.0,
    ));
    v["offsetY"] = Value::from(clamp(
        finite(raw.get("offsetY"), v["offsetY"].as_f64().unwrap()),
        -100000.0,
        100000.0,
    ));
    v["safeMargin"] = Value::from(clamp(
        finite(raw.get("safeMargin"), v["safeMargin"].as_f64().unwrap()),
        0.0,
        45.0,
    ));
    v["maxWidth"] = Value::from(clamp(
        finite(raw.get("maxWidth"), v["maxWidth"].as_f64().unwrap()),
        10.0,
        100.0,
    ));
    v["maxLines"] = Value::from(clamp(
        (finite(raw.get("maxLines"), v["maxLines"].as_f64().unwrap()) + 0.5).floor(),
        1.0,
        6.0,
    ));
    v["highlightScale"] = Value::from(clamp(
        finite(
            raw.get("highlightScale"),
            v["highlightScale"].as_f64().unwrap(),
        ),
        50.0,
        300.0,
    ));
    v["italic"] = Value::Bool(raw.get("italic").and_then(Value::as_bool) == Some(true));
    v["box"] = Value::Bool(raw.get("box").and_then(Value::as_bool) == Some(true));
    v["highlight"] = Value::Bool(raw.get("highlight").and_then(Value::as_bool) == Some(true));
    if let Some(s) = raw
        .get("fill")
        .and_then(Value::as_str)
        .filter(|s| valid_color(s))
    {
        v["fill"] = Value::from(s.to_ascii_uppercase());
    }
    if let Some(s) = raw
        .get("strokeColor")
        .and_then(Value::as_str)
        .filter(|s| valid_color(s))
    {
        v["strokeColor"] = Value::from(s.to_ascii_uppercase());
    }
    if let Some(s) = raw
        .get("boxColor")
        .and_then(Value::as_str)
        .filter(|s| valid_color(s))
    {
        v["boxColor"] = Value::from(s.to_ascii_uppercase());
    }
    if let Some(s) = raw
        .get("highlightColor")
        .and_then(Value::as_str)
        .filter(|s| valid_color(s))
    {
        v["highlightColor"] = Value::from(s.to_ascii_uppercase());
    }
    if let Some(s) = raw
        .get("placement")
        .and_then(Value::as_str)
        .filter(|s| ["bottom", "middle", "top"].contains(s))
    {
        v["placement"] = Value::from(s);
    }
    if let Some(s) = raw
        .get("align")
        .and_then(Value::as_str)
        .filter(|s| ["left", "center", "right"].contains(s))
    {
        v["align"] = Value::from(s);
    }
    if let Some(s) = raw
        .get("textCase")
        .and_then(Value::as_str)
        .filter(|s| ["original", "upper", "lower"].contains(s))
    {
        v["textCase"] = Value::from(s);
    }
    if let Some(s) = raw
        .get("wordAnimation")
        .and_then(Value::as_str)
        .filter(|s| ["none", "pop", "fade"].contains(s))
    {
        v["wordAnimation"] = Value::from(s);
    }
    if let Some(s) = raw
        .get("preset")
        .and_then(Value::as_str)
        .filter(|s| *s == "custom" || caption_preset(s).is_some())
    {
        v["preset"] = Value::from(s);
    }
    if let Some(s) = raw
        .get("font")
        .and_then(Value::as_str)
        .filter(|s| !crate::text::trim(s).is_empty())
    {
        v["font"] = Value::from(s);
    }
    serde_json::from_value(v).unwrap()
}
fn valid_color(s: &str) -> bool {
    (s.len() == 7 || s.len() == 9)
        && s.starts_with('#')
        && s[1..].bytes().all(|c| c.is_ascii_hexdigit())
}
pub fn patch_style(style: &CaptionStyle, patch: &Value, composition_height: f64) -> CaptionStyle {
    let position = ["placement", "offsetX", "offsetY"];
    let mut current = serde_json::to_value(style).unwrap();
    if let Some(preset) = patch
        .get("preset")
        .and_then(Value::as_str)
        .filter(|id| *id != style.preset && caption_preset(id).is_some())
    {
        let reference =
            serde_json::to_value(preset_style(&style.preset, composition_height)).unwrap();
        let moved = position.iter().any(|k| current[k] != reference[k]);
        let mut next = serde_json::to_value(preset_style(preset, composition_height)).unwrap();
        if moved {
            for key in position {
                next[key] = current[key].clone();
            }
        }
        if let Some(p) = patch.as_object() {
            for (k, v) in p {
                next[k] = v.clone();
            }
        }
        return normalize_caption_style(&next);
    }
    if let Some(p) = patch.as_object() {
        for (k, v) in p {
            current[k] = v.clone();
        }
    }
    let mut next = normalize_caption_style(&current);
    if patch.get("preset").is_none() && next.preset != "custom" {
        let reference =
            serde_json::to_value(preset_style(&next.preset, composition_height)).unwrap();
        let normalized = serde_json::to_value(&next).unwrap();
        if patch.as_object().is_some_and(|p| {
            p.keys().any(|k| {
                k != "preset" && !position.contains(&k.as_str()) && reference[k] != normalized[k]
            })
        }) {
            next.preset = "custom".into();
        }
    }
    next
}
