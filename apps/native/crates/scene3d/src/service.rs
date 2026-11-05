//! Renderer-independent scene assembly and Studio's compound document edits.
use crate::{
    Error, Result, fail,
    navigation::*,
    operations::{self, Action, Plan},
    schema::{self, Role},
};
use serde_json::{Value, json};
// Kept explicit at the kernel boundary; fields are the stored add-layer contract.
#[allow(clippy::too_many_arguments)]
pub fn new_layer(
    comp: &Value,
    id: &str,
    name: &str,
    role: Role,
    data: Value,
    properties: Value,
    visible: bool,
    parent: Option<String>,
    time: f64,
) -> Result<Value> {
    let position = match role {
        Role::Object => [0., 0., 0.],
        Role::Light => [-3., 5., 4.],
        Role::Camera => [3., 2., 6.],
    };
    let mut p = json!({});
    for (alias, path) in schema::TRANSFORMS {
        let value = properties.get(*path).cloned().unwrap_or_else(|| {
            json!(match *alias {
                "x" => position[0],
                "y" => position[1],
                "z" => position[2],
                "sx" | "sy" | "sz" => 100.,
                _ => 0.,
            })
        });
        p[*path] = if value.is_object() {
            value
        } else {
            schema::channel(value)
        };
    }
    for (k, v) in [
        ("anchor.x", 0.),
        ("anchor.y", 0.),
        ("anchor.z", 0.),
        ("orientation.x", 0.),
        ("orientation.y", 0.),
        ("orientation.z", 0.),
        ("opacity", 100.),
        ("skew", 0.),
    ] {
        p[k] = schema::channel(json!(v));
    }
    let fps = comp["fps"]
        .as_f64()
        .filter(|x| x.is_finite() && *x > 0.)
        .unwrap_or(30.);
    let from = (time * fps).round() / fps;
    Ok(
        json!({"id":id,"name":name,"type":"extension","threeD":true,"from":from,"dur":(comp["dur"].as_f64().unwrap_or(5.)-from).max(0.1),"on":visible,"lock":false,"solo":false,"shy":false,"parent":parent,"blend":"normal","mblur":false,"p":p,"fx":[],"masks":[],"d":{"definition":role.definition(),"version":1,"w":comp["w"].as_f64().unwrap_or(1920.),"h":comp["h"].as_f64().unwrap_or(1080.),"params":{},"data":schema::layer_data(role,data)?}}),
    )
}
/// Evaluators must resolve expressions and keys using the document's own time
/// semantics. Errors are surfaced; no capability is inferred from a fallback.
pub trait Evaluator {
    fn value(&mut self, layer: &Value, path: &str, channel: &Value, time: f64) -> Result<Value>;
    fn active(&mut self, layer: &Value, time: f64) -> Result<bool>;
}
pub struct StaticEvaluator;
impl Evaluator for StaticEvaluator {
    fn value(&mut self, _: &Value, _: &str, c: &Value, _: f64) -> Result<Value> {
        if c["kf"].as_array().is_some_and(|a| !a.is_empty())
            || c["expr"].as_str().is_some_and(|s| !s.is_empty())
        {
            return fail("An evaluator is required for animated 3D channels");
        }
        Ok(c["v"].clone())
    }
    fn active(&mut self, l: &Value, t: f64) -> Result<bool> {
        let from = l["from"].as_f64().unwrap_or(0.);
        Ok(l["on"] != false && t >= from && t < from + l["dur"].as_f64().unwrap_or(f64::INFINITY))
    }
}
pub fn composition(comp: &Value, time: f64, ev: &mut impl Evaluator) -> Result<Value> {
    if !time.is_finite() {
        return fail("Invalid composition time");
    }
    let mut scene = schema::parse_scene(json!({}))?;
    let mut objects = vec![];
    let mut lights = vec![];
    let mut camera_set = false;
    let layers = comp["layers"]
        .as_array()
        .ok_or_else(|| Error("Composition has no layers".into()))?;
    for l in layers {
        let Some(role) = schema::role(l) else {
            continue;
        };
        if !ev.active(l, time)? {
            continue;
        }
        let data = schema::layer_data(role, l["d"]["data"].clone())?;
        let mut trans = json!({});
        for (alias, path) in schema::TRANSFORMS {
            let fallback = schema::channel(json!(if alias.starts_with('s') { 100. } else { 0. }));
            let c = l["p"].get(*path).unwrap_or(&fallback);
            let mut value = ev
                .value(l, path, c, time)?
                .as_f64()
                .ok_or_else(|| Error("Non-numeric transform".into()))?;
            if alias.starts_with('s') {
                value /= 100.;
            }
            if ["rx", "ry", "rz"].contains(alias) {
                let axis = match *alias {
                    "rx" => "x",
                    "ry" => "y",
                    _ => "z",
                };
                let p = format!("orientation.{axis}");
                if let Some(c) = l["p"].get(&p) {
                    value += ev
                        .value(l, &p, c, time)?
                        .as_f64()
                        .ok_or_else(|| Error("Non-numeric orientation".into()))?;
                }
            }
            trans[*alias] = json!(value);
        }
        let mut eval_p = |p: &Value, prefix: &str| -> Result<Value> {
            let mut out = json!({});
            for (k, c) in p
                .as_object()
                .ok_or_else(|| Error("Missing properties".into()))?
            {
                out[k] = ev.value(l, &format!("{prefix}.{k}"), c, time)?;
            }
            Ok(out)
        };
        match role {
            Role::Object => {
                let mut o = data["object"].clone();
                o["id"] = l["id"].clone();
                o["name"] = l["name"].clone();
                o["parent"] = l["parent"].clone();
                o["p"] = trans;
                o["material"]["p"] = eval_p(&o["material"]["p"], "m")?;
                objects.push(o);
            }
            Role::Light => {
                let mut light = data["light"].clone();
                light["id"] = l["id"].clone();
                light["name"] = l["name"].clone();
                light["p"] = eval_p(&light["p"], "light")?;
                for k in ["x", "y", "z"] {
                    light["p"][k] = trans[k].clone();
                }
                lights.push(light);
            }
            Role::Camera if !camera_set => {
                scene["camera"] = data["camera"].clone();
                scene["camera"]["p"] = eval_p(&scene["camera"]["p"], "camera")?;
                for k in ["x", "y", "z"] {
                    scene["camera"]["p"][k] = trans[k].clone();
                }
                scene["environment"] = data["environment"].clone();
                scene["environment"]["p"] = eval_p(&scene["environment"]["p"], "environment")?;
                camera_set = true;
            }
            _ => {}
        }
    }
    let ids = objects
        .iter()
        .filter_map(|o| o["id"].as_str().map(str::to_owned))
        .collect::<std::collections::HashSet<_>>();
    for o in &mut objects {
        if !o["parent"].as_str().is_some_and(|p| ids.contains(p)) {
            o["parent"] = Value::Null;
        }
    }
    if lights.is_empty() {
        let mut id = "default_sun".to_string();
        while ids.contains(&id) {
            id.push('x');
        }
        lights.push(json!({"id":id,"type":"sun"}));
    }
    scene["objects"] = json!(objects);
    scene["lights"] = json!(lights);
    schema::parse_scene(scene)
}
pub fn presets() -> Value {
    serde_json::from_str(include_str!("material-presets.json")).expect("frozen preset data")
}
fn pos(l: &Value) -> Vec3 {
    ["position.x", "position.y", "position.z"].map(|p| l["p"][p]["v"].as_f64().unwrap_or(0.))
}
fn target(l: &Value) -> Vec3 {
    let role = schema::role(l).unwrap();
    ["targetX", "targetY", "targetZ"].map(|p| {
        l["d"]["data"][role.key()]["p"][p]["v"]
            .as_f64()
            .unwrap_or(0.)
    })
}
fn scalar(args: &Value, k: &str, f: f64) -> Result<f64> {
    match args.get(k) {
        None => Ok(f),
        Some(v) => v
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| Error(format!("Invalid {k}"))),
    }
}
fn vec3(v: &Value) -> Result<Vec3> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| Error("Expected [x,y,z]".into()))?;
    Ok([
        scalar(&json!({"v":a[0]}), "v", 0.)?,
        scalar(&json!({"v":a[1]}), "v", 0.)?,
        scalar(&json!({"v":a[2]}), "v", 0.)?,
    ])
}
fn box_for(comp: &Value, id: &str) -> Result<Bounds> {
    let a = comp["_scene3dBounds"][id]
        .as_array()
        .filter(|a| a.len() == 6)
        .ok_or_else(|| Error(format!("Evaluated geometry bounds unavailable: {id}")))?;
    let mut b = [0.; 6];
    for (i, v) in a.iter().enumerate() {
        b[i] = v
            .as_f64()
            .ok_or_else(|| Error("Invalid geometry bounds".into()))?;
    }
    let b = Bounds {
        min: [b[0], b[1], b[2]],
        max: [b[3], b[4], b[5]],
    };
    b.validate()?;
    Ok(b)
}
fn union(boxes: impl Iterator<Item = Result<Bounds>>) -> Result<Bounds> {
    let mut out: Option<Bounds> = None;
    for b in boxes {
        let b = b?;
        out = Some(match out {
            None => b,
            Some(a) => Bounds {
                min: std::array::from_fn(|i| a.min[i].min(b.min[i])),
                max: std::array::from_fn(|i| a.max[i].max(b.max[i])),
            },
        });
    }
    out.ok_or_else(|| Error("Add a model first".into()))
}
fn unparented(l: &Value) -> Result<()> {
    if !l["parent"].is_null() {
        return fail(
            "Geometry placement requires evaluated parent-space conversion; parented placement is unavailable",
        );
    }
    Ok(())
}
pub(crate) fn studio_plan(
    comp: &Value,
    args: &Value,
    selection: &[String],
    uid: &mut impl FnMut(&str) -> String,
) -> Result<Plan> {
    schema::plain(args)?;
    schema::strict(
        args,
        &[
            "operation",
            "target",
            "id",
            "name",
            "preset",
            "p",
            "at",
            "normal",
            "size",
            "intensity",
            "distance",
            "color",
            "shot",
            "start",
            "duration",
            "degrees",
            "amount",
            "from",
            "to",
            "on",
            "on_target",
        ],
    )?;
    let layers = comp["layers"]
        .as_array()
        .ok_or_else(|| Error("Composition has no layers".into()))?;
    let selected = args["target"]
        .as_str()
        .or_else(|| selection.first().map(String::as_str));
    let layer = selected.and_then(|id| layers.iter().find(|l| l["id"] == id));
    let camera = layers
        .iter()
        .find(|l| schema::role(l) == Some(Role::Camera) && l["on"] != false);
    let mut edit = |a: Value| operations::plan(comp, &a, selection, uid);
    let op = args["operation"].as_str().unwrap_or("");
    match op {
        "apply_material" => {
            let presets = presets();
            let preset = presets
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == args["preset"]);
            let mut p = preset.map(|p| p["p"].clone()).unwrap_or(json!({}));
            if preset.is_none() && args.get("p").is_none() {
                return fail("Unknown material preset");
            }
            if let Some(extra) = args.get("p") {
                for (k, v) in extra
                    .as_object()
                    .ok_or_else(|| Error("Expected material parameters".into()))?
                {
                    p[k] = v.clone();
                }
            }
            edit(
                json!({"operation":"update_object","target":selected,"patch":{"material":{"p":p}}}),
            )
        }
        "set_studio" => {
            let mut patch = json!({"shadowCatcher":args["on"]!=false});
            if let Some(c) = args.get("color") {
                patch["background"] = c.clone();
            }
            edit(json!({"operation":"set_environment","patch":patch}))
        }
        "light_highlight" => {
            let point = vec3(&args["at"])?;
            let normal = vec3(&args["normal"])?;
            let eye = camera.map(pos).unwrap_or([3., 2., 6.]);
            let distance = scalar(args, "distance", (length(sub(eye, point)) * 0.9).max(2.))?;
            let at = crate::light_tool::highlight_placement(point, normal, eye, distance)?;
            let size = scalar(args, "size", distance * 0.6)?;
            edit(
                json!({"operation":"add_light","name":args["name"].as_str().unwrap_or("Softbox"),"light":{"type":"area","p":{"x":at[0],"y":at[1],"z":at[2],"targetX":point[0],"targetY":point[1],"targetZ":point[2],"width":size,"height":size*0.62,"intensity":scalar(args,"intensity",6.)?,"color":args["color"].as_str().unwrap_or("#FFFFFF")}}}),
            )
        }
        "soften" => {
            let l = layer.ok_or_else(|| Error("Choose a softbox".into()))?;
            if l["d"]["data"]["light"]["type"] != "area" {
                return fail("Choose a softbox");
            }
            let k = scalar(args, "amount", 1.15)?.clamp(0.1, 10.);
            edit(
                json!({"operation":"update_light","target":selected,"patch":{"p":{"width":l["d"]["data"]["light"]["p"]["width"]["v"].as_f64().unwrap_or(2.)*k,"height":l["d"]["data"]["light"]["p"]["height"]["v"].as_f64().unwrap_or(1.2)*k}}}),
            )
        }
        "drop_to_floor" | "place_on" => {
            let l = layer
                .filter(|l| schema::role(l) == Some(Role::Object))
                .ok_or_else(|| Error("Choose a model to place".into()))?;
            unparented(l)?;
            let b = box_for(comp, l["id"].as_str().unwrap())?;
            let mut p = pos(l);
            let mut top = 0.;
            if op == "place_on" {
                let b2 = box_for(
                    comp,
                    args["on_target"]
                        .as_str()
                        .ok_or_else(|| Error("Choose a model to place it on".into()))?,
                )?;
                top = b2.max[1];
                p[0] += b2.center()[0] - b.center()[0];
                p[2] += b2.center()[2] - b.center()[2];
            }
            p[1] += top - b.min[1];
            edit(
                json!({"operation":"update_object","target":selected,"patch":{"p":{"x":p[0],"y":p[1],"z":p[2]}}}),
            )
        }
        "shot" => {
            let start = scalar(args, "start", 0.)?;
            let duration = scalar(args, "duration", 4.)?.max(0.1);
            if start < 0. || start + duration > 100000. {
                return fail("Shot outside supported time range");
            }
            let mut actions = vec![];
            let l = if args["shot"] == "turntable" {
                layer.ok_or_else(|| Error("Choose a model or group".into()))?
            } else {
                camera.ok_or_else(|| Error("Add a camera first".into()))?
            };
            let id = l["id"].as_str().unwrap().to_string();
            let mut keys = |path: &str, frames: Vec<(f64, f64)>| -> Result<()> {
                let from = l["from"].as_f64().unwrap_or(0.);
                let channel = schema::channel(json!(frames[0].1));
                let mut c = channel;
                c["kf"] = json!(
                    frames
                        .iter()
                        .map(|(t, v)| json!({"t":(t-from).max(0.),"v":v,"ease":"ease-in-out"}))
                        .collect::<Vec<_>>()
                );
                actions.push(Action::Channel {
                    id: id.clone(),
                    path: path.into(),
                    value: c,
                });
                Ok(())
            };
            match args["shot"].as_str() {
                Some("turntable") => {
                    if schema::role(l) != Some(Role::Object) && l["type"] != "group" {
                        return fail("A turntable needs a model or group");
                    }
                    let from = l["p"]["rotation.y"]["v"].as_f64().unwrap_or(0.);
                    keys(
                        "rotation.y",
                        vec![
                            (start, from),
                            (start + duration, from + scalar(args, "degrees", 360.)?),
                        ],
                    )?;
                }
                Some("rack_focus") => {
                    let eye = pos(l);
                    let near = scalar(args, "from", length(sub(eye, target(l))) * 0.7)?;
                    let far = scalar(
                        args,
                        "to",
                        layer
                            .map(|l| length(sub(eye, pos(l))))
                            .unwrap_or(near * 1.6),
                    )?;
                    keys(
                        "camera.focusDistance",
                        vec![(start, near), (start + duration, far)],
                    )?;
                }
                Some(shot @ ("push_in" | "orbit" | "hero_tilt")) => {
                    let eye = pos(l);
                    let aim = target(l);
                    let offset = sub(eye, aim);
                    let count = if shot == "push_in" { 2 } else { 9 };
                    let mut frames = [vec![], vec![], vec![]];
                    for i in 0..count {
                        let k = i as f64 / (count - 1) as f64;
                        let v = if shot == "push_in" {
                            mul(offset, 1. - scalar(args, "amount", 0.3)? * k)
                        } else {
                            let axis = if shot == "orbit" {
                                [0., 1., 0.]
                            } else {
                                normalize(cross(offset, [0., 1., 0.]))
                            };
                            let angle = if shot == "orbit" {
                                scalar(args, "degrees", 90.)? * k
                            } else {
                                -scalar(args, "degrees", 25.)? * (1. - k)
                            }
                            .to_radians();
                            rotate(offset, axis, angle)
                        };
                        let p = add(aim, v);
                        for a in 0..3 {
                            frames[a].push((start + duration * k, p[a]));
                        }
                    }
                    for (a, path) in ["position.x", "position.y", "position.z"]
                        .iter()
                        .enumerate()
                    {
                        keys(path, frames[a].clone())?;
                    }
                }
                _ => return fail("Unknown shot"),
            }
            Ok(Plan {
                actions,
                id: Some(id),
            })
        }
        "auto_studio" => {
            let models = layers
                .iter()
                .filter(|l| schema::role(l) == Some(Role::Object))
                .collect::<Vec<_>>();
            let b = union(
                models
                    .iter()
                    .map(|l| box_for(comp, l["id"].as_str().unwrap())),
            )?;
            let c = b.center();
            let r = b.radius().max(1e-3);
            let distance = r / (15f64.to_radians().sin()) * 1.15;
            let eye = add(c, mul(normalize([0.35, 0.55, 1.]), distance));
            let cam = json!({"p":{"x":eye[0],"y":eye[1],"z":eye[2],"targetX":c[0],"targetY":c[1],"targetZ":c[2],"fov":30}});
            let mut result = if let Some(camera) = camera {
                edit(json!({"operation":"set_camera","target":camera["id"],"patch":cam}))?
            } else {
                edit(
                    json!({"operation":"add_camera","camera":{"camera":cam,"environment":{"shadowCatcher":true}}}),
                )?
            };
            for (name, az, el, power, scale) in [
                ("Key softbox", -40f64, 50f64, 6., 1.4),
                ("Fill softbox", 60., 20., 2.2, 1.8),
                ("Rim strip", 150., 25., 7., 0.9),
            ] {
                let a = az.to_radians();
                let e = el.to_radians();
                let at = add(
                    c,
                    mul(
                        [a.sin() * e.cos(), e.sin(), a.cos() * e.cos()],
                        distance * 0.9,
                    ),
                );
                let p = edit(
                    json!({"operation":"add_light","name":name,"light":{"type":"area","p":{"x":at[0],"y":at[1],"z":at[2],"targetX":c[0],"targetY":c[1],"targetZ":c[2],"width":r*2.2*scale,"height":r*if name=="Rim strip"{0.35}else{1.4}*scale,"intensity":power}}}),
                )?;
                result.actions.extend(p.actions);
            }
            if camera.is_some() {
                result
                    .actions
                    .extend(edit(json!({"operation":"set_studio","on":true}))?.actions);
            }
            Ok(result)
        }
        "explode" => fail(
            "Exploded views require evaluated group/parent-space bounds; adapter not yet supplied",
        ),
        _ => fail("Unknown Studio operation"),
    }
}
fn rotate(v: Vec3, axis: Vec3, a: f64) -> Vec3 {
    add(
        add(mul(v, a.cos()), mul(cross(axis, v), a.sin())),
        mul(axis, dot(axis, v) * (1. - a.cos())),
    )
}
