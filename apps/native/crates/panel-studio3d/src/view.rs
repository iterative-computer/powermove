//! Projections and typed commands for Opus 3d-studio.md. No renderer/UI fallback.
use crate::{Result, fields, invalid};
use pm_kernel::{Command, Published, QueryMirror, Scene3dCommand};
use pm_scene3d::{
    from_model,
    navigation::{Vec3, sub},
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
/// Whether the open composition has a 3D layer (`schema::role`), read on
/// the document tree.
fn has_3d_layer(p: &Published) -> bool {
    let Some(comp) = p
        .snapshot
        .root()
        .and_then(|root| pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref()))
    else {
        return false;
    };
    let Some(pm_model::NodeKind::Array(layers)) = comp.get("layers").map(pm_model::Node::kind)
    else {
        return false;
    };
    layers.iter().any(|layer| {
        let definition = layer
            .get("d")
            .and_then(|d| d.get("definition"))
            .map(pm_model::Node::kind);
        let is_role = matches!(definition, Some(pm_model::NodeKind::String(s))
            if [schema::OBJECT, schema::LIGHT, schema::CAMERA]
                .iter()
                .any(|d| s.as_str() == Some(*d)));
        let is_studio_null = matches!(
            layer.get("type").map(pm_model::Node::kind),
            Some(pm_model::NodeKind::String(s)) if s.as_str() == Some("null")
        ) && matches!(
            layer.get("threeD").map(pm_model::Node::kind),
            Some(pm_model::NodeKind::Bool(true))
        ) && matches!(
            layer
                .get("d")
                .and_then(|d| d.get("studio3d"))
                .map(pm_model::Node::kind),
            Some(pm_model::NodeKind::Bool(true))
        );
        is_role || is_studio_null
    })
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
        let is_null_3d = l["type"] == "null" && l["threeD"] == true;
        if is_null_3d {
            if l["lock"] == true {
                return Err(invalid("Layer is locked"));
            }
            continue;
        }
        let role = schema::role(l).ok_or_else(|| invalid("Not available for this layer."))?;
        let kind = l["d"]["data"]["light"]["type"].as_str();
        if l["lock"] == true
            || (role == Role::Camera && action == "scale")
            || (role == Role::Light
                && ((kind == Some("sun") && action == "scale")
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
    {"id":"import","label":"Import 3D Model…"},{"id":"area","label":"Softbox"},{"id":"spot","label":"Spotlight"},{"id":"point","label":"Bulb"},{"id":"sun","label":"Sun"},{"id":"camera","label":"Camera"},{"id":"null","label":"3D Null"}])
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
/// The camera's channels at `time`, in its parent's space: position and
/// rotation (x, y, z) from the layer, its point of interest from its data.
fn camera_channels(p: &Published, time: f64, l: &Value) -> Result<(Vec3, Vec3, Vec3)> {
    let comp = composition(p)?;
    let index = comp["layers"]
        .as_array()
        .and_then(|ls| ls.iter().position(|x| x["id"] == l["id"]))
        .ok_or_else(|| invalid("Camera not in the composition"))?;
    let open = p.snapshot.open_comp();
    let id = l["id"].as_str().unwrap_or_default();
    let mut mirror = QueryMirror::new(p);
    let mut layer = |key: &str| -> Result<f64> {
        mirror
            .queries()
            .evaluate(None, id, key, time)
            .ok()
            .and_then(|v| from_model(&v).ok())
            .and_then(|v| v.as_f64())
            .ok_or_else(|| invalid("Unreadable camera channel"))
    };
    let position = [
        layer("position.x")?,
        layer("position.y")?,
        layer("position.z")?,
    ];
    let rotation = [
        layer("rotation.x")?,
        layer("rotation.y")?,
        layer("rotation")?,
    ];
    let mut mirror = QueryMirror::new(p);
    let mut data = |key: &str| -> Result<f64> {
        let q = json!({"op":"evP","comp":open,"open":open,"layer":index,"t":time,
            "key":format!("camera.{key}"),"prop":l["d"]["data"]["camera"]["p"][key]});
        mirror
            .queries()
            .query(&to_model(&q))
            .ok()
            .and_then(|v| from_model(&v).ok())
            .and_then(|v| v.as_f64())
            .ok_or_else(|| invalid("Unreadable point of interest"))
    };
    let target = [data("targetX")?, data("targetY")?, data("targetZ")?];
    Ok((position, rotation, target))
}
/// The camera's view direction for rotation (x, y) in degrees: −Z turned by
/// Rx then Ry, the order the evaluator composes them (Rz rolls only).
pub(crate) fn camera_forward(rotation: Vec3) -> Vec3 {
    let (a, b) = (rotation[0].to_radians(), rotation[1].to_radians());
    [-a.cos() * b.sin(), a.sin(), -a.cos() * b.cos()]
}
/// The rotation (x, y, 0) whose view direction is `forward`.
pub(crate) fn camera_rotation(forward: Vec3) -> Vec3 {
    let l = (forward[0].powi(2) + forward[1].powi(2) + forward[2].powi(2)).sqrt();
    let f = forward.map(|v| v / l.max(1e-12));
    [
        f[1].clamp(-1., 1.).asin().to_degrees(),
        (-f[0]).atan2(-f[2]).to_degrees(),
        0.,
    ]
}
/// Evaluated 3D position of a scene object layer at `time`.
fn object_position(p: &Published, time: f64, id: &str) -> Result<Vec3> {
    let mut mirror = QueryMirror::new(p);
    let mut axis = |key: &str| -> Result<f64> {
        mirror
            .queries()
            .evaluate(None, id, key, time)
            .ok()
            .and_then(|v| from_model(&v).ok())
            .and_then(|v| v.as_f64())
            .ok_or_else(|| invalid("Unreadable 3D object position"))
    };
    Ok([axis("position.x")?, axis("position.y")?, axis("position.z")?])
}
/// Options and active selection for the camera's `Focus On` picker:
/// Point of Interest (Auto), each 3D model layer in the composition, and
/// Custom Distance when `focusDistance` doesn't match any listed model.
pub fn dof_focus_options(
    p: &Published,
    camera_layer: &str,
    time: f64,
    focus_distance: f64,
) -> Result<(String, Vec<Value>)> {
    let comp = composition(p)?;
    let layers = comp["layers"].as_array().cloned().unwrap_or_default();
    let cam = layers
        .iter()
        .find(|l| l["id"] == camera_layer)
        .ok_or_else(|| invalid("Missing camera"))?;
    let cam_pos = camera_channels(p, time, cam).map(|(pos, _, _)| pos).ok();
    let mut options = vec![json!({
        "v": "target",
        "label": "Point of Interest (Auto)",
        "detail": "Keeps focus locked to the camera's point of interest."
    })];
    let mut matched: Option<String> = None;
    for l in &layers {
        if schema::role(l) != Some(Role::Object)
            || crate::fields::environment(&l["d"]["data"]).is_some()
        {
            continue;
        }
        let Some(id) = l["id"].as_str() else { continue };
        let name = l["name"].as_str().unwrap_or("3D Object");
        let v = format!("layer:{id}");
        if focus_distance > 0.
            && matched.is_none()
            && let (Some(cp), Ok(op)) = (cam_pos, object_position(p, time, id))
        {
            let dist = ((op[0] - cp[0]).powi(2)
                + (op[1] - cp[1]).powi(2)
                + (op[2] - cp[2]).powi(2))
            .sqrt();
            if (dist - focus_distance).abs() <= (focus_distance * 0.02).max(0.02) {
                matched = Some(v.clone());
            }
        }
        options.push(json!({"v": v, "label": name}));
    }
    let current = if focus_distance <= 0. {
        "target".to_owned()
    } else if let Some(m) = matched {
        m
    } else {
        options.push(json!({"v": "manual", "label": "Custom Distance", "disabled": true}));
        "manual".to_owned()
    };
    Ok((current, options))
}
/// Switches a camera between aiming at its point of interest (two-node) and
/// Auto-Orient Off (one-node), keeping the view exactly where it is: Off
/// writes the rotation that looks where the point of interest was; back on
/// puts the point of interest straight ahead at the same distance and clears
/// the rotation. Channels are set at `time`, as any Properties edit.
fn auto_orient(p: &Published, time: f64, l: &Value, id: &str, off: bool) -> Result<Vec<Command>> {
    let was_off = l["d"]["data"]["camera"]["autoOrient"] == "off";
    let mut commands = vec![];
    if off != was_off {
        let (position, rotation, target) = camera_channels(p, time, l)?;
        let d = (0..3)
            .map(|i| (target[i] - position[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        let set = |path: &str, v: f64| -> Command {
            let mut set = pm_kernel::commands::properties::SetProperty::new(
                id,
                path,
                to_model(&json!(v)),
                time,
            );
            // A deliberate overwrite (as a gizmo drag is): the conversion
            // replaces values the person set by hand.
            set.preserve_hand_edits = false;
            set.mark_intent = true;
            pm_kernel::PropertyCommand::Set(set).into()
        };
        if off {
            let r = camera_rotation(sub(target, position));
            for (path, v) in [
                ("rotation.x", r[0]),
                ("rotation.y", r[1]),
                ("rotation", rotation[2]),
            ] {
                commands.push(set(path, v));
            }
        } else {
            let f = camera_forward(rotation);
            let d = if d > 1e-6 { d } else { 10. };
            for (i, key) in ["targetX", "targetY", "targetZ"].iter().enumerate() {
                commands.push(set(&format!("camera.{key}"), position[i] + f[i] * d));
            }
            for path in ["rotation.x", "rotation.y"] {
                commands.push(set(path, 0.));
            }
        }
    }
    commands.push(scene(
        p,
        time,
        json!({"operation":"set_camera","target":id,"patch":{"autoOrient":if off {"off"} else {"target"}}}),
    ));
    Ok(commands)
}
/// The commands that store a flight (Fly navigation through the shot camera)
/// on the active camera, as one Undo: the camera moves by the flight, and
/// with Auto-Orient Off its rotation takes the flown heading and pitch (its
/// roll stays); aiming at a point of interest, that point moves so the
/// camera looks where the flight ended. `from` and `to` are world poses;
/// the camera's channels change by the same amounts, as a gizmo drag does.
pub fn fly_camera(
    p: &Published,
    time: f64,
    from: pm_scene3d::navigation::Pose,
    to: pm_scene3d::navigation::Pose,
) -> Result<(String, Vec<Command>)> {
    let comp = composition(p)?;
    let l = comp["layers"]
        .as_array()
        .and_then(|ls| {
            ls.iter()
                .find(|l| schema::role(l) == Some(Role::Camera) && l["on"] != false)
        })
        .cloned()
        .ok_or_else(|| invalid("This composition has no camera"))?;
    if l["lock"] == true {
        return Err(invalid("The camera is locked"));
    }
    let id = l["id"]
        .as_str()
        .ok_or_else(|| invalid("Missing camera ID"))?;
    let (position, rotation, target) = camera_channels(p, time, &l)?;
    let set = |path: &str, v: f64| -> Command {
        let mut set =
            pm_kernel::commands::properties::SetProperty::new(id, path, to_model(&json!(v)), time);
        set.preserve_hand_edits = false;
        set.mark_intent = true;
        pm_kernel::PropertyCommand::Set(set).into()
    };
    let moved = sub(to.position, from.position);
    let mut commands = vec![];
    for (i, path) in ["position.x", "position.y", "position.z"]
        .iter()
        .enumerate()
    {
        commands.push(set(path, position[i] + moved[i]));
    }
    if l["d"]["data"]["camera"]["autoOrient"] == "off" {
        let r = camera_rotation(sub(to.target, to.position));
        commands.push(set("rotation.x", r[0]));
        commands.push(set("rotation.y", r[1]));
        let _ = rotation;
    } else {
        let aimed = sub(to.target, from.target);
        for (i, key) in ["targetX", "targetY", "targetZ"].iter().enumerate() {
            commands.push(set(&format!("camera.{key}"), target[i] + aimed[i]));
        }
    }
    Ok(("Fly Camera".into(), commands))
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
pub fn active_part(selection: &pm_kernel::Selection) -> Option<(&str, &str)> {
    let [layer] = selection.layers() else {
        return None;
    };
    let (key, owner) = selection.channel()?.strip_prefix("studioPart:")?.split_once(':')?;
    (owner == layer && pm_scene3d::part_material::valid_part_key(key))
        .then_some((layer.as_str(), key))
}

pub fn part_commands(
    p: &Published,
    time: f64,
    layer: &str,
    key: &str,
    name: &str,
    values: &Value,
) -> Result<Vec<Command>> {
    let record = composition(p)?["layers"]
        .as_array()
        .and_then(|layers| layers.iter().find(|l| l["id"] == layer))
        .cloned()
        .ok_or_else(|| invalid("Model disappeared"))?;
    let mut out = vec![];
    if let Some(data) = pm_scene3d::part_material::install_part(&record, key, name)? {
        out.push(
            Scene3dCommand::SetData {
                layer_id: layer.into(),
                data: to_model(&data),
            }
            .into(),
        );
    }
    for (field, value) in values
        .as_object()
        .ok_or_else(|| invalid("Invalid part transform"))?
    {
        let validated = schema::validate_channel("part", field, value.clone())?;
        let mut set = pm_kernel::commands::properties::SetProperty::new(
            layer,
            format!("pt.{key}.{field}"),
            to_model(&validated["v"]),
            time,
        );
        set.preserve_hand_edits = false;
        set.mark_intent = true;
        out.push(pm_kernel::PropertyCommand::Set(set).into());
    }
    Ok(out)
}

pub fn select_part(layer: &str, key: Option<&str>) -> Result<pm_kernel::SelectionCall> {
    if key.is_some_and(|key| !pm_scene3d::part_material::valid_part_key(key)) {
        return Err(invalid("Choose a model part"));
    }
    Ok(pm_kernel::SelectionCall::Channel(
        key.map(|key| format!("studioPart:{key}:{layer}")),
    ))
}

pub(crate) fn commands(
    p: &Published,
    time: f64,
    id: &str,
    arg: Value,
) -> Result<(String, Vec<Command>)> {
    let id = id.strip_prefix("3d.").unwrap_or(id);
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
        if kind == "null" {
            let (label, commands, _) = add_layer(p, time, "null", &pm_kernel::new_id("null"))?;
            return Ok((label, commands));
        }
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
        if let Some((key, field)) = control.split_once('.')
            && let Some((_, label)) = pm_scene3d::part_material::TRANSFORM_FIELDS
                .iter()
                .find(|(property, _)| *property == field)
        {
            let record = selected(p)?;
            let layer = record["id"]
                .as_str()
                .ok_or_else(|| invalid("Missing layer ID"))?;
            let name = record["d"]["data"]["object"]["parts"][key]["name"]
                .as_str()
                .unwrap_or("");
            return Ok((
                format!("Part {label}"),
                part_commands(p, time, layer, key, name, &json!({(field):arg}))?,
            ));
        }
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
        (Role::Camera, "autoOrient") => {
            let off = match arg.as_str() {
                Some("off") => true,
                Some("target") => false,
                _ => return Err(invalid("Invalid Auto-Orient")),
            };
            return Ok(("Auto-Orient".into(), auto_orient(p, time, &l, target, off)?));
        }
        (Role::Camera, "projection") => {
            if !["perspective", "orthographic"].iter().any(|k| arg == *k) {
                return Err(invalid("Invalid projection"));
            }
            patch["projection"] = arg;
            ("set_camera", "Projection")
        }
        (Role::Camera, "dof.enabled") => {
            let on = arg.as_bool().ok_or_else(|| invalid("Expected checkbox"))?;
            let f_stop = if on { 2.8 } else { 0.0 };
            let mut set = pm_kernel::commands::properties::SetProperty::new(
                target,
                "camera.fStop",
                to_model(&json!(f_stop)),
                time,
            );
            set.preserve_hand_edits = false;
            set.mark_intent = true;
            return Ok(("Depth of Field".into(), vec![pm_kernel::PropertyCommand::Set(set).into()]));
        }
        (Role::Camera, "dof.focusTarget") => {
            let choice = arg.as_str().ok_or_else(|| invalid("Choose a focus target"))?;
            let dist = if choice == "target" {
                0.0
            } else if let Some(obj_id) = choice.strip_prefix("layer:") {
                let (cam_pos, _, _) = camera_channels(p, time, &l)?;
                let obj_pos = object_position(p, time, obj_id)?;
                ((obj_pos[0] - cam_pos[0]).powi(2)
                    + (obj_pos[1] - cam_pos[1]).powi(2)
                    + (obj_pos[2] - cam_pos[2]).powi(2))
                .sqrt()
                .max(0.05)
            } else {
                return Err(invalid("Invalid focus target"));
            };
            let mut set = pm_kernel::commands::properties::SetProperty::new(
                target,
                "camera.focusDistance",
                to_model(&json!((dist * 1000.).round() / 1000.)),
                time,
            );
            set.preserve_hand_edits = false;
            set.mark_intent = true;
            return Ok(("Focus Distance".into(), vec![pm_kernel::PropertyCommand::Set(set).into()]));
        }
        (Role::Camera, "dof.irisShape") => {
            let blades = match &arg {
                Value::String(s) => s.parse::<f64>().ok(),
                Value::Number(n) => n.as_f64(),
                _ => None,
            }
            .filter(|v| [0., 5., 6., 7., 8.].contains(v))
            .ok_or_else(|| invalid("Invalid Bokeh Shape"))?;
            let mut set = pm_kernel::commands::properties::SetProperty::new(
                target,
                "camera.irisShape",
                to_model(&json!(blades)),
                time,
            );
            set.preserve_hand_edits = false;
            set.mark_intent = true;
            return Ok(("Bokeh Shape".into(), vec![pm_kernel::PropertyCommand::Set(set).into()]));
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
        ("null", "3D Null"),
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
    // Most compositions have no 3D layer: answer that from the tree, without
    // converting every layer (and its keyframes) to JSON first.
    if !has_3d_layer(p) {
        return Ok(None);
    }
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
    if kind == "null" {
        let mut properties = pm_model::Map::new();
        for key in [
            "position.x",
            "position.y",
            "position.z",
            "anchor.x",
            "anchor.y",
            "anchor.z",
        ] {
            properties.insert(key, pm_model::Value::Number(0.0));
        }
        let mut content = pm_model::Map::new();
        content.insert("studio3d", pm_model::Value::Bool(true));
        let fps = comp["fps"]
            .as_f64()
            .filter(|f| f.is_finite() && *f > 0.)
            .unwrap_or(30.);
        let dur = comp["dur"].as_f64().unwrap_or(5.);
        let from = (time * fps).round() / fps;
        let duration = (dur - from).max(1.);
        let add = pm_kernel::commands::layers::AddLayer {
            id: id.into(),
            layer_type: "null".into(),
            comp: None,
            name: Some(label.into()),
            from: Some(from),
            duration: Some(duration),
            content,
            properties,
            color: None,
            index: None,
            parent: None,
            blend: None,
            motion_blur: None,
            visible: None,
            shy: None,
            collapsed: None,
            track: None,
            time,
        };
        let set_3d = pm_kernel::LayerCommand::Set {
            layer_id: id.into(),
            patch: pm_kernel::commands::layers::LayerPatch {
                three_d: Some(true),
                time,
                ..Default::default()
            },
        };
        return Ok((
            format!("Add {label}"),
            vec![pm_kernel::LayerCommand::Add(Box::new(add)).into(), set_3d.into()],
            id.into(),
        ));
    }
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
    let mut args = args;
    // The first 3D layer is seen by the default camera at (3, 2, 6), which
    // already looks at it three-quarters on: a cube this size spans about
    // 37% of the frame's height, and the other shapes are scaled to match.
    if !layers.iter().any(|l| schema::role(l).is_some()) {
        if kind == "box" {
            args["object"]["source"]["parameters"] =
                json!({"width":FIRST_CUBE,"height":FIRST_CUBE,"depth":FIRST_CUBE});
        } else if let Some(scale) = first_shape_scale(kind) {
            let s = (scale * 100.).round() / 100.;
            args["object"]["p"] = json!({"sx":s,"sy":s,"sz":s});
        }
    }
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
        commands.extend(key_light(&layers, id).into_iter().map(|a| scene(p, time, a)));
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
/// Side of the cube Layer ▸ Add 3D Layer adds to a composition with no 3D yet.
const FIRST_CUBE: f64 = 1.55;
/// The first shape's largest projected dimension, as a share of the frame's
/// height: what [`FIRST_CUBE`] spans.
const FIRST_SHAPE: f64 = 0.37;

/// The uniform scale at which a menu primitive, as built, spans
/// [`FIRST_SHAPE`] of the frame's height seen by the default camera (no
/// camera layer: at (3, 2, 6), aimed at the origin, 45° vertical view).
fn first_shape_scale(kind: &str) -> Option<f64> {
    let shape = pm_geom3d::primitives::Primitive::from_parameters(kind, &Default::default())?;
    let positions = shape.geometry().positions;
    let eye = [0., 0., 6.];
    let len = |a: [f64; 3]| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let f = [-eye[0] / len(eye), -eye[1] / len(eye), -eye[2] / len(eye)];
    let r = [-f[2], 0., f[0]].map(|x| x / len([-f[2], 0., f[0]]));
    let u = [
        r[1] * f[2] - r[2] * f[1],
        r[2] * f[0] - r[0] * f[2],
        r[0] * f[1] - r[1] * f[0],
    ];
    let t = (22.5f64).to_radians().tan();
    let extent = |scale: f64| {
        let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in positions.chunks_exact(3) {
            let d = [0, 1, 2].map(|k| p[k] as f64 * scale - eye[k]);
            let (x, y) = (dot(d, r) / dot(d, f) / t, dot(d, u) / dot(d, f) / t);
            (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
        }
        (x1 - x0).max(y1 - y0) / 2.
    };
    let mut scale = 1.;
    for _ in 0..4 {
        let e = extent(scale);
        if !(e > 0.) {
            return None;
        }
        scale *= FIRST_SHAPE / e;
    }
    Some(scale)
}

/// A shape added from Layer ▸ Add 3D Layer to an unlit composition gets the
/// key softbox and nothing else: no floor and no world over the
/// composition's 2D layers. Set Up Studio adds the full studio set.
fn key_light(layers: &[Value], id: &str) -> Vec<Value> {
    starter(layers, id)
        .into_iter()
        .filter(|a| a["operation"] == "add_light")
        .collect()
}

/// The first 3D model in a composition with no 3D layers yet arrives lit:
/// a clear sky, a key softbox, and a floor (when no 2D layers are present).
/// Once the composition already contains any 3D layer (model, light, camera,
/// or world), importing another 3D model adds nothing extra.
pub fn starter(layers: &[Value], id: &str) -> Vec<Value> {
    let has_3d = layers.iter().any(|l| {
        schema::role(l).is_some()
            || fields::environment(&l["d"]["data"]) == Some("world")
    });
    if has_3d {
        return vec![];
    }
    // Among 2D layers the 3D layer composites in stack order over a clear
    // background, as a 3D layer does in After Effects (and v1.2): the sky
    // lights it without being seen and there is no floor. The seamless set
    // is for a composition that is only 3D.
    let flat = layers
        .iter()
        .any(|l| schema::role(l).is_none() && l["type"] != "audio");
    let mut set = vec![
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
    ];
    if flat {
        set.truncate(2);
        set[0]["world"]["p"]["visible"] = json!(false);
    }
    set
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
        let obj = record["format"].as_str().map_or_else(
            || file.to_ascii_lowercase().ends_with(".obj"),
            |format| format == "obj",
        );
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

#[cfg(test)]
mod model_part_tests {
    use super::*;

    #[test]
    fn an_active_part_belongs_to_its_layer() {
        let pm_kernel::SelectionCall::Channel(channel) = select_part("chair", Some("0_1")).unwrap()
        else {
            panic!("part selection channel");
        };
        let selection = |layer: &str| pm_kernel::Selection::from_parts(
            vec![layer.into()], vec![], channel.clone(),
        );
        assert_eq!(active_part(&selection("chair")), Some(("chair", "0_1")));
        assert_eq!(active_part(&selection("desk")), None);
        assert!(select_part("chair", Some("0_01")).is_err());
    }

    #[test]
    fn a_part_command_can_be_keyframed_and_undone() {
        use pm_kernel::{Call, Caller, KernelActor, KernelConfig, OpenSource, Reply, Request, WriteBody};
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        let project = "part-test";
        let root = json!({"id":project,"name":"Part test","compId":"c","w":640,"h":360,
            "fps":30,"dur":5,"assets":{},"comps":{},"layers":[]});
        kernel.open(project, OpenSource::Untitled(to_model(&root))).wait().unwrap().unwrap();
        let call = |call: Call| {
            let ack = kernel.call(project, Request::new(Caller::native(), call)).wait().unwrap().unwrap();
            assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
        };
        let transact = |commands| call(Call::Transact { label: None, expected_revision: None,
            body: WriteBody::Commands(commands) });
        let read = || kernel.published(project).unwrap();
        transact(vec![scene(&read(), 0., json!({
            "operation":"add_object","id":"chair","object":{"source":{"assetId":"asset"}}
        }))]);
        call(pm_kernel::SelectionCall::Layers { ids:vec!["chair".into()], add:false }.into());

        let mut edits = part_commands(&read(), 0., "chair", "0_1", "Seat", &json!({"x":2})).unwrap();
        edits.push(pm_kernel::KeyframeCommand::Toggle {
            layer_id: "chair".into(), channel: "pt.0_1.x".into(), time: 0.,
            key_id: pm_kernel::new_id("kf"), value: None,
        }.into());
        transact(edits);
        transact(commands(&read(), 1., "3d.part.0_1.x", json!(6)).unwrap().1);

        let x = |time| {
            let p = read();
            let scene = pm_kernel::commands::scene3d::resolve_scene(
                &p.snapshot, p.snapshot.open_comp().as_deref(), time,
            ).unwrap();
            scene.scene["objects"].as_array().unwrap().iter()
                .find(|object| object["id"] == "chair").unwrap()["parts"]["0_1"]["p"]["x"]["v"]
                .as_f64().unwrap()
        };
        assert_eq!(x(0.), 2.);
        assert_eq!(x(0.5), 4.);
        assert_eq!(x(1.), 6.);
        call(pm_kernel::HistoryCall::Undo.into());
        assert_eq!(x(1.), 2.);
    }

}
