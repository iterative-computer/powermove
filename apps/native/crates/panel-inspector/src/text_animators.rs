//! Add Text Animator presets (`extensions/inspector/text-animators.ts`).
use crate::{Inspector, InspectorError};
use pm_kernel::{Caller, Request, TextCommand};
use pm_model::{Map, Value};

struct Preset {
    id: &'static str,
    label: &'static str,
    group: &'static str,
    mode: &'static str,
    unit: &'static str,
    easing: Option<&'static str>,
    settings: &'static [(&'static str, f64)],
    /// Channels that scale with the font size use a factor over it.
    channels: &'static [(&'static str, f64, bool)],
}
const fn preset(
    id: &'static str,
    label: &'static str,
    group: &'static str,
    mode: &'static str,
    unit: &'static str,
    easing: Option<&'static str>,
    settings: &'static [(&'static str, f64)],
    channels: &'static [(&'static str, f64, bool)],
) -> Preset {
    Preset {
        id,
        label,
        group,
        mode,
        unit,
        easing,
        settings,
        channels,
    }
}
const STAGGER: &str = "stagger";
const CHARACTERS: &str = "characters";
const PRESETS: [Preset; 13] = [
    preset(
        "fade",
        "Fade",
        "in",
        STAGGER,
        CHARACTERS,
        Some("cubicOut"),
        &[("duration", 0.5), ("stagger", 0.03)],
        &[("opacity", 0., false)],
    ),
    preset(
        "rise",
        "Rise",
        "in",
        STAGGER,
        CHARACTERS,
        Some("expoOut"),
        &[("duration", 0.8), ("stagger", 0.03)],
        &[("opacity", 0., false), ("y", 0.6, true)],
    ),
    preset(
        "rise-words",
        "Rise by word",
        "in",
        STAGGER,
        "words",
        Some("expoOut"),
        &[("duration", 0.9), ("stagger", 0.08)],
        &[("opacity", 0., false), ("y", 0.8, true)],
    ),
    preset(
        "pop",
        "Pop",
        "in",
        STAGGER,
        CHARACTERS,
        Some("backOut"),
        &[("duration", 0.5), ("stagger", 0.035)],
        &[("opacity", 0., false), ("scale", 0., false)],
    ),
    preset(
        "drop",
        "Spring drop",
        "in",
        STAGGER,
        CHARACTERS,
        Some("spring"),
        &[("duration", 1.), ("stagger", 0.04)],
        &[("opacity", 0., false), ("y", -0.9, true)],
    ),
    preset(
        "tumble",
        "Tumble",
        "in",
        STAGGER,
        CHARACTERS,
        Some("backOut"),
        &[("duration", 0.7), ("stagger", 0.04)],
        &[
            ("opacity", 0., false),
            ("y", 0.4, true),
            ("rotation", -45., false),
            ("scale", 60., false),
        ],
    ),
    preset(
        "blur",
        "Blur in",
        "in",
        STAGGER,
        CHARACTERS,
        Some("cubicOut"),
        &[("duration", 0.8), ("stagger", 0.03)],
        &[("opacity", 0., false), ("blur", 0.25, true)],
    ),
    preset(
        "tracking",
        "Tracking in",
        "in",
        STAGGER,
        CHARACTERS,
        Some("expoOut"),
        &[("duration", 1.2), ("stagger", 0.)],
        &[("opacity", 0., false), ("tracking", 0.4, true)],
    ),
    preset(
        "typewriter",
        "Typewriter",
        "in",
        STAGGER,
        CHARACTERS,
        Some("linear"),
        &[("duration", 0.), ("stagger", 0.05)],
        &[("opacity", 0., false)],
    ),
    preset(
        "wave",
        "Wave",
        "loop",
        "wave",
        CHARACTERS,
        None,
        &[("speed", 1.), ("spread", 8.)],
        &[("y", -0.15, true)],
    ),
    preset(
        "pulse",
        "Pulse",
        "loop",
        "wave",
        CHARACTERS,
        None,
        &[("speed", 0.8), ("spread", 6.)],
        &[("scale", 115., false)],
    ),
    preset(
        "custom-stagger",
        "Stagger",
        "custom",
        STAGGER,
        CHARACTERS,
        Some("cubicOut"),
        &[],
        &[("opacity", 0., false)],
    ),
    preset(
        "custom-range",
        "Range selector",
        "custom",
        "range",
        CHARACTERS,
        None,
        &[],
        &[("opacity", 0., false)],
    ),
];

/// The menu in its reading order, each entry a label with an id or a header.
pub fn preset_menu() -> Vec<(Option<&'static str>, &'static str)> {
    let mut out = Vec::new();
    for (group, header) in [("in", "Animate In"), ("loop", "Loop"), ("custom", "Custom")] {
        out.push((None, header));
        out.extend(
            PRESETS
                .iter()
                .filter(|p| p.group == group)
                .map(|p| (Some(p.id), p.label)),
        );
    }
    out
}

fn property(v: Value) -> Value {
    let mut m = Map::new();
    m.insert("v", v);
    m.insert("kf", Value::Array(Vec::new()));
    m.insert("expr", Value::Null);
    Value::Object(m)
}

/// `makeAnimator`.
fn animator(preset: &Preset, name: &str, size: f64) -> Value {
    let defaults: &[(&str, f64)] = match preset.mode {
        STAGGER => &[("delay", 0.), ("duration", 0.6), ("stagger", 0.04)],
        "range" => &[
            ("start", 0.),
            ("end", 100.),
            ("offset", 0.),
            ("smoothness", 0.),
        ],
        _ => &[("speed", 1.), ("spread", 8.)],
    };
    let mut p = Map::new();
    p.insert("amount", property(Value::Number(100.)));
    for (key, value) in defaults.iter().chain(preset.settings) {
        p.insert(*key, property(Value::Number(*value)));
    }
    for (key, value, scaled) in preset.channels {
        let value = if *scaled {
            pm_eval::js_sem::math_round(size * value)
        } else {
            *value
        };
        p.insert(*key, property(Value::Number(value)));
    }
    let mut m = Map::new();
    m.insert("name", Value::from(name));
    m.insert("enabled", Value::Bool(true));
    m.insert("mode", Value::from(preset.mode));
    m.insert("unit", Value::from(preset.unit));
    m.insert("order", Value::from("forward"));
    if preset.mode == STAGGER {
        m.insert("direction", Value::from("in"));
        m.insert("easing", Value::from(preset.easing.unwrap_or("cubicOut")));
    }
    if preset.mode == "range" {
        m.insert("shape", Value::from("square"));
    }
    m.insert("p", Value::Object(p));
    Value::Object(m)
}

impl Inspector {
    /// `{id, name}` of each authored animator, in stack order.
    pub fn text_animators(&self, layer: &str) -> Result<Vec<(String, String)>, InspectorError> {
        let layer = self.layer(layer)?;
        let list = layer
            .get("d")
            .and_then(|d| d.get("animators"))
            .and_then(Value::as_array);
        Ok(list
            .into_iter()
            .flatten()
            .filter_map(|a| {
                let text = |key| a.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
                a.get("id")?.as_str()?;
                Some((text("id"), text("name")))
            })
            .collect())
    }
    /// `addPreset`: the first free name, sized from the type at the playhead.
    pub fn add_text_animator(
        &self,
        caller: Caller,
        layer: &str,
        preset_id: &str,
        size: f64,
    ) -> Result<Request, InspectorError> {
        let preset = PRESETS
            .iter()
            .find(|p| p.id == preset_id)
            .ok_or(InspectorError::InvalidInput)?;
        let taken: Vec<String> = self
            .text_animators(layer)?
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        let name = std::iter::once(preset.label.to_owned())
            .chain((2..).map(|n| format!("{} {n}", preset.label)))
            .find(|name| !taken.contains(name))
            .expect("unbounded");
        let size = if size.is_finite() && size != 0. {
            size
        } else {
            64.
        };
        self.transaction(
            caller,
            vec![
                TextCommand::AddAnimator {
                    layer_id: layer.into(),
                    animator_id: pm_kernel::new_id("ta"),
                    animator: animator(preset, &name, size),
                }
                .into(),
            ],
        )
    }
    pub fn remove_text_animator(
        &self,
        caller: Caller,
        layer: &str,
        animator_id: &str,
    ) -> Result<Request, InspectorError> {
        self.transaction(
            caller,
            vec![
                TextCommand::RemoveAnimator {
                    layer_id: layer.into(),
                    animator_id: animator_id.into(),
                }
                .into(),
            ],
        )
    }
}
