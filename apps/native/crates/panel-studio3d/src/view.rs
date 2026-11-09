//! Projections and typed commands for Opus 3d-studio.md. No renderer/UI fallback.
use crate::{Result, fields, invalid};
use pm_kernel::{Command, Published, QueryMirror, Scene3dCommand};
use pm_scene3d::{
    from_model,
    schema::{self, Role},
    to_model,
};
use serde_json::{Value, json};
fn composition(p: &Published) -> Result<Value> {
    let root = p.snapshot.root().ok_or_else(|| invalid("No project"))?;
    from_model(
        &pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref())
            .ok_or_else(|| invalid("No composition"))?
            .to_value(),
    )
    .map_err(Into::into)
}
fn selected(p: &Published) -> Result<Value> {
    if p.selection.layers().len() != 1 {
        return Err(invalid("Choose one 3D layer"));
    }
    composition(p)?["layers"]
        .as_array()
        .and_then(|layers| layers.iter().find(|l| l["id"] == p.selection.layers()[0]))
        .cloned()
        .ok_or_else(|| invalid("Layer disappeared"))
}
pub(crate) fn orthographic(p: &Published) -> Result<bool> {
    Ok(composition(p)?["layers"]
        .as_array()
        .ok_or_else(|| invalid("No layers"))?
        .iter()
        .find(|l| schema::role(l) == Some(Role::Camera) && l["on"] != false)
        .is_some_and(|l| l["d"]["data"]["camera"]["projection"] == "orthographic"))
}
pub(crate) fn allow_transform(p: &Published, action: &str) -> Result<()> {
    let comp = composition(p)?;
    let layers = comp["layers"]
        .as_array()
        .ok_or_else(|| invalid("No layers"))?;
    for id in p.selection.layers() {
        let l = layers
            .iter()
            .find(|l| l["id"] == *id)
            .ok_or_else(|| invalid("Layer disappeared"))?;
        let role = schema::role(l).ok_or_else(|| invalid("Not available for this layer."))?;
        let kind = l["d"]["data"]["light"]["type"].as_str();
        if l["lock"] == true
            || (role == Role::Camera && action == "scale")
            || (role == Role::Light
                && ((kind == Some("sun") && action != "rotate")
                    || (kind == Some("point") && action != "move")))
        {
            return Err(invalid(if l["lock"] == true {
                "Layer is locked"
            } else {
                "Not available for this layer."
            }));
        }
    }
    Ok(())
}
pub(crate) fn inspector(
    p: &Published,
    comp: &Value,
    l: &Value,
    time: f64,
    mirror: &mut QueryMirror,
) -> Result<Value> {
    let Some(role) = schema::role(l) else {
        if l.get("native3dUpgradeDiagnostic").is_some() {
            return Ok(
                json!({"id":l["id"],"error":"This 3D layer couldn’t be upgraded.","diagnostic":l["native3dUpgradeDiagnostic"]}),
            );
        }
        return Ok(Value::Null);
    };
    let data = &l["d"]["data"];
    let mut rows = fields::catalog(role.key(), data);
    let index = comp["layers"]
        .as_array()
        .unwrap()
        .iter()
        .position(|a| a["id"] == l["id"])
        .unwrap();
    for f in &mut rows {
        let c = fields::channel(data, f.channel.as_deref().unwrap());
        f.value=from_model(&mirror.queries().query(&to_model(&json!({"op":"evP","comp":p.snapshot.open_comp(),"open":p.snapshot.open_comp(),"layer":index,"t":time,"key":f.channel,"prop":c}))).map_err(|e|invalid(e.to_string()))?)?;
        f.animated = c["kf"].as_array().is_some_and(|k| !k.is_empty());
        let local = ((time - l["from"].as_f64().unwrap_or(0.))
            * comp["fps"].as_f64().unwrap_or(30.))
        .round()
            / comp["fps"].as_f64().unwrap_or(30.);
        let half = 0.5 / comp["fps"].as_f64().unwrap_or(30.);
        f.keyed = c["kf"].as_array().is_some_and(|k| {
            k.iter()
                .any(|k| k["t"].as_f64().is_some_and(|t| (t - local).abs() < half))
        });
    }
    Ok(
        json!({"id":l["id"],"role":role.key(),"name":l["name"],"locked":l["lock"]==true,"data":data,"fields":rows,
        "screens":screen_options(p,l["id"].as_str().unwrap_or(""))?,"source":data["object"]["source"]}),
    )
}
pub fn add_menu() -> Value {
    json!([
    {"id":"box","label":"Cube"},{"id":"sphere","label":"Sphere"},{"id":"plane","label":"Plane"},{"id":"cylinder","label":"Cylinder"},{"id":"cone","label":"Cone"},{"id":"torus","label":"Torus"},{"id":"capsule","label":"Capsule"},{"id":"icosahedron","label":"Icosphere"},
    {"id":"import","label":"Import 3D Model…"},{"id":"area","label":"Softbox"},{"id":"spot","label":"Spotlight"},{"id":"point","label":"Bulb"},{"id":"sun","label":"Sun"},{"id":"camera","label":"Camera"}])
}
pub fn shots() -> Value {
    json!([
    {"id":"turntable","label":"Turntable","detail":"The selected model or group turns 360°."},
    {"id":"push_in","label":"Push-In","detail":"The camera moves 30 % closer."},
    {"id":"orbit","label":"Orbit Reveal","detail":"The camera orbits 90° around its aim."},
    {"id":"hero_tilt","label":"Hero Tilt","detail":"The camera rises into a low hero angle."},
    {"id":"rack_focus","label":"Rack Focus","detail":"Focus moves from near to far."}])
}
/// A composition can contain a layer through nested composition layers or
/// through another model's screen. Refuse cycles before a typed kernel edit.
fn contains(
    comp: &str,
    target: &str,
    comps: &Value,
    seen: &mut std::collections::HashSet<String>,
) -> bool {
    if !seen.insert(comp.into()) {
        return false;
    }
    comps[comp]["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|l| {
            l["id"] == target
                || [
                    l["comp"].as_str(),
                    l["compId"].as_str(),
                    l["d"]["data"]["object"]["material"]["screen"].as_str(),
                ]
                .into_iter()
                .flatten()
                .any(|c| contains(c, target, comps, seen))
        })
}
pub(crate) fn screen_options(p: &Published, target: &str) -> Result<Vec<Value>> {
    let root = from_model(
        &p.snapshot
            .root()
            .ok_or_else(|| invalid("No project"))?
            .to_value(),
    )?;
    let mut out = vec![json!({"v":null,"label":"None","disabled":false})];
    for (id, comp) in root["comps"]
        .as_object()
        .ok_or_else(|| invalid("No compositions"))?
    {
        let disabled = p.snapshot.open_comp().as_deref() == Some(id)
            || contains(id, target, &root["comps"], &mut Default::default());
        out.push(json!({"v":id,"label":comp["name"].as_str().unwrap_or(""),"disabled":disabled,"detail":if disabled {Some("A composition can’t play inside itself.")}else{None}}));
    }
    Ok(out)
}
fn scene(p: &Published, time: f64, args: Value) -> Command {
    Scene3dCommand::Apply {
        comp: p.snapshot.open_comp(),
        args: to_model(&args),
        selection: p.selection.layers().to_vec(),
        time,
        autokey: false,
    }
    .into()
}
pub(crate) fn commands(
    p: &Published,
    time: f64,
    id: &str,
    arg: Value,
) -> Result<(String, Vec<Command>)> {
    if let Some(key) = id.strip_prefix("light.adjust.") {
        let label = match key {
            "brightness" => "Light Brightness",
            "size" => "Light Size",
            "distance" => "Light Distance",
            "spread" => "Light Spread",
            _ => return Err(invalid("Unknown light adjustment")),
        };
        return Ok((
            label.into(),
            vec![scene(p, time, crate::light::adjustment(p, time, key, arg)?)],
        ));
    }
    if id == "add" {
        let kind = arg.as_str().ok_or_else(|| invalid("Invalid layer kind"))?;
        let args = match kind {
            "camera" => json!({"operation":"add_camera"}),
            "area" | "spot" | "point" | "sun" => {
                json!({"operation":"add_light","light":{"type":kind}})
            }
            "box" | "sphere" | "plane" | "cylinder" | "cone" | "torus" | "capsule"
            | "icosahedron" => {
                json!({"operation":"add_object","object":{"source":{"primitive":kind}}})
            }
            _ => return Err(invalid("Import requires the native selected-file worker")),
        };
        return Ok((
            format!(
                "Add {}",
                add_menu()
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|a| a["id"] == kind)
                    .unwrap()["label"]
                    .as_str()
                    .unwrap()
            ),
            vec![scene(p, time, args)],
        ));
    }
    if id == "shot" {
        if !shots().as_array().unwrap().iter().any(|a| a["id"] == arg) {
            return Err(invalid("Unknown shot"));
        }
        return Ok((
            "Add Shot".into(),
            vec![scene(p, time, json!({"operation":"shot","shot":arg}))],
        ));
    }
    let l = selected(p)?;
    if l["lock"] == true {
        return Err(invalid("Layer is locked"));
    }
    let role = schema::role(&l).ok_or_else(|| invalid("Choose a 3D layer"))?;
    let target = l["id"]
        .as_str()
        .ok_or_else(|| invalid("Missing layer ID"))?;
    if let Some(path) = id.strip_prefix("field.") {
        let spec = fields::catalog(role.key(), &l["d"]["data"])
            .into_iter()
            .find(|f| f.id == path)
            .ok_or_else(|| invalid("Unknown Studio field"))?;
        if let Some(n) = arg.as_f64()
            && (spec.min.is_some_and(|lo| n < lo) || spec.max.is_some_and(|hi| n > hi))
        {
            return Err(invalid("Studio field outside range"));
        }
        let mut set =
            pm_kernel::commands::properties::SetProperty::new(target, path, to_model(&arg), time);
        set.mark_intent = true;
        return Ok((
            format!("Change {}", spec.label),
            vec![pm_kernel::PropertyCommand::Set(set).into()],
        ));
    }
    if let Some(path) = id.strip_prefix("key.") {
        let spec = fields::catalog(role.key(), &l["d"]["data"])
            .into_iter()
            .find(|f| f.id == path)
            .ok_or_else(|| invalid("Unknown Studio channel"))?;
        if !arg.is_null() {
            return Err(invalid("Unexpected keyframe argument"));
        }
        // Insert/remove at the playhead using the kernel's normal keyframe path.
        return Ok((
            format!("Keyframe {}", spec.label),
            vec![
                pm_kernel::KeyframeCommand::Toggle {
                    layer_id: target.into(),
                    channel: path.into(),
                    time,
                    key_id: format!("studio-key-{}-{}", p.snapshot.doc_id, p.revision()),
                    value: None,
                }
                .into(),
            ],
        ));
    }
    if let Some(slot) = id.strip_prefix("texture.") {
        if role != Role::Object
            || ![
                "color",
                "normal",
                "roughness",
                "metalness",
                "emissive",
                "ao",
            ]
            .contains(&slot)
        {
            return Err(invalid("Unknown texture slot"));
        }
        let mut maps = l["d"]["data"]["object"]["material"]["maps"].clone();
        if arg.is_null() {
            maps.as_object_mut()
                .ok_or_else(|| invalid("Invalid texture maps"))?
                .remove(slot);
        } else {
            let asset = arg
                .as_str()
                .ok_or_else(|| invalid("Choose an imported image"))?;
            let root = p.snapshot.root().ok_or_else(|| invalid("No project"))?;
            let record = root
                .get("assets")
                .and_then(|a| a.get(asset))
                .ok_or_else(|| invalid("Texture asset is not imported"))?;
            let record = from_model(&record.to_value())?;
            if record["kind"] != "image" {
                return Err(invalid("Choose an imported image"));
            }
            maps[slot] = arg.clone();
        }
        return Ok((
            (if arg.is_null() {
                "Remove Texture"
            } else {
                "Set Texture"
            })
            .into(),
            vec![scene(
                p,
                time,
                json!({"operation":"update_object","target":target,"patch":{"useSourceMaterials":false,"material":{"maps":maps}}}),
            )],
        ));
    }
    let mut patch = json!({});
    let (op, label) = match (role, id) {
        (Role::Object, "finish") => {
            return Ok((
                pm_scene3d::service::presets()
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|f| f["id"] == arg)
                    .ok_or_else(|| invalid("Unknown finish"))?["label"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                vec![scene(
                    p,
                    time,
                    json!({"operation":"apply_material","target":target,"preset":arg}),
                )],
            ));
        }
        (Role::Object, "screen") => {
            if !screen_options(p, target)?
                .iter()
                .any(|a| a["v"] == arg && a["disabled"] == false)
            {
                return Err(invalid("A composition can’t play inside itself."));
            }
            patch = json!({"material":{"screen":arg}});
            if !arg.is_null() {
                let strength = l["d"]["data"]["object"]["material"]["p"]["emissiveIntensity"]["v"]
                    .as_f64()
                    .unwrap_or(0.);
                if strength == 0. {
                    patch["material"]["p"] = json!({"emissiveIntensity":1});
                }
            }
            ("update_object", "Screen")
        }
        (Role::Object, "useSourceMaterials" | "castShadow" | "receiveShadow") => {
            if !arg.is_boolean() {
                return Err(invalid("Expected checkbox"));
            }
            patch[id] = arg;
            (
                "update_object",
                match id {
                    "useSourceMaterials" => "Model Materials",
                    "castShadow" => "Cast Shadows",
                    _ => "Receive Shadows",
                },
            )
        }
        (Role::Light, "type") => {
            if !["area", "spot", "point", "sun"].iter().any(|k| arg == *k) {
                return Err(invalid("Invalid light type"));
            }
            patch["type"] = arg;
            ("update_light", "Type")
        }
        (Role::Light, "castShadow") => {
            if !arg.is_boolean() {
                return Err(invalid("Expected checkbox"));
            }
            patch["castShadow"] = arg;
            ("update_light", "Cast Shadows")
        }
        (Role::Camera, "projection") => {
            if !["perspective", "orthographic"].iter().any(|k| arg == *k) {
                return Err(invalid("Invalid projection"));
            }
            patch["projection"] = arg;
            ("set_camera", "Projection")
        }
        (Role::Camera, "samples") => {
            if ![16, 64, 256, 1024].iter().any(|n| arg == *n)
                || l["d"]["data"]["camera"]["projection"] == "orthographic"
            {
                return Err(invalid("Invalid Final Quality"));
            }
            return Ok((
                "Final Quality".into(),
                vec![scene(
                    p,
                    time,
                    json!({"operation":"set_environment","patch":{"samples":arg}}),
                )],
            ));
        }
        (Role::Camera, "studio") => {
            if !arg.is_boolean() {
                return Err(invalid("Expected checkbox"));
            }
            return Ok((
                "Studio Set".into(),
                vec![scene(p, time, json!({"operation":"set_studio","on":arg}))],
            ));
        }
        (Role::Camera, "shadows" | "background" | "background.color") => {
            if id == "shadows" && !arg.is_boolean() {
                return Err(invalid("Expected checkbox"));
            }
            let (key, value) = match id {
                "background" => {
                    let on = arg.as_bool().ok_or_else(|| invalid("Expected checkbox"))?;
                    (
                        "background",
                        if on { json!("#161616") } else { Value::Null },
                    )
                }
                "background.color" => ("background", arg),
                _ => ("shadows", arg),
            };
            return Ok((
                (if id == "shadows" {
                    "Shadows"
                } else {
                    "Background"
                })
                .into(),
                vec![scene(
                    p,
                    time,
                    json!({"operation":"set_environment","patch":{key:value}}),
                )],
            ));
        }
        _ => return Err(invalid("Unknown Studio control")),
    };
    Ok((
        label.into(),
        vec![scene(
            p,
            time,
            json!({"operation":op,"target":target,"patch":patch}),
        )],
    ))
}
pub(crate) fn gesture_label(label: &str) -> bool {
    [
        "Move 3D Layer",
        "Move 3D Layers",
        "Rotate 3D Layer",
        "Rotate 3D Layers",
        "Scale 3D Layer",
        "Scale 3D Layers",
        "Aim Light",
        "Light Brightness",
        "Light Size",
        "Light Distance",
        "Light Spread",
        "Softbox Size",
    ]
    .contains(&label)
        || label.starts_with("Change ")
}
