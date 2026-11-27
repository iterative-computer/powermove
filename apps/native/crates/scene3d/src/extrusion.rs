//! Studio text/shape extrusion contract, including sparse surface inheritance.
use crate::{
    Error, Result, fail,
    operations::{Action, Plan},
    schema,
};
pub use pm_geom3d::extrusion::{Bevel, Extrusion, Options, Outline, build};
use serde_json::{Value, json};
pub const SURFACES: &[&str] = &["front", "bevel", "side", "back"];
pub const SLOTS: &[&str] = &[
    "color",
    "normal",
    "roughness",
    "metalness",
    "emissive",
    "ao",
];
pub const OPERATIONS: &[&str] = &[
    "set_extrusion",
    "set_material_map",
    "set_surface_material_map",
    "set_material_properties",
    "set_surface_material_properties",
];
pub fn eligible(layer: &Value) -> bool {
    matches!(layer["type"].as_str(), Some("text" | "shape"))
}
pub fn enabled(layer: &Value) -> bool {
    eligible(layer) && layer["threeD"] == true && layer["d"]["extrusion"]["enabled"] == true
}
/// Absent settings read as disabled; callers never rewrite them on read.
pub fn parse(raw: &Value) -> Result<Value> {
    let empty = json!({});
    let raw = if raw.is_null() { &empty } else { raw };
    schema::strict(raw, &["version", "enabled", "p", "surfaces"])?;
    if raw.get("version").is_some_and(|v| v != 1) {
        return fail("Unsupported extrusion version");
    }
    if raw.get("enabled").is_some_and(|v| !v.is_boolean()) {
        return fail("Extrusion enabled must be a boolean");
    }
    let mut out = json!({"version":1,"enabled":raw.get("enabled").cloned().unwrap_or(json!(false)),"p":schema::properties("extrusion",raw.get("p"))?,"surfaces":{}});
    if let Some(surfaces) = raw.get("surfaces") {
        schema::strict(surfaces, SURFACES)?;
        for (name, surface) in surfaces.as_object().unwrap() {
            schema::strict(surface, &["maps", "properties"])?;
            let maps = surface.get("maps").cloned().unwrap_or(json!({}));
            schema::strict(&maps, SLOTS)?;
            for (slot, source) in maps.as_object().unwrap() {
                crate::contract::source(source, false)?;
                if source.is_object() && !matches!(slot.as_str(), "color" | "emissive") {
                    return fail("Live sources support color and emissive maps");
                }
            }
            let mut properties = json!({});
            if let Some(p) = surface.get("properties") {
                for (key, value) in p
                    .as_object()
                    .ok_or_else(|| Error("Expected surface properties".into()))?
                {
                    properties[key] = schema::validate_channel("surface", key, value.clone())?;
                }
            }
            out["surfaces"][name] = json!({"maps":maps,"properties":properties});
        }
    }
    Ok(out)
}
pub fn channels(layer: &Value) -> Vec<(String, &Value)> {
    if !eligible(layer) {
        return vec![];
    }
    let data = &layer["d"]["extrusion"];
    let mut out = vec![];
    if let Some(p) = data["p"].as_object() {
        out.extend(p.iter().map(|(k, v)| (format!("geo.{k}"), v)));
    }
    for surface in SURFACES {
        if let Some(p) = data["surfaces"][surface]["properties"].as_object() {
            out.extend(p.iter().map(|(k, v)| (format!("ms.{surface}.{k}"), v)));
        }
    }
    out
}
pub fn plan(comp: &Value, args: &Value, selection: &[String]) -> Result<Plan> {
    let target = args["target"]
        .as_str()
        .or(args["id"].as_str())
        .or_else(|| selection.first().map(String::as_str))
        .ok_or_else(|| Error("Choose a text or shape layer".into()))?;
    let layers = comp["layers"]
        .as_array()
        .ok_or_else(|| Error("No composition layers".into()))?;
    let mut matches = layers
        .iter()
        .filter(|l| l["id"] == target || (l["name"] == target && eligible(l)));
    let layer = matches
        .next()
        .ok_or_else(|| Error("Extrusion layer not found".into()))?;
    if matches.next().is_some() || !eligible(layer) {
        return fail("Choose a unique text or shape layer ID");
    }
    let id = layer["id"]
        .as_str()
        .ok_or_else(|| Error("Layer has no ID".into()))?
        .to_owned();
    let mut data = parse(&layer["d"]["extrusion"])?;
    let mut actions = vec![];
    let empty = json!({});
    let patch = args.get("patch").unwrap_or(&empty);
    let patch = patch
        .as_object()
        .ok_or_else(|| Error("Expected extrusion patch".into()))?;
    let mut enable_three_d = false;
    match args["operation"].as_str().unwrap_or("") {
        "set_extrusion" => {
            for (key, value) in patch {
                if key == "enabled" {
                    let enabled = value
                        .as_bool()
                        .ok_or_else(|| Error("Extrusion enabled must be a boolean".into()))?;
                    data["enabled"] = json!(enabled);
                    enable_three_d = enabled;
                } else {
                    schema::validate_channel("extrusion", key, value.clone())?;
                    actions.push(Action::Channel {
                        id: id.clone(),
                        path: format!("geo.{key}"),
                        value: value.clone(),
                    });
                }
            }
        }
        operation => {
            let surface = if let Some(name) = args.get("surface") {
                name.as_str().filter(|s| SURFACES.contains(s))
            } else if let Some(index) = args.get("materialIndex") {
                index
                    .as_u64()
                    .and_then(|i| SURFACES.get(i as usize).copied())
            } else {
                Some("front")
            }
            .ok_or_else(|| Error("Choose Front, Bevel, Side or Back".into()))?;
            if data["surfaces"].get(surface).is_none() {
                data["surfaces"][surface] = json!({"maps":{},"properties":{}});
            }
            let material = &mut data["surfaces"][surface];
            if matches!(operation, "set_material_map" | "set_surface_material_map") {
                let slot = args["slot"]
                    .as_str()
                    .filter(|s| SLOTS.contains(s))
                    .ok_or_else(|| Error("Choose a material map slot".into()))?;
                let source = &args["map"];
                if source.is_null() {
                    material["maps"].as_object_mut().unwrap().remove(slot);
                } else {
                    material["maps"][slot] = source.clone();
                }
            } else {
                for (key, value) in patch {
                    if value.is_null() {
                        schema::validate_channel(
                            "surface",
                            key,
                            schema::properties("surface", None)?[key].clone(),
                        )?;
                        material["properties"].as_object_mut().unwrap().remove(key);
                        continue;
                    }
                    schema::validate_channel("surface", key, value.clone())?;
                    if material["properties"].get(key).is_none() {
                        let mut defaults = schema::properties("surface", None)?;
                        if key == "color" {
                            let color = &layer["d"]["color"];
                            let color = color.get("v").unwrap_or(color);
                            if color.is_string() {
                                defaults["color"] = schema::channel(color.clone());
                            }
                        }
                        material["properties"][key] = defaults[key].clone();
                    }
                    actions.push(Action::Channel {
                        id: id.clone(),
                        path: format!("ms.{surface}.{key}"),
                        value: value.clone(),
                    });
                }
            }
        }
    }
    actions.insert(
        0,
        Action::Extrusion {
            id: id.clone(),
            data: parse(&data)?,
            enable_three_d,
        },
    );
    Ok(Plan {
        actions,
        id: Some(id),
    })
}
