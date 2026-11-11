//! Pure plans. The kernel executes these inside its scratch transaction; planning
//! never mutates a live document, publishes history, or enters JS.
use crate::{
    Error, Result, fail,
    schema::{self, Role, TRANSFORMS},
};
use serde_json::{Value, json};
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Scratch-only content staging. The kernel validates the final compound
    /// state before publication, after normal channel/autokey commands.
    StagedData {
        id: String,
        data: Value,
    },
    RenderQuality {
        quality: Value,
    },
    Add {
        id: String,
        name: String,
        role: Role,
        data: Value,
        properties: Value,
        visible: bool,
        parent: Option<String>,
    },
    Channel {
        id: String,
        path: String,
        value: Value,
    },
    Data {
        id: String,
        data: Value,
    },
    Metadata {
        id: String,
        patch: Value,
    },
    Remove {
        id: String,
    },
    Duplicate {
        id: String,
        new_id: String,
        name: Option<String>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub id: Option<String>,
}
fn transform(p: &Value) -> Value {
    let mut out = json!({});
    for (alias, path) in TRANSFORMS {
        if let Some(v) = p.get(*alias) {
            let factor = if alias.starts_with('s') { 100. } else { 1. };
            let mut v = v.clone();
            if v.is_object() {
                v["v"] = json!(v["v"].as_f64().unwrap() * factor);
                for k in v["kf"].as_array_mut().unwrap() {
                    k["v"] = json!(k["v"].as_f64().unwrap() * factor);
                }
            } else {
                v = json!(v.as_f64().unwrap() * factor);
            }
            out[*path] = v;
        }
    }
    out
}
pub fn add(
    role: Role,
    mut node: Value,
    id: String,
    name: Option<&str>,
    parent: Option<String>,
) -> Result<Action> {
    let label = node["name"]
        .as_str()
        .or(name)
        .unwrap_or(match role {
            Role::Object => "3D Model",
            Role::Light => match node["type"].as_str().unwrap_or("sun") {
                "area" => "Softbox",
                "spot" => "Spotlight",
                "point" => "Bulb",
                _ => "Sun",
            },
            Role::Camera => "Camera",
        })
        .to_string();
    node.as_object_mut()
        .ok_or_else(|| Error("Expected 3D node".into()))?
        .remove("name");
    let mut visible = true;
    let (data, props) = match role {
        Role::Object => {
            node["id"] = json!(id);
            node["parent"] = Value::Null;
            let mut n = schema::object(node)?;
            let p = n["p"].clone();
            visible = visibility(&p["visible"])?;
            for k in ["id", "name", "parent", "p"] {
                n.as_object_mut().unwrap().remove(k);
            }
            (
                schema::layer_data(role, json!({"object":n}))?,
                transform(&p),
            )
        }
        Role::Light => {
            node["id"] = json!(id);
            let mut n = schema::light(node)?;
            let p = n["p"].clone();
            visible = visibility(&p["visible"])?;
            for k in ["id", "name"] {
                n.as_object_mut().unwrap().remove(k);
            }
            for k in ["x", "y", "z", "visible"] {
                n["p"].as_object_mut().unwrap().remove(k);
            }
            (schema::layer_data(role, json!({"light":n}))?, transform(&p))
        }
        Role::Camera => {
            let c = if node.get("camera").is_some() {
                node["camera"].clone()
            } else {
                node.clone()
            };
            let mut c = schema::camera(c)?;
            let p = c["p"].clone();
            for k in ["x", "y", "z"] {
                c["p"].as_object_mut().unwrap().remove(k);
            }
            (
                schema::layer_data(
                    role,
                    json!({"camera":c,"environment":node.get("environment").cloned().unwrap_or(json!({}))}),
                )?,
                transform(&p),
            )
        }
    };
    Ok(Action::Add {
        id,
        name: label,
        role,
        data,
        properties: props,
        visible,
        parent,
    })
}
fn visibility(v: &Value) -> Result<bool> {
    if let Some(b) = v.as_bool() {
        return Ok(b);
    }
    if v["kf"].as_array().is_some_and(|a| a.is_empty())
        && v["expr"].is_null()
        && let Some(b) = v["v"].as_bool()
    {
        return Ok(b);
    }
    fail("Animated visibility is not supported; use opacity or light intensity")
}
fn choose<'a>(layers: &'a [Value], target: Option<&str>) -> Result<&'a Value> {
    let t = target.ok_or_else(|| Error("Choose a 3D layer".into()))?;
    if let Some(l) = layers.iter().find(|l| l["id"] == t) {
        return Ok(l);
    }
    let mut matches = layers
        .iter()
        .filter(|l| l["name"] == t && schema::role(l).is_some());
    let first = matches
        .next()
        .ok_or_else(|| Error("3D layer not found".into()))?;
    if matches.next().is_some() {
        return fail("Multiple 3D layers have that name; use a layer ID");
    }
    Ok(first)
}
/// Current-comp snapshot and an ID allocator supplied by the host. Selection is
/// explicit; no global PM object and no cached array indices.
pub fn plan(
    comp: &Value,
    args: &Value,
    selection: &[String],
    uid: &mut impl FnMut(&str) -> String,
) -> Result<Plan> {
    schema::plain(args)?;
    let operation = args["operation"]
        .as_str()
        .ok_or_else(|| Error("Missing 3D operation".into()))?;
    if [
        "set_world",
        "add_fog",
        "update_fog",
        "add_terrain",
        "update_terrain",
        "add_scatter",
        "update_scatter",
        "set_material_options",
        "set_render_quality",
    ]
    .contains(&operation)
    {
        return crate::contract::plan(comp, args, selection, uid);
    }
    if [
        "apply_material",
        "light_highlight",
        "shot",
        "set_studio",
        "soften",
        "drop_to_floor",
        "place_on",
        "explode",
        "auto_studio",
    ]
    .contains(&operation)
    {
        return crate::service::studio_plan(comp, args, selection, uid);
    }
    schema::strict(
        args,
        &[
            "operation",
            "target",
            "id",
            "name",
            "object",
            "light",
            "camera",
            "patch",
            "scene",
        ],
    )?;
    let layers = comp["layers"]
        .as_array()
        .ok_or_else(|| Error("Composition has no layers".into()))?;
    let mut actions = vec![];
    // Legacy sample-count commands and composition policy are one atomic edit,
    // as on Studio's source branch. Reads of old documents keep Production's
    // default without rewriting their serialized camera data.
    if operation == "set_environment" && args["patch"].get("samples").is_some() {
        let samples = args["patch"]["samples"]
            .as_f64()
            .filter(|v| v.is_finite() && v.fract() == 0. && (1. ..=4096.).contains(v))
            .ok_or_else(|| Error("Invalid Studio sample count".into()))?;
        let current = crate::quality::get(comp)?;
        let quality = crate::quality::resolve(
            &current,
            &json!({"maxSamples":samples,"minSamples":current["minSamples"].as_f64().unwrap().min(samples)}),
        )?;
        actions.push(Action::RenderQuality { quality });
    }

    if operation == "create" && args.get("scene").is_some() {
        let scene = schema::parse_scene(args["scene"].clone())?;
        let mut mapping = std::collections::HashMap::new();
        for o in scene["objects"].as_array().unwrap() {
            mapping.insert(o["id"].as_str().unwrap().to_string(), uid("object"));
        }
        let mut pending = scene["objects"].as_array().unwrap().clone();
        let mut added = std::collections::HashSet::new();
        let mut first = None;
        while !pending.is_empty() {
            let i = pending
                .iter()
                .position(|o| {
                    o["parent"].is_null() || added.contains(o["parent"].as_str().unwrap())
                })
                .ok_or_else(|| Error("Cyclic scene".into()))?;
            let o = pending.remove(i);
            let old = o["id"].as_str().unwrap();
            let id = mapping[old].clone();
            let parent = o["parent"].as_str().map(|p| mapping[p].clone());
            actions.push(add(Role::Object, o.clone(), id.clone(), None, parent)?);
            added.insert(old.to_string());
            first.get_or_insert(id);
        }
        for l in scene["lights"].as_array().unwrap() {
            actions.push(add(Role::Light, l.clone(), uid("light"), None, None)?);
        }
        let id = uid("camera");
        first.get_or_insert(id.clone());
        actions.push(add(
            Role::Camera,
            json!({"camera":scene["camera"],"environment":scene["environment"]}),
            id,
            None,
            None,
        )?);
        return Ok(Plan { actions, id: first });
    }
    let adding = match operation {
        "create" | "add_object" => Some(Role::Object),
        "add_light" => Some(Role::Light),
        "add_camera" => Some(Role::Camera),
        _ => None,
    };
    if let Some(role) = adding {
        let node = args
            .get(role.key())
            .cloned()
            .or_else(|| {
                if role == Role::Camera {
                    args.get("patch").cloned()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                if role == Role::Object {
                    json!({"source":{"primitive":"box"}})
                } else {
                    json!({})
                }
            });
        let id = node["id"]
            .as_str()
            .or(args["id"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| uid(role.key()));
        if layers.iter().any(|l| l["id"] == id) {
            return fail("Layer ID already exists");
        }
        let parent = node["parent"].as_str().map(str::to_owned);
        actions.push(add(role, node, id.clone(), args["name"].as_str(), parent)?);
        return Ok(Plan {
            actions,
            id: Some(id),
        });
    }
    let selected = args["id"]
        .as_str()
        .or(args["target"].as_str())
        .or_else(|| selection.first().map(String::as_str));
    let mut layer = choose(layers, selected).ok();
    if operation == "set_camera" || operation == "set_environment" {
        if layer.and_then(schema::role) != Some(Role::Camera) {
            layer = layers
                .iter()
                .find(|l| schema::role(l) == Some(Role::Camera) && l["on"] != false);
        }
        if layer.is_none() {
            let id = uid("camera");
            let node = if operation == "set_camera" {
                json!({"camera":args.get("patch").cloned().unwrap_or(json!({}))})
            } else {
                json!({"camera":{},"environment":args.get("patch").cloned().unwrap_or(json!({}))})
            };
            actions.push(add(Role::Camera, node, id.clone(), None, None)?);
            return Ok(Plan {
                actions,
                id: Some(id),
            });
        }
    }
    let layer = match layer {
        Some(l) => l,
        None => choose(layers, selected)?,
    };
    let role =
        schema::role(layer).ok_or_else(|| Error("Choose a model, light or camera layer".into()))?;
    let id = layer["id"].as_str().unwrap().to_string();
    match operation {
        "remove" => {
            actions.push(Action::Remove { id: id.clone() });
        }
        "duplicate" => {
            let new_id = uid(role.key());
            actions.push(Action::Duplicate {
                id,
                new_id: new_id.clone(),
                name: args["name"].as_str().map(str::to_owned),
            });
            return Ok(Plan {
                actions,
                id: Some(new_id),
            });
        }
        "update_object" | "update_light" | "set_camera" | "set_environment" => {
            if (operation == "update_object" && role != Role::Object)
                || (operation == "update_light" && role != Role::Light)
            {
                return fail("Wrong 3D layer kind");
            }
            let mut data = schema::layer_data(role, layer["d"]["data"].clone())?;
            let mut patch = args.get("patch").cloned().unwrap_or(json!({}));
            if patch.get("id").is_some_and(|v| v != &json!(id)) {
                return fail("3D IDs cannot be changed");
            }
            let mut meta = json!({});
            for k in ["name", "parent"] {
                if let Some(v) = patch.get(k) {
                    meta[k] = v.clone();
                }
            }
            for k in ["id", "name", "parent"] {
                patch
                    .as_object_mut()
                    .ok_or_else(|| Error("Expected patch object".into()))?
                    .remove(k);
            }
            if let Some(p) = patch.get("p") {
                let p = p
                    .as_object()
                    .ok_or_else(|| Error("3D properties must be an object".into()))?;
                for (k, v) in p {
                    if let Some((alias, path)) = TRANSFORMS.iter().find(|(a, _)| *a == k) {
                        let v = schema::validate_channel("object", alias, v.clone())?;
                        let scaled = transform(&json!({k:v}));
                        actions.push(Action::Channel {
                            id: id.clone(),
                            path: path.to_string(),
                            value: if p[k].is_object() {
                                scaled[*path].clone()
                            } else {
                                scaled[*path]["v"].clone()
                            },
                        });
                    } else if k == "visible" && operation != "set_environment" {
                        meta["visible"] = json!(visibility(v)?);
                    } else if operation != "update_object" {
                        let group = if operation == "set_environment" {
                            "environment"
                        } else {
                            role.key()
                        };
                        let value = schema::validate_channel(group, k, v.clone())?;
                        actions.push(Action::Channel {
                            id: id.clone(),
                            path: format!("{group}.{k}"),
                            value: if v.is_object() {
                                value
                            } else {
                                value["v"].clone()
                            },
                        });
                    } else {
                        return fail(format!("Unknown 3D property: {k}"));
                    }
                }
            }
            patch.as_object_mut().unwrap().remove("p");
            if role == Role::Object {
                if let Some(mat) = patch.get_mut("material") {
                    if let Some(p) = mat.get("p") {
                        for (k, v) in p
                            .as_object()
                            .ok_or_else(|| Error("Expected material properties".into()))?
                        {
                            let value = schema::validate_channel("material", k, v.clone())?;
                            actions.push(Action::Channel {
                                id: id.clone(),
                                path: format!("m.{k}"),
                                value: if v.is_object() {
                                    value
                                } else {
                                    value["v"].clone()
                                },
                            });
                        }
                    }
                    mat.as_object_mut()
                        .ok_or_else(|| Error("Expected material".into()))?
                        .remove("p");
                    for (k, v) in mat.as_object().unwrap() {
                        data["object"]["material"][k] = v.clone();
                    }
                }
                patch.as_object_mut().unwrap().remove("material");
            }
            let key = if operation == "set_environment" {
                "environment"
            } else {
                role.key()
            };
            for (k, v) in patch.as_object().unwrap() {
                data[key][k] = v.clone();
            }
            let checked = schema::layer_data(role, data)?;
            if checked != layer["d"]["data"] {
                actions.push(Action::Data {
                    id: id.clone(),
                    data: checked,
                });
            }
            if !meta.as_object().unwrap().is_empty() {
                actions.push(Action::Metadata {
                    id: id.clone(),
                    patch: meta,
                });
            }
        }
        _ => return fail("Unknown 3D operation"),
    }
    Ok(Plan {
        actions,
        id: Some(id),
    })
}
