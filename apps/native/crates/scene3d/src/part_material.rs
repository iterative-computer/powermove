//! Per-part look development on imported models: a part's material slot (its
//! source material index) takes sparse overrides that live in the project;
//! the source file is never touched. Values are ordinary `mi.<index>.<key>`
//! channels (keyframeable, one Undo per edit); without `materialIndex` the
//! edit applies to every part (`mi.<key>`).
use crate::{
    Error, Result, fail,
    operations::{Action, Plan},
    schema::{self, Role},
};
use serde_json::{Value, json};

/// What a part's material can override, in Properties order.
pub const PROPERTIES: &[&str] = &[
    "color",
    "metallic",
    "roughness",
    "clearcoat",
    "clearcoatRoughness",
    "transmission",
    "ior",
    "emissionColor",
    "emissionIntensity",
    "iridescence",
    "opacity",
    "ambientResponse",
];
/// Map slots a part takes; `color` and `emissive` also take live sources
/// (a 2D layer, group or composition: "use a layer as the material").
pub const MAP_SLOTS: &[&str] = &["color", "normal", "roughness", "metalness", "emissive", "ao"];

/// An imported model (not a primitive, terrain or nature object).
pub fn is_model(layer: &Value) -> bool {
    schema::role(layer) == Some(Role::Object)
        && layer["d"]["data"]["object"]["source"]["assetId"].is_string()
}

/// `set_material_properties` / `set_material_map` aimed at an imported model;
/// `None` when the target is something else (text and shape extrusion keep
/// their own planner).
pub fn plan(comp: &Value, args: &Value, selection: &[String]) -> Result<Option<Plan>> {
    let Some(target) = args["target"]
        .as_str()
        .or(args["id"].as_str())
        .or_else(|| selection.first().map(String::as_str))
    else {
        return Ok(None);
    };
    let layers = comp["layers"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let Some(layer) = layers
        .iter()
        .find(|l| l["id"] == target)
        .or_else(|| layers.iter().find(|l| l["name"] == target && is_model(l)))
    else {
        return Ok(None);
    };
    if !is_model(layer) {
        return Ok(None);
    }
    if layer["lock"] == true {
        return fail("Layer is locked");
    }
    let id = layer["id"]
        .as_str()
        .ok_or_else(|| Error("Layer has no ID".into()))?
        .to_owned();
    let index = match args.get("materialIndex") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .filter(|i| *i <= 9999)
                .ok_or_else(|| Error("materialIndex must be a material slot number".into()))?
                as usize,
        ),
    };
    let mut data = layer["d"]["data"].clone();
    let object = &mut data["object"];
    // Slot overrides apply on top of the model's own materials.
    if index.is_some() {
        object["useSourceMaterials"] = json!(true);
    }
    let mut actions = vec![];
    match args["operation"].as_str().unwrap_or("") {
        "set_material_map" => {
            let slot = args["slot"]
                .as_str()
                .filter(|s| MAP_SLOTS.contains(s))
                .ok_or_else(|| Error("Choose a material map slot".into()))?;
            let source = &args["map"];
            if !source.is_null() {
                crate::contract::source(source, false)?;
                if source.is_object() && !matches!(slot, "color" | "emissive") {
                    return fail("Live sources support color and emissive maps");
                }
            }
            let maps = match index {
                Some(i) => {
                    let key = i.to_string();
                    if !object["sourceMaterials"].is_object() {
                        object["sourceMaterials"] = json!({});
                    }
                    if !object["sourceMaterials"][&key].is_object() {
                        object["sourceMaterials"][&key] = json!({});
                    }
                    &mut object["sourceMaterials"][&key]
                }
                None => {
                    if !object["material"]["maps"].is_object() {
                        object["material"]["maps"] = json!({});
                    }
                    &mut object["material"]["maps"]
                }
            };
            if source.is_null() {
                maps.as_object_mut().unwrap().remove(slot);
            } else {
                maps[slot] = source.clone();
            }
        }
        "set_material_properties" => {
            let patch = args["patch"]
                .as_object()
                .ok_or_else(|| Error("Expected material properties".into()))?;
            if patch.is_empty() {
                return fail("Nothing to change");
            }
            let defaults = schema::properties("intrinsic", None)?;
            let entry = match index {
                Some(i) => {
                    let key = i.to_string();
                    if !object["sourceMaterialProperties"].is_object() {
                        object["sourceMaterialProperties"] = json!({});
                    }
                    if !object["sourceMaterialProperties"][&key].is_object() {
                        object["sourceMaterialProperties"][&key] = json!({});
                    }
                    &mut object["sourceMaterialProperties"][&key]
                }
                None => {
                    if !object["materialProperties"].is_object() {
                        object["materialProperties"] = json!({});
                    }
                    &mut object["materialProperties"]
                }
            };
            for (key, value) in patch {
                if !PROPERTIES.contains(&key.as_str()) {
                    return fail(format!("Unknown part material property: {key}"));
                }
                if value.is_null() {
                    // Back to the part's own value.
                    entry.as_object_mut().unwrap().remove(key);
                    continue;
                }
                schema::validate_channel("intrinsic", key, value.clone())?;
                if entry.get(key).is_none() {
                    entry[key] = defaults[key].clone();
                }
                actions.push(Action::Channel {
                    id: id.clone(),
                    path: match index {
                        Some(i) => format!("mi.{i}.{key}"),
                        None => format!("mi.{key}"),
                    },
                    value: value.clone(),
                });
            }
        }
        _ => return Ok(None),
    }
    actions.insert(
        0,
        Action::Data {
            id: id.clone(),
            data,
        },
    );
    Ok(Some(Plan {
        actions,
        id: Some(id),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn comp() -> Value {
        json!({"layers":[
            {"id":"ipad","name":"iPad","type":"3d","d":{"definition":schema::OBJECT,"data":{"object":{"source":{"assetId":"a1"},"useSourceMaterials":false,"material":{"p":{}}}}}},
            {"id":"t","name":"Title","type":"text","d":{}}
        ]})
    }
    #[test]
    fn a_part_edit_stores_sparse_overrides_and_uses_the_models_own_materials() {
        let plan = plan(
            &comp(),
            &json!({"operation":"set_material_properties","target":"ipad","materialIndex":2,
                    "patch":{"metallic":1,"roughness":0.18,"clearcoat":0.6}}),
            &[],
        )
        .unwrap()
        .unwrap();
        let Action::Data { data, .. } = &plan.actions[0] else {
            panic!("data first")
        };
        assert_eq!(data["object"]["useSourceMaterials"], true);
        let entry = &data["object"]["sourceMaterialProperties"]["2"];
        assert!(entry.get("metallic").is_some() && entry.get("clearcoat").is_some());
        let paths: Vec<_> = plan.actions[1..]
            .iter()
            .map(|a| match a {
                Action::Channel { path, .. } => path.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(paths, ["mi.2.metallic", "mi.2.roughness", "mi.2.clearcoat"]);
        // A text layer goes to the extrusion planner.
        assert!(plan_for("t").is_none());
        // Unknown keys are refused.
        assert!(
            super::plan(
                &comp(),
                &json!({"operation":"set_material_properties","target":"ipad","materialIndex":0,"patch":{"glow":1}}),
                &[]
            )
            .is_err()
        );
    }
    fn plan_for(target: &str) -> Option<Plan> {
        plan(
            &comp(),
            &json!({"operation":"set_material_properties","target":target,"patch":{"metallic":1}}),
            &[],
        )
        .unwrap()
    }
    #[test]
    fn a_layer_can_be_a_parts_colour_map() {
        let plan = plan(
            &comp(),
            &json!({"operation":"set_material_map","target":"ipad","materialIndex":1,"slot":"color","map":{"layerId":"t"}}),
            &[],
        )
        .unwrap()
        .unwrap();
        let Action::Data { data, .. } = &plan.actions[0] else {
            panic!()
        };
        assert_eq!(data["object"]["sourceMaterials"]["1"]["color"], json!({"layerId":"t"}));
    }
}
