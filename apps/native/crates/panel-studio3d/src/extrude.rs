//! Text and shape extrusion (AE's Geometry Options) and live material
//! sources. One click gives a layer depth, a rounded bevel that catches
//! light and four surfaces that read as one object; every value after that
//! is an ordinary keyframeable channel (`geo.*`, `ms.SURFACE.*`).
use crate::{Result, invalid};
use pm_kernel::{Command, Published, Scene3dCommand};
use pm_scene3d::{
    extrusion::{self, SURFACES},
    from_model,
    schema,
    to_model,
};
use serde_json::{Value, json};

/// Front, Bevel, Side, Back as people read them.
pub const SURFACE_LABELS: &[(&str, &str)] = &[
    ("front", "Front"),
    ("bevel", "Bevel"),
    ("side", "Side"),
    ("back", "Back"),
];
pub const BEVELS: &[(&str, &str)] = &[
    ("none", "None"),
    ("angular", "Angular"),
    ("convex", "Rounded"),
    ("concave", "Inset"),
];
pub const DETAIL: &[(&str, &str, &str)] = &[
    ("draft", "Draft", "Fewer curve steps; fastest."),
    ("standard", "Standard", "Smooth at normal sizes."),
    ("high", "High", "For close-ups."),
];

fn composition(p: &Published) -> Result<Value> {
    let root = p.snapshot.root().ok_or_else(|| invalid("No project"))?;
    from_model(
        &pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref())
            .ok_or_else(|| invalid("No composition"))?
            .to_value(),
    )
    .map_err(Into::into)
}
fn apply(p: &Published, time: f64, args: Value) -> Command {
    Scene3dCommand::Apply {
        comp: p.snapshot.open_comp(),
        args: to_model(&args),
        selection: p.selection.layers().to_vec(),
        time,
        autokey: false,
    }
    .into()
}
fn static_value(v: &Value) -> &Value {
    v.get("v").unwrap_or(v)
}
/// The layer's own fill, which Front starts from.
pub fn base_colour(layer: &Value) -> String {
    for key in ["color", "fill"] {
        if let Some(c) = static_value(&layer["d"][key]).as_str()
            && c.starts_with('#')
            && (c.len() == 7 || c.len() == 9)
        {
            return c[..7].to_ascii_uppercase();
        }
    }
    "#E8E2CF".into()
}
fn shade(hex: &str, k: f64) -> String {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xE8E2CF);
    let c = |s: u32| ((((n >> s) & 255) as f64 * k).round().clamp(0., 255.)) as u32;
    format!("#{:02X}{:02X}{:02X}", c(16), c(8), c(0))
}
/// How big the artwork is, in composition pixels: text by its size, a shape
/// by its smaller side.
fn scale(layer: &Value) -> f64 {
    let n = |k: &str| static_value(&layer["d"][k]).as_f64();
    let s = if layer["type"] == "text" {
        n("size").unwrap_or(100.)
    } else {
        match (n("w"), n("h")) {
            (Some(w), Some(h)) => w.min(h),
            _ => 200.,
        }
    };
    s.clamp(8., 2000.)
}
/// The one-click look: about a third of the type size deep, a rounded bevel
/// that rims the letters with light, a darker side so the face reads first,
/// and a satin face.
pub fn defaults(layer: &Value) -> Value {
    let s = scale(layer);
    let colour = base_colour(layer);
    json!({
        "geo": {
            "depth": (s * 0.32).round().max(4.),
            "bevelStyle": "convex",
            "bevelDepth": (s * 0.035).round().clamp(1., 40.),
            "holeBevelDepth": 100,
            "quality": "standard",
        },
        "surfaces": {
            "front": {"color": colour, "roughness": 0.32, "metallic": 0},
            "bevel": {"color": shade(&colour, 1.08), "roughness": 0.16, "metallic": 0.65},
            "side": {"color": shade(&colour, 0.58), "roughness": 0.45, "metallic": 0},
            "back": {"color": shade(&colour, 0.5), "roughness": 0.5, "metallic": 0},
        },
    })
}
/// The extrusion record, or `None` for a layer that can't be extruded.
pub fn data(layer: &Value) -> Option<Value> {
    if !extrusion::eligible(layer) {
        return None;
    }
    extrusion::parse(&layer["d"]["extrusion"]).ok()
}
pub fn enabled(layer: &Value) -> bool {
    extrusion::enabled(layer)
}
/// A surface's value for `key`: its own, else what it starts from.
pub fn surface_value(layer: &Value, surface: &str, key: &str) -> Value {
    let own = &layer["d"]["extrusion"]["surfaces"][surface]["properties"][key];
    if !own.is_null() {
        return static_value(own).clone();
    }
    if key == "color" {
        return json!(base_colour(layer));
    }
    schema::properties("surface", None)
        .map(|d| static_value(&d[key]).clone())
        .unwrap_or(Value::Null)
}
/// Whether `surface.key` is stored (and so an ordinary channel).
pub fn has_override(layer: &Value, surface: &str, key: &str) -> bool {
    !layer["d"]["extrusion"]["surfaces"][surface]["properties"][key].is_null()
}

/// Live map sources (AE 27 layers as materials): `layer:<id>` for a 2D
/// layer or group in this composition, `comp:<id>` for a composition.
pub fn sources(p: &Published, receiver: &str) -> Result<Vec<Value>> {
    let comp = composition(p)?;
    let mut out = vec![];
    for l in comp["layers"].as_array().into_iter().flatten() {
        let id = l["id"].as_str().unwrap_or("");
        let kind = l["type"].as_str().unwrap_or("");
        if id == receiver
            || schema::role(l).is_some()
            || matches!(kind, "audio" | "camera" | "light" | "null" | "adjustment")
            || extrusion::enabled(l)
        {
            continue;
        }
        let detail = if kind == "group" { "Group" } else { "Layer" };
        out.push(json!({"v":format!("layer:{id}"),"label":l["name"].as_str().unwrap_or(id),"detail":detail}));
    }
    let root = from_model(
        &p.snapshot
            .root()
            .ok_or_else(|| invalid("No project"))?
            .to_value(),
    )?;
    for (id, c) in root["comps"].as_object().into_iter().flatten() {
        if p.snapshot.open_comp().as_deref() == Some(id) {
            continue;
        }
        out.push(json!({"v":format!("comp:{id}"),"label":c["name"].as_str().unwrap_or(id),"detail":"Composition"}));
    }
    Ok(out)
}
/// A saved map record as its menu value.
pub fn source_value(map: &Value) -> Value {
    if let Some(id) = map["layerId"].as_str() {
        json!(format!("layer:{id}"))
    } else if let Some(id) = map["compositionId"].as_str() {
        json!(format!("comp:{id}"))
    } else {
        map.clone()
    }
}
/// A menu value as the saved record: live sources become references,
/// anything else (an imported image's asset ID, None) stays as it is.
pub fn source_record(arg: &Value) -> Value {
    match arg.as_str() {
        Some(s) if s.starts_with("layer:") => json!({"layerId": &s[6..]}),
        Some(s) if s.starts_with("comp:") => json!({"compositionId": &s[5..]}),
        _ => arg.clone(),
    }
}

/// `extrude.enabled`, `extrude.bevelStyle`, `extrude.quality`,
/// `extrude.SURFACE.KEY` and `extrude.SURFACE.map.SLOT` on the selected
/// text or shape layer.
pub(crate) fn commands(
    p: &Published,
    time: f64,
    control: &str,
    arg: Value,
    lit: &dyn Fn(&[Value], &str) -> Vec<Value>,
) -> Result<(String, Vec<Command>)> {
    if p.selection.layers().len() != 1 {
        return Err(invalid("Choose one text or shape layer"));
    }
    let comp = composition(p)?;
    let layers = comp["layers"].as_array().cloned().unwrap_or_default();
    let layer = layers
        .iter()
        .find(|l| l["id"] == p.selection.layers()[0])
        .ok_or_else(|| invalid("Layer disappeared"))?;
    if !extrusion::eligible(layer) {
        return Err(invalid("Only text and shape layers can be extruded"));
    }
    if layer["lock"] == true {
        return Err(invalid("Layer is locked"));
    }
    let target = layer["id"].as_str().unwrap_or("").to_owned();
    let set = |patch: Value| json!({"operation":"set_extrusion","target":target,"patch":patch});
    match control {
        "enabled" => {
            let on = arg.as_bool().ok_or_else(|| invalid("Expected checkbox"))?;
            let mut out = vec![];
            let fresh = data(layer)
                .map(|d| d["p"]["depth"]["v"].as_f64().unwrap_or(0.) == 0.)
                .unwrap_or(true);
            if on && fresh {
                // First time on: the whole look in one Undo step.
                let d = defaults(layer);
                let mut patch = d["geo"].clone();
                patch["enabled"] = json!(true);
                out.push(apply(p, time, set(patch)));
                for (surface, props) in d["surfaces"].as_object().unwrap() {
                    out.push(apply(
                        p,
                        time,
                        json!({"operation":"set_surface_material_properties","target":target,"surface":surface,"patch":props}),
                    ));
                }
                // An unlit composition gets a soft sky (lighting only, so the
                // design's own background stays) and a key light.
                let id = format!("x{}", p.revision());
                for args in lit(&layers, &id) {
                    out.push(apply(p, time, args));
                }
            } else {
                out.push(apply(p, time, set(json!({"enabled": on}))));
            }
            Ok((if on { "Extrude" } else { "Flatten" }.into(), out))
        }
        "bevelStyle" => {
            if !BEVELS.iter().any(|(v, _)| arg == *v) {
                return Err(invalid("Unknown bevel"));
            }
            Ok((
                "Bevel".into(),
                vec![apply(p, time, set(json!({"bevelStyle": arg})))],
            ))
        }
        "quality" => {
            if !DETAIL.iter().any(|(v, ..)| arg == *v) {
                return Err(invalid("Unknown detail"));
            }
            Ok((
                "Extrusion Detail".into(),
                vec![apply(p, time, set(json!({"quality": arg})))],
            ))
        }
        other => {
            let (surface, rest) = other
                .split_once('.')
                .ok_or_else(|| invalid("Unknown extrusion control"))?;
            let name = SURFACE_LABELS
                .iter()
                .find(|(s, _)| *s == surface)
                .ok_or_else(|| invalid("Choose Front, Bevel, Side or Back"))?
                .1;
            debug_assert!(SURFACES.contains(&surface));
            if let Some(slot) = rest.strip_prefix("map.") {
                if !matches!(slot, "color" | "emissive") {
                    return Err(invalid("Unknown surface map"));
                }
                let map = source_record(&arg);
                return Ok((
                    format!("{name} {}", if slot == "color" { "Color Map" } else { "Emissive Map" }),
                    vec![apply(
                        p,
                        time,
                        json!({"operation":"set_surface_material_map","target":target,"surface":surface,"slot":slot,"map":map}),
                    )],
                ));
            }
            let label = match rest {
                "color" => "Color",
                "roughness" => "Roughness",
                "metallic" => "Metalness",
                "opacity" => "Opacity",
                "emissionColor" => "Emission Color",
                "emissionIntensity" => "Emission",
                _ => return Err(invalid("Unknown surface property")),
            };
            Ok((
                format!("{name} {label}"),
                vec![apply(
                    p,
                    time,
                    json!({"operation":"set_surface_material_properties","target":target,"surface":surface,"patch":{rest: arg}}),
                )],
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_click_scales_with_the_type() {
        let text = json!({"type":"text","d":{"size":120,"color":"#3366ff"}});
        let d = defaults(&text);
        assert_eq!(d["geo"]["depth"], json!(38.));
        assert_eq!(d["geo"]["bevelDepth"], json!(4.));
        assert_eq!(d["geo"]["bevelStyle"], "convex");
        assert_eq!(d["surfaces"]["front"]["color"], "#3366FF");
        assert_eq!(d["surfaces"]["side"]["color"], "#1E3B94");
    }
    #[test]
    fn sources_round_trip() {
        assert_eq!(source_record(&json!("layer:a")), json!({"layerId":"a"}));
        assert_eq!(source_record(&json!("comp:c")), json!({"compositionId":"c"}));
        assert_eq!(source_record(&json!("asset1")), json!("asset1"));
        assert_eq!(source_value(&json!({"layerId":"a"})), json!("layer:a"));
    }
}
