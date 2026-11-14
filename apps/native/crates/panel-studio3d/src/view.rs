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
    // The Light panel's presets, colour temperature and Aim At.
    if let Some(key) = id.strip_prefix("light.adjust.")
        && matches!(key, "preset" | "temperature" | "aimAt")
    {
        let chosen = p.selection.layers().first().cloned();
        let comp = composition(p)?;
        let layers = comp["layers"].as_array().cloned().unwrap_or_default();
        let role_of = |id: &str| {
            layers
                .iter()
                .find(|l| l["id"] == id)
                .and_then(schema::role)
        };
        return match key {
            "preset" => {
                let preset = arg.as_str().ok_or_else(|| invalid("Choose a light preset"))?;
                // Lights the selected model, else the first model.
                let target = chosen.filter(|id| role_of(id) == Some(Role::Object));
                let mut args = json!({"operation":"light_preset","preset":preset});
                if let Some(target) = target {
                    args["target"] = json!(target);
                }
                let name = pm_scene3d::light_tool::preset_placement(preset, [0.; 3], 1., [0., 0., 5.])
                    .map_err(|e| invalid(e.0))?
                    .name;
                Ok((format!("Add {name}"), vec![scene(p, time, args)]))
            }
            "temperature" => {
                let light = chosen
                    .filter(|id| role_of(id) == Some(Role::Light))
                    .ok_or_else(|| invalid("Choose a light"))?;
                let kelvin = arg.as_f64().ok_or_else(|| invalid("Invalid temperature"))?;
                Ok((
                    "Light Temperature".into(),
                    vec![scene(
                        p,
                        time,
                        json!({"operation":"update_light","target":light,"patch":{"p":{"color":pm_scene3d::light_tool::kelvin_hex(kelvin)}}}),
                    )],
                ))
            }
            _ => {
                let light = chosen
                    .filter(|id| role_of(id) == Some(Role::Light))
                    .ok_or_else(|| invalid("Choose a light"))?;
                let at = arg.as_str().ok_or_else(|| invalid("Choose a model"))?;
                Ok((
                    "Aim Light".into(),
                    vec![scene(
                        p,
                        time,
                        json!({"operation":"aim_light","target":light,"at":at}),
                    )],
                ))
            }
        };
    }
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
    if let Some(control) = id.strip_prefix("scene.") {
        return scene_commands(p, time, control, arg);
    }
    if let Some(control) = id.strip_prefix("extrude.") {
        return crate::extrude::commands(p, time, control, arg, &artwork_lights);
    }
    if let Some(control) = id.strip_prefix("part.") {
        return crate::extrude::part_commands(p, time, control, arg);
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
        } else if arg
            .as_str()
            .is_some_and(|s| s.starts_with("layer:") || s.starts_with("comp:"))
        {
            if !matches!(slot, "color" | "emissive") {
                return Err(invalid("Layers can drive Color and Emissive maps"));
            }
            maps[slot] = crate::extrude::source_record(&arg);
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
                format!(
                    "Finish: {}",
                    pm_scene3d::service::presets()
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|f| f["id"] == arg)
                        .ok_or_else(|| invalid("Unknown finish"))?["label"]
                        .as_str()
                        .unwrap()
                ),
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
        (Role::Object, "world.mode" | "world.preset" | "world.visible") => {
            let mut world = json!({});
            match id {
                "world.mode" => {
                    if !["sky", "hdri", "none"].iter().any(|k| arg == *k) {
                        return Err(invalid("Invalid World mode"));
                    }
                    world["mode"] = arg;
                }
                "world.preset" => {
                    if !SKIES.iter().any(|(k, _, _)| arg == *k) {
                        return Err(invalid("Invalid sky"));
                    }
                    world["mode"] = json!("sky");
                    world["preset"] = arg;
                }
                _ => {
                    let on = arg.as_bool().ok_or_else(|| invalid("Expected checkbox"))?;
                    world["p"] = json!({"visible":on});
                }
            }
            return Ok((
                match id {
                    "world.mode" => "World Lighting",
                    "world.preset" => "Sky",
                    _ => "Show Sky in Camera",
                }
                .into(),
                vec![scene(
                    p,
                    time,
                    json!({"operation":"set_world","target":target,"world":world}),
                )],
            ));
        }
        (Role::Object, "scatter.model" | "scatter.surface") => {
            let key = &id["scatter.".len()..];
            let options = scatter_options(p, target, key)?;
            if !options
                .iter()
                .any(|o| o["v"] == arg && o["disabled"] != true)
            {
                return Err(invalid("Choose a model in this composition"));
            }
            return Ok((
                if key == "model" {
                    "Scatter Object"
                } else {
                    "Scatter Surface"
                }
                .into(),
                vec![scene(
                    p,
                    time,
                    json!({"operation":"update_scatter","target":target,"scatter":{key:arg}}),
                )],
            ));
        }
        (Role::Object, "fog.shape") => {
            if !["box", "sphere"].iter().any(|k| arg == *k) {
                return Err(invalid("Invalid fog shape"));
            }
            return Ok((
                "Fog Shape".into(),
                vec![scene(
                    p,
                    time,
                    json!({"operation":"update_fog","target":target,"fog":{"p":{"shape":arg}}}),
                )],
            ));
        }
        (Role::Object, "terrain.ridged" | "terrain.natural" | "terrain.adaptive") => {
            let on = arg.as_bool().ok_or_else(|| invalid("Expected checkbox"))?;
            let key = &id["terrain.".len()..];
            return Ok((
                match key {
                    "ridged" => "Ridged Peaks",
                    "natural" => "Natural Ground",
                    _ => "Adaptive Detail",
                }
                .into(),
                vec![scene(
                    p,
                    time,
                    json!({"operation":"update_terrain","target":target,"terrain":{"p":{key:on}}}),
                )],
            ));
        }
        (
            Role::Object,
            "options.castsShadows"
            | "options.acceptsShadows"
            | "options.acceptsLights"
            | "options.appearsInReflections",
        ) => {
            let key = &id["options.".len()..];
            if key == "castsShadows" {
                if !["off", "on", "only"].iter().any(|k| arg == *k) {
                    return Err(invalid("Invalid shadow option"));
                }
            } else if !arg.is_boolean() {
                return Err(invalid("Expected checkbox"));
            }
            return Ok((
                match key {
                    "castsShadows" => "Casts Shadows",
                    "acceptsShadows" => "Accepts Shadows",
                    "acceptsLights" => "Accepts Lights",
                    _ => "Appears in Reflections",
                }
                .into(),
                vec![scene(
                    p,
                    time,
                    json!({"operation":"set_material_options","target":target,"patch":{key:arg}}),
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

/// Sky presets: stored id, visible name and a one-line description.
pub const SKIES: &[(&str, &str, &str)] = &[
    ("clear-noon", "Clear Noon", "High sun, crisp shadows."),
    ("golden-hour", "Golden Hour", "Low warm sun, long shadows."),
    ("overcast", "Overcast", "Soft light with no hard shadows."),
    ("blue-hour", "Blue Hour", "Cool light just after sunset."),
    ("night", "Night", "Dark sky; your own lights lead."),
];
/// Per-composition render presets: id, name, and what each one trades.
pub const QUALITY: &[(&str, &str, &str)] = &[
    ("draft", "Draft", "Fast checks: 4–16 samples, no caustics."),
    ("preview", "Preview", "Clean motion tests: 16–64 samples."),
    (
        "production",
        "Production",
        "Delivery: 32–256 samples, caustics.",
    ),
    ("final", "Final", "Stills and hero shots: 512–1024 samples."),
];
pub const NATURE: &[(&str, &str)] = &[
    ("conifer", "Pine Trees"),
    ("broadleaf", "Oak Trees"),
    ("birch", "Birch Trees"),
    ("bush", "Bushes"),
    ("grass", "Grass"),
    ("rock", "Rocks"),
];
/// Layer ▸ Add 3D Layer, grouped as the menu draws it (empty id = separator).
pub fn new_menu() -> Vec<(&'static str, &'static str)> {
    vec![
        ("box", "Cube"),
        ("sphere", "Sphere"),
        ("plane", "Plane"),
        ("cylinder", "Cylinder"),
        ("cone", "Cone"),
        ("torus", "Torus"),
        ("capsule", "Capsule"),
        ("icosahedron", "Icosphere"),
        ("", ""),
        ("area", "Softbox"),
        ("spot", "Spotlight"),
        ("point", "Bulb"),
        ("sun", "Sun"),
        ("camera", "Camera"),
        ("", ""),
        ("sky", "Sky"),
        ("fog", "Fog Volume"),
        ("terrain", "Terrain"),
    ]
}
/// Composition-level Studio edits: no selected 3D layer is needed.
fn scene_commands(
    p: &Published,
    time: f64,
    id: &str,
    arg: Value,
) -> Result<(String, Vec<Command>)> {
    match id {
        "quality" => {
            let name = arg
                .as_str()
                .ok_or_else(|| invalid("Choose a quality preset"))?;
            let label = QUALITY
                .iter()
                .find(|(k, _, _)| *k == name)
                .ok_or_else(|| invalid("Unknown quality preset"))?
                .1;
            Ok((
                format!("3D Quality: {label}"),
                vec![scene(
                    p,
                    time,
                    json!({"operation":"set_render_quality","preset":name}),
                )],
            ))
        }
        "sky" => {
            let name = arg.as_str().ok_or_else(|| invalid("Choose a sky"))?;
            let world = if name == "none" {
                json!({"mode":"none"})
            } else {
                SKIES
                    .iter()
                    .find(|(k, _, _)| *k == name)
                    .ok_or_else(|| invalid("Unknown sky"))?;
                json!({"mode":"sky","preset":name})
            };
            Ok((
                if name == "none" {
                    "Remove Sky".into()
                } else {
                    "Sky".into()
                },
                vec![scene(
                    p,
                    time,
                    json!({"operation":"set_world","world":world}),
                )],
            ))
        }
        "shot" => {
            let label = shots()
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["id"] == arg)
                .and_then(|a| a["label"].as_str().map(str::to_owned))
                .ok_or_else(|| invalid("Unknown shot"))?;
            Ok((
                format!("Add Shot: {label}"),
                vec![scene(p, time, json!({"operation":"shot","shot":arg}))],
            ))
        }
        _ => Err(invalid("Unknown Studio scene control")),
    }
}
/// Set Up Studio (3d-studio.md §8): softboxes, a framing camera and the
/// seamless set around what's in the scene, from its evaluated geometry.
/// Imported models need loaded geometry; without it this says so.
/// A model's size from what the import measured (`native3dBounds`), or the
/// unit envelope older imports were fitted into: enough to frame a studio
/// set without loading the file.
struct RetainedBounds;
impl pm_scene3d::geometry::AssetGeometry for RetainedBounds {
    fn bounds(
        &self,
        _id: &str,
        record: &Value,
        _time: f64,
    ) -> pm_scene3d::Result<pm_scene3d::navigation::Bounds> {
        if record["native3dBounds"]["version"] == 1
            && let Some(v) = record["native3dBounds"]["bounds"]
                .as_array()
                .filter(|a| a.len() == 6)
                .and_then(|a| a.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>())
        {
            let b = pm_scene3d::navigation::Bounds {
                min: [v[0], v[1], v[2]],
                max: [v[3], v[4], v[5]],
            };
            if b.validate().is_ok() {
                return Ok(b);
            }
        }
        Ok(pm_scene3d::navigation::Bounds {
            min: [-1.; 3],
            max: [1.; 3],
        })
    }
}
pub fn set_up_studio(p: &Published, time: f64) -> Result<(String, Vec<Command>)> {
    let comp = composition(p)?;
    let layers = comp["layers"].as_array().cloned().unwrap_or_default();
    // The set frames the product: the selected model, else the first model
    // that isn't an environment or a flat plane (a floor would make the set
    // the size of the floor).
    let product = |l: &Value| {
        schema::role(l) == Some(Role::Object)
            && fields::environment(&l["d"]["data"]).is_none()
            && l["d"]["data"]["object"]["source"]["primitive"] != "plane"
    };
    let hero = layers
        .iter()
        .find(|l| p.selection.layers().iter().any(|id| l["id"] == id.as_str()) && product(l))
        .or_else(|| layers.iter().find(|l| product(l)))
        .and_then(|l| l["id"].as_str())
        .map(str::to_owned);
    let mut args = json!({"operation":"auto_studio"});
    if let Some(hero) = &hero {
        args["target"] = json!(hero);
    }
    // The starter light, sky and floor a first model brought give way to the
    // studio set, so there's one lighting setup, not two.
    let starter: Vec<String> = layers
        .iter()
        .filter_map(|l| l["id"].as_str())
        .filter(|id| {
            ["w", "k", "f"].iter().any(|suffix| {
                id.strip_suffix(suffix)
                    .is_some_and(|model| layers.iter().any(|m| m["id"] == model && product(m)))
            })
        })
        .map(str::to_owned)
        // A studio set from before is replaced, not doubled.
        .chain(
            layers
                .iter()
                .filter(|l| {
                    schema::role(l) == Some(Role::Light)
                        && matches!(
                            l["name"].as_str(),
                            Some("Key softbox" | "Fill softbox" | "Rim strip")
                        )
                })
                .filter_map(|l| l["id"].as_str().map(str::to_owned)),
        )
        .collect();
    let geometry = pm_kernel::commands::scene3d::prepare_geometry(
        &p.snapshot,
        p.snapshot.open_comp(),
        &to_model(&args),
        p.selection.layers(),
        time,
        &RetainedBounds,
    )
    .map_err(|e| {
        let e = e.to_string();
        if e.contains("not loaded") {
            invalid("Set Up Studio needs the model’s geometry, which isn’t loaded yet.")
        } else {
            invalid(e)
        }
    })?;
    Ok((
        "Set Up Studio".into(),
        [Scene3dCommand::ApplyPrepared {
                comp: p.snapshot.open_comp(),
                args: to_model(&args),
                selection: p.selection.layers().to_vec(),
                time,
                autokey: false,
                geometry,
            }
            .into()]
        .into_iter()
        .chain((!starter.is_empty()).then(|| {
            pm_kernel::LayerCommand::Delete {
                layer_ids: starter,
            }
            .into()
        }))
        .collect(),
    ))
}
/// What the composition's 3D summary shows, or `None` when it has no 3D layer:
/// its saved render quality and its World's sky (`none`, a preset or `hdri`).
pub fn scene_summary(p: &Published) -> Result<Option<Value>> {
    let comp = composition(p)?;
    let layers = comp["layers"].as_array().cloned().unwrap_or_default();
    if !layers.iter().any(|l| schema::role(l).is_some()) {
        return Ok(None);
    }
    let quality = pm_scene3d::quality::get(&comp)?;
    let world = layers.iter().find_map(|l| {
        l["d"]["data"]["object"]
            .get("world")
            .filter(|w| w.is_object())
    });
    let sky = match world {
        None => "none".to_string(),
        Some(w) if w["mode"] == "sky" => w["preset"].as_str().unwrap_or("clear-noon").into(),
        Some(w) => w["mode"].as_str().unwrap_or("none").into(),
    };
    let models = layers
        .iter()
        .filter(|l| {
            schema::role(l) == Some(Role::Object) && fields::environment(&l["d"]["data"]).is_none()
        })
        .count();
    let lights = layers
        .iter()
        .filter(|l| schema::role(l) == Some(Role::Light))
        .count();
    Ok(Some(
        json!({"quality":quality,"sky":sky,"models":models,"lights":lights,
        "terrain":layers.iter().any(|l| fields::environment(&l["d"]["data"]) == Some("terrain"))}),
    ))
}
/// A rough per-frame export time for a quality policy at a pixel size, from
/// the measured M5 worker timings (docs/native/studio3d-port.md): tracing
/// scales with samples and pixels, OIDN adds a fixed cost per frame.
pub fn estimate_seconds_per_frame(quality: &Value, width: f64, height: f64) -> f64 {
    let max = quality["maxSamples"].as_f64().unwrap_or(256.);
    let min = quality["minSamples"].as_f64().unwrap_or(32.);
    let pixels = (width * height / (1920. * 1080.)).max(0.05);
    // Adaptive stopping usually lands between the floor and the ceiling.
    let samples = min + (max - min) * 0.35;
    // Hero Production 256 traced in ~2.1 s; ~1.3 s OIDN + readback per frame.
    let trace = 2.07 * samples / (32. + (256. - 32.) * 0.35) * pixels;
    let denoise = if quality["denoise"] == false {
        0.
    } else {
        1.3 * pixels.max(0.25)
    };
    trace + denoise + 0.1
}

/// Layer ▸ Add 3D Layer: the label, commands and the layer to select afterwards.
/// `kind` is a New 3D menu id or `scatter:<species>`. `id` is a fresh layer ID
/// from the host; a scatter or a sky that already exists reuses its layer.
pub fn add_layer(
    p: &Published,
    time: f64,
    kind: &str,
    id: &str,
) -> Result<(String, Vec<Command>, String)> {
    let comp = composition(p)?;
    let layers = comp["layers"].as_array().cloned().unwrap_or_default();
    if let Some(species) = kind.strip_prefix("scatter:") {
        let label = NATURE
            .iter()
            .find(|(k, _)| *k == species)
            .ok_or_else(|| invalid("Unknown nature asset"))?
            .1;
        // A grove, not a wall: trees are large, grass is small.
        // A grove on a landscape that runs past the frame: trees about ten
        // units apart, small things denser.
        let count = match species {
            "conifer" | "broadleaf" | "birch" => 320,
            "bush" => 500,
            "rock" => 200,
            _ => 4000,
        };
        // Scatter on the selected model or terrain, else the first terrain;
        // with none, the same Undo step adds a terrain to grow on.
        let selected = p.selection.layers().first().cloned();
        let surface = layers
            .iter()
            .find(|l| {
                l["id"].as_str().map(str::to_owned) == selected
                    && schema::role(l) == Some(Role::Object)
                    && fields::environment(&l["d"]["data"]).is_none_or(|k| k == "terrain")
            })
            .or_else(|| {
                layers
                    .iter()
                    .find(|l| fields::environment(&l["d"]["data"]) == Some("terrain"))
            })
            .and_then(|l| l["id"].as_str().map(str::to_owned));
        let mut commands = vec![];
        let surface = match surface {
            Some(s) => s,
            None => {
                let terrain = format!("{id}t");
                commands.push(scene(
                    p,
                    time,
                    json!({"operation":"add_terrain","id":terrain,"terrain":{"p":{"natural":true,"erosion":10,"amplitude":2.5,"width":260,"depth":260,"resolution":160,"frequency":0.06}}}),
                ));
                terrain
            }
        };
        commands.push(scene(
            p,
            time,
            // The hidden model is named for what it is, one of what the
            // scatter repeats: "Pine Tree" under "Pine Trees".
            json!({"operation":"add_object","id":format!("{id}s"),"name":match species {
                "conifer" => "Pine Tree",
                "broadleaf" => "Oak Tree",
                "birch" => "Birch Tree",
                "bush" => "Bush",
                "grass" => "Grass Tuft",
                _ => "Rock",
            },"object":{"source":{"nature":species,"seed":1},"p":{"visible":false}}}),
        ));
        commands.push(scene(
            p,
            time,
            json!({"operation":"add_scatter","id":id,"name":label,"scatter":{"model":format!("{id}s"),"surface":surface,"p":{"count":count}}}),
        ));
        if let Some(camera) = overview_camera(&layers, id) {
            commands.push(scene(p, time, camera));
        }
        if let Some(sky) = landscape_sky(&layers, id).or_else(|| landscape_fog(&layers)) {
            commands.push(scene(p, time, sky));
        }
        return Ok((format!("Add {label}"), commands, id.into()));
    }
    // A light preset placed for the selected model (else the first) and the
    // shot: the same `light_preset` operation agents use headless.
    if let Some(preset) = kind.strip_prefix("preset:") {
        let name = pm_scene3d::light_tool::preset_placement(preset, [0.; 3], 1., [0., 0., 5.])
            .map_err(|e| invalid(e.0))?
            .name;
        let chosen = p
            .selection
            .layers()
            .first()
            .filter(|s| {
                layers
                    .iter()
                    .any(|l| l["id"] == s.as_str() && schema::role(l) == Some(Role::Object))
            })
            .cloned();
        let mut args = json!({"operation":"light_preset","preset":preset,"id":id});
        if let Some(target) = chosen {
            args["target"] = json!(target);
        }
        return Ok((format!("Add {name}"), vec![scene(p, time, args)], id.into()));
    }
    let label = new_menu()
        .into_iter()
        .find(|(k, _)| *k == kind && !k.is_empty())
        .ok_or_else(|| invalid("Unknown 3D layer"))?
        .1;
    let world = layers
        .iter()
        .find(|l| fields::environment(&l["d"]["data"]) == Some("world"))
        .and_then(|l| l["id"].as_str().map(str::to_owned));
    let args = match kind {
        "camera" => json!({"operation":"add_camera","id":id}),
        "area" | "spot" | "point" | "sun" => {
            json!({"operation":"add_light","id":id,"name":label,"light":{"type":kind}})
        }
        "sky" => {
            json!({"operation":"set_world","id":world.as_deref().unwrap_or(id),"world":{"mode":"sky","preset":"clear-noon"}})
        }
        "fog" => json!({"operation":"add_fog","id":id}),
        "terrain" => {
            json!({"operation":"add_terrain","id":id,"terrain":{"p":{"natural":true,"erosion":10,"amplitude":2.5,"width":260,"depth":260,"resolution":160,"frequency":0.06}}})
        }
        _ => {
            json!({"operation":"add_object","id":id,"name":label,"object":{"source":{"primitive":kind}}})
        }
    };
    let mut commands = vec![scene(p, time, args)];
    if kind == "terrain" {
        if let Some(camera) = overview_camera(&layers, id) {
            commands.push(scene(p, time, camera));
        }
        if let Some(sky) = landscape_sky(&layers, id).or_else(|| landscape_fog(&layers)) {
            commands.push(scene(p, time, sky));
        }
    }
    let shape = new_menu().iter().take(8).any(|(k, _)| *k == kind);
    if shape {
        commands.extend(starter(&layers, id).into_iter().map(|a| scene(p, time, a)));
    }
    let select = if kind == "sky" {
        world.unwrap_or_else(|| id.into())
    } else {
        id.into()
    };
    Ok((format!("Add {label}"), commands, select))
}
/// The 3D quality an export renders with, for a composition (`None` without 3D
/// layers): `choice` is the sheet's per-export override, or `composition` /
/// absent for the composition's saved preset. Returns the policy and its name.
pub fn export_quality(comp: &Value, choice: Option<&str>) -> Option<(Value, String)> {
    let layers = comp["layers"].as_array()?;
    if !layers.iter().any(|l| schema::role(l).is_some()) {
        return None;
    }
    let saved = pm_scene3d::quality::get(comp).ok()?;
    let policy = match choice {
        Some(name) if name != "composition" => pm_scene3d::quality::preset(name).ok()?,
        _ => saved,
    };
    let name = policy["preset"].as_str().unwrap_or("custom");
    let label = QUALITY
        .iter()
        .find(|(k, _, _)| *k == name)
        .map_or("Custom", |q| q.1)
        .to_string();
    Some((policy, label))
}
/// "About 7 min" style copy for a duration in seconds.
pub fn about(seconds: f64) -> String {
    if seconds < 90. {
        format!("about {} s", seconds.round().max(1.))
    } else if seconds < 90. * 60. {
        format!("about {} min", (seconds / 60.).round())
    } else {
        let h = seconds / 3600.;
        format!("about {} h", (h * 10.).round() / 10.)
    }
}

/// A landscape fills the default editor view from inside. With no camera yet,
/// add one standing back and above, aimed at the middle, so the first look is
/// the whole scene.
fn overview_camera(layers: &[Value], id: &str) -> Option<Value> {
    if layers.iter().any(|l| schema::role(l) == Some(Role::Camera)) {
        return None;
    }
    // Far and high enough to see distinct trees against the sky.
    Some(
        json!({"operation":"add_camera","id":format!("{id}c"),"camera":{"p":{
        "x":0,"y":14,"z":46,"targetX":0,"targetY":3,"targetZ":0,"fov":35}}}),
    )
}
/// A landscape with no sky gets a clear day; its ground colour matches the
/// meadow so the far terrain edge doesn't read as a strip. (Atmospheric fog
/// darkens the whole foreground in Quality for now; see the port doc.)
fn landscape_sky(layers: &[Value], id: &str) -> Option<Value> {
    if layers
        .iter()
        .any(|l| fields::environment(&l["d"]["data"]) == Some("world"))
    {
        return None;
    }
    Some(
        json!({"operation":"set_world","id":format!("{id}w"),"world":{"mode":"sky","preset":"clear-noon",
        "p":{"elevation":38,"haze":3,"intensity":1,"sunIntensity":0.9,"groundColor":"#5F6B45",
        "fogHeightFalloff":LANDSCAPE_FOG.0,"fogDistance":LANDSCAPE_FOG.1}}}),
    )
}
/// A landscape's atmosphere shape: fog that thins with height and ends past
/// the terrain's far edge (400 units from the camera), shaped like the
/// engine's landscape reference. Density stays 0 (no fog) until the user
/// raises it. The schema's
/// defaults (even fog filling 10,000 units) black out the sun and sky in
/// Quality at any density a user would try.
const LANDSCAPE_FOG: (f64, f64) = (0.06, 400.);
/// An existing World gets the landscape fog shape when its own is still the
/// default (neither channel stored).
fn landscape_fog(layers: &[Value]) -> Option<Value> {
    let world = layers
        .iter()
        .find(|l| fields::environment(&l["d"]["data"]) == Some("world"))?;
    let p = &world["d"]["data"]["object"]["world"]["p"];
    // A World stores every channel; untouched means its default value, with
    // no keyframes or expression.
    let untouched = |key: &str, default: f64| {
        let c = &p[key];
        c.is_null()
            || c.as_f64() == Some(default)
            || (c["v"].as_f64() == Some(default)
                && c["kf"].as_array().is_none_or(Vec::is_empty)
                && c["expr"].is_null())
    };
    if !untouched("fogDensity", 0.) || !untouched("fogBaseHeight", 0.)
        || !untouched("fogHeightFalloff", 0.) || !untouched("fogDistance", 10000.)
        || ["fogDensity", "fogBaseHeight", "fogHeightFalloff", "fogDistance"].iter()
            .any(|key| !world["locked_intent"][format!("world.{key}")].is_null()) {
        return None;
    }
    Some(json!({"operation":"set_world","id":world["id"],"world":{"p":{
        "fogHeightFalloff":LANDSCAPE_FOG.0,"fogDistance":LANDSCAPE_FOG.1}}}))
}
/// The first object in an empty scene arrives lit (3d-studio.md's product
/// film): a clear sky, a key softbox placed for a highlight, and a floor so
/// it has a contact shadow and a reflection. Nothing is added
/// when the composition already has a light or a sky.
fn starter(layers: &[Value], id: &str) -> Vec<Value> {
    let lit = layers.iter().any(|l| {
        schema::role(l) == Some(Role::Light)
            || fields::environment(&l["d"]["data"]) == Some("world")
    });
    if lit {
        return vec![];
    }
    vec![
        json!({"operation":"set_world","id":format!("{id}w"),"world":{"mode":"sky","preset":"clear-noon","p":{"elevation":35,"intensity":0.6,"sunIntensity":0.5,"haze":3}}}),
        json!({"operation":"add_light","id":format!("{id}k"),"name":"Key","light":{"type":"area","p":{
            "x":-2.6,"y":3.4,"z":3.2,"targetX":0,"targetY":0.3,"targetZ":0,"width":2.4,"height":1.6,"intensity":9}}}),
        // A real floor under the object's base for its contact shadow and a
        // soft reflection (the renderer's shadow catcher draws a black void
        // for now; see the port doc).
        json!({"operation":"add_object","id":format!("{id}f"),"name":"Floor","object":{
            "source":{"primitive":"plane","parameters":{"width":200,"height":200}},
            "p":{"y":-0.5,"rx":-90},
            "material":{"p":{"color":"#8A8781","roughness":0.32,"metalness":0}}}}),
    ]
}

/// Light for extruded artwork in an unlit composition: the soft sky as
/// lighting only, so the design's own background stays, and a key light up
/// and to the left. Nothing when the composition already has light.
fn artwork_lights(layers: &[Value], id: &str) -> Vec<Value> {
    let mut out = starter(layers, id);
    out.truncate(2);
    if let Some(world) = out.first_mut() {
        world["world"]["p"]["visible"] = json!(false);
        world["world"]["p"]["intensity"] = json!(0.8);
    }
    out
}

/// Import 3D Model…: one model layer per imported model asset, in one Undo
/// step. `assets` pairs each asset ID with a fresh layer ID from the host.
/// Imported glTF/FBX keep their own materials; OBJ starts on the Studio
/// material, as the kernel's import does.
pub fn add_models(
    p: &Published,
    time: f64,
    assets: &[(String, String)],
) -> Result<(String, Vec<Command>, Vec<String>)> {
    let root = from_model(
        &p.snapshot
            .root()
            .ok_or_else(|| invalid("No project"))?
            .to_value(),
    )?;
    let mut commands = vec![];
    let mut names = vec![];
    let mut select = vec![];
    for (asset, layer) in assets {
        let record = &root["assets"][asset];
        if record["kind"] != "model" {
            continue;
        }
        let file = record["name"].as_str().unwrap_or("3D Model").to_string();
        let obj = file.to_ascii_lowercase().ends_with(".obj");
        // The layer is named for the model, not its file: "Bottle".
        let name = std::path::Path::new(&file)
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&file)
            .to_string();
        commands.push(scene(
            p,
            time,
            json!({"operation":"add_object","id":layer,"name":name,
                "object":{"source":{"assetId":asset},"useSourceMaterials":!obj}}),
        ));
        names.push(name);
        select.push(layer.clone());
    }
    // The first model in an unlit composition arrives lit, as a first shape
    // does: a soft sky, a key softbox and a floor for its contact shadow.
    if let Some(first) = select.first() {
        let comp = composition(p)?;
        let layers = comp["layers"].as_array().cloned().unwrap_or_default();
        for args in starter(&layers, first) {
            commands.push(scene(p, time, args));
        }
    }
    let label = match names.as_slice() {
        [] => return Err(invalid("No 3D models were imported")),
        [one] => format!("Import {one}"),
        many => format!("Import {} 3D Models", many.len()),
    };
    Ok((label, commands, select))
}

/// Scatter's Object and Surface choices: the composition's models (a hidden
/// scatter source reads as itself), and for Surface also terrains. The
/// scatter layer itself is never a choice.
pub fn scatter_options(p: &Published, scatter: &str, key: &str) -> Result<Vec<Value>> {
    let comp = composition(p)?;
    let mut out = vec![];
    for l in comp["layers"].as_array().into_iter().flatten() {
        if schema::role(l) != Some(Role::Object) || l["id"] == scatter {
            continue;
        }
        let env = fields::environment(&l["d"]["data"]);
        let ok = match key {
            "model" => env.is_none(),
            _ => env.is_none() || env == Some("terrain"),
        };
        if ok {
            // Older projects named the hidden model "… Source".
            let name = l["name"].as_str().unwrap_or("Model");
            out.push(json!({"v":l["id"],"label":name.trim_end_matches(" Source")}));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod landscape_fog_tests {
    use super::*;
    fn world(p: Value) -> Value {
        json!({"id":"w1","d":{"data":{"object":{"world":{"mode":"sky","preset":"clear-noon","p":p}}}}})
    }
    #[test]
    fn a_default_world_gets_the_landscape_fog_shape_once() {
        let op = landscape_fog(&[world(json!({"elevation":45}))]).unwrap();
        assert_eq!(op["operation"], "set_world");
        assert_eq!(op["id"], "w1");
        assert_eq!(op["world"]["p"]["fogHeightFalloff"], 0.06);
        assert_eq!(op["world"]["p"]["fogDistance"], 400.);
        assert!(op["world"].get("preset").is_none(), "the sky the user chose stays");
        // A shape the user (or an earlier terrain) set is left alone.
        assert!(landscape_fog(&[world(json!({"fogDistance":50}))]).is_none());
        // Stored defaults (every channel is materialised) still count as untouched.
        let stored = json!({"fogDistance":{"expr":null,"kf":[],"v":10000},"fogHeightFalloff":{"expr":null,"kf":[],"v":0}});
        assert!(landscape_fog(&[world(stored)]).is_some());
        let keyed = json!({"fogDistance":{"expr":null,"kf":[[0,10000]],"v":10000}});
        assert!(landscape_fog(&[world(keyed)]).is_none());
        assert!(landscape_fog(&[world(json!({"fogDensity":0.003}))]).is_none());
        assert!(landscape_fog(&[world(json!({"fogBaseHeight":3}))]).is_none());
        assert!(landscape_fog(&[]).is_none());
        // A new landscape sky carries the shape from the start.
        let sky = landscape_sky(&[], "t1").unwrap();
        assert_eq!(sky["world"]["p"]["fogHeightFalloff"], 0.06);
    }
}
