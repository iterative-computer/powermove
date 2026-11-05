//! Load-time upgrade with an immutable copy of each replaced source layer.
//! Unconvertible authored input stays intact with a durable diagnostic.
use crate::{
    Error, Result,
    operations::{self, Action},
    schema::{self},
    service,
};
use serde_json::{Value, json};
use std::collections::HashSet;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub upgraded: usize,
    pub issues: Vec<String>,
}
pub trait LegacySampler {
    fn sample(&mut self, layer_id: &str, key: &str, property: &Value, time: f64) -> Result<Value>;
}
pub struct StaticSampler;
impl LegacySampler for StaticSampler {
    fn sample(&mut self, _: &str, _: &str, property: &Value, _: f64) -> Result<Value> {
        if animated(property) {
            return Err(Error(
                "An evaluator is required to upgrade animated 3D channels".into(),
            ));
        }
        Ok(property["v"].clone())
    }
}
fn animated(prop: &Value) -> bool {
    prop["kf"].as_array().is_some_and(|a| !a.is_empty())
        || prop["expr"].as_str().is_some_and(|s| !s.is_empty())
}
pub fn upgrade_project(project: &mut Value) -> Result<Report> {
    upgrade_project_with(project, &mut StaticSampler)
}
pub fn upgrade_project_with(
    project: &mut Value,
    sampler: &mut impl LegacySampler,
) -> Result<Report> {
    let mut report = Report {
        upgraded: 0,
        issues: vec![],
    };
    if let Some(comps) = project.get_mut("comps").and_then(Value::as_object_mut) {
        for (_, comp) in comps {
            upgrade_comp(comp, &mut report, sampler)?;
        }
    }
    if project.get("layers").is_some() {
        upgrade_comp(project, &mut report, sampler)?;
    }
    if report.upgraded > 0 || !report.issues.is_empty() {
        project["native3dUpgrade"] =
            json!({"version":1,"upgraded":report.upgraded,"issues":report.issues});
    }
    Ok(report)
}
fn upgrade_comp(
    comp: &mut Value,
    report: &mut Report,
    sampler: &mut impl LegacySampler,
) -> Result<()> {
    let Some(layers) = comp["layers"].as_array() else {
        return Ok(());
    };
    let original = layers.clone();
    let mut used = original
        .iter()
        .filter_map(|l| l["id"].as_str().map(str::to_owned))
        .collect::<HashSet<_>>();
    let mut out = vec![];
    for mut l in original {
        let id = l["id"].as_str().unwrap_or("layer").to_string();
        if let Some(role) = schema::role(&l) {
            let mut input = l["d"]["data"].clone();
            // Engine-side historical fixtures carried empty slot lists. They
            // author no override; retain the entire source in the archive and
            // remove only that empty field from the source-selected schema.
            if role == schema::Role::Object
                && input["object"]["slots"]
                    .as_array()
                    .is_some_and(|a| a.is_empty())
            {
                input["object"].as_object_mut().unwrap().remove("slots");
            }
            match schema::layer_data(role, input) {
                Ok(data) => {
                    if data != l["d"]["data"] || l["threeD"] != true {
                        let prior = l.clone();
                        l["d"]["data"] = data;
                        archive(&mut l, &prior);
                        report.upgraded += 1;
                    }
                    l["threeD"] = json!(true);
                }
                Err(e) => {
                    report.issues.push(format!("{id}: {e}"));
                    mark_failed(&mut l, &e.to_string());
                }
            }
            out.push(l);
            continue;
        }
        let definition = l["d"]["definition"].as_str().unwrap_or("");
        if definition != schema::SCENE && definition != "powermove.3d.obj-model" {
            out.push(l);
            continue;
        }
        let scene = if definition == schema::SCENE {
            schema::parse_scene(l["d"]["data"]["scene"].clone())
        } else {
            legacy_obj(&l, comp, sampler)
        };
        match scene {
            Ok(scene) => {
                let mut serial = 0;
                let prefix = id
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .take(40)
                    .collect::<String>();
                let mut uid = |kind: &str| loop {
                    serial += 1;
                    let id = format!("N{prefix}_{kind}_{serial}");
                    if used.insert(id.clone()) {
                        break id;
                    }
                };
                let plan = operations::plan(
                    comp,
                    &json!({"operation":"create","scene":scene}),
                    &[],
                    &mut uid,
                )?;
                let source = l.clone();
                l["type"] = json!("group");
                l["threeD"] = json!(true);
                archive(&mut l, &source);
                l["d"] = json!({});
                out.push(l);
                let mut made = vec![];
                for action in plan.actions {
                    if let Action::Add {
                        id: new_id,
                        name,
                        role,
                        data,
                        properties,
                        visible,
                        parent,
                    } = action
                    {
                        let mut next = service::new_layer(
                            comp,
                            &new_id,
                            &name,
                            role,
                            data,
                            properties,
                            visible,
                            parent.or_else(|| Some(id.clone())),
                            0.,
                        )?;
                        next["from"] = source["from"].clone();
                        next["dur"] = source["dur"].clone();
                        next["group"] = json!(id);
                        next["native3dSourceLayer"] = json!(id);
                        fn assign_keys(v: &mut Value, uid: &mut impl FnMut(&str) -> String) {
                            match v {
                                Value::Object(m) => {
                                    if let Some(Value::Array(keys)) = m.get_mut("kf") {
                                        for key in keys {
                                            key["i"] = json!(uid("kf"));
                                        }
                                    }
                                    for (_, value) in m {
                                        assign_keys(value, uid);
                                    }
                                }
                                Value::Array(values) => {
                                    for value in values {
                                        assign_keys(value, uid);
                                    }
                                }
                                _ => {}
                            }
                        }
                        assign_keys(&mut next, &mut uid);
                        made.push(next);
                    }
                }
                // Add operations insert above earlier additions, as the timeline
                // does. Keep the old layer as the enclosing identity/transform.
                made.reverse();
                out.extend(made);
                report.upgraded += 1;
            }
            Err(e) => {
                report.issues.push(format!("{id}: {e}"));
                mark_failed(&mut l, &e.to_string());
                out.push(l);
            }
        }
    }
    comp["layers"] = json!(out);
    Ok(())
}
fn archive(layer: &mut Value, original: &Value) {
    if layer.get("native3dArchive").is_none() {
        layer["native3dArchive"] = json!({"version":1,"source":original});
    }
}
fn legacy_obj(l: &Value, comp: &Value, sampler: &mut impl LegacySampler) -> Result<Value> {
    let params = &l["d"]["params"];
    if animated(&params["background"]) {
        return Err(Error(crate::presentation::ANIMATED_BACKGROUND.into()));
    }
    let p = |k: &str, d: Value| params.get(k).cloned().unwrap_or_else(|| schema::channel(d));
    let mappings = [
        ("rotationX", "rx", json!(0.)),
        ("rotationY", "ry", json!(25.)),
        ("rotationZ", "rz", json!(0.)),
        ("size", "sx", json!(1.35)),
        ("objectColor", "color", json!("#C7C4FF")),
        ("roughness", "roughness", json!(0.25)),
        ("metalness", "metalness", json!(0.7)),
        ("lightColor", "lightColor", json!("#FFB36B")),
    ];
    let asset = l["d"]["data"]["assetId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error(crate::presentation::MISSING_MODEL.into()))?;
    let mut scene = schema::parse_scene(
        json!({"objects":[{"id":"model","name":l["name"].as_str().unwrap_or("Object"),"source":{"assetId":asset}}],"lights":[{"id":"key","name":"Sun","type":"sun"}],"camera":{"p":{"fov":58.109208}},"environment":{"background":params["background"]["v"].as_str().unwrap_or("#0C0D12"),"p":{"ambient":0.4}}}),
    )?;
    for (old, key, d) in &mappings {
        let channel = p(old, d.clone());
        if *key == "lightColor" {
            scene["lights"][0]["p"]["color"] = channel;
        } else if ["color", "roughness", "metalness"].contains(key) {
            scene["objects"][0]["material"]["p"][key] = channel;
        } else {
            scene["objects"][0]["p"][key] = channel;
        }
    }
    let bake_camera = ["yaw", "pitch", "distance"]
        .iter()
        .any(|k| animated(&params[*k]));
    let bake_rotation = params["autoRotate"]["v"] == true || animated(&params["autoRotate"]);
    let has_expressions = mappings
        .iter()
        .any(|(old, _, _)| params[*old]["expr"].as_str().is_some_and(|s| !s.is_empty()));
    let fps = comp["fps"]
        .as_f64()
        .filter(|x| x.is_finite() && *x > 0.)
        .unwrap_or(30.)
        .max(1.);
    let duration = l["dur"]
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.)
        .unwrap_or(0.);
    let from = l["from"].as_f64().unwrap_or(0.);
    let frames = (duration * fps).ceil() as usize;
    if (bake_camera || bake_rotation || has_expressions) && frames > 2398 {
        return Err(Error(crate::presentation::LONG_ANIMATION.into()));
    }
    let id = l["id"].as_str().unwrap_or("");
    let mut sample =
        |key: &str, d: Value, t: f64| sampler.sample(id, &format!("x.{key}"), &p(key, d), from + t);
    let numeric = |v: Value| {
        v.as_f64()
            .filter(|x| x.is_finite())
            .ok_or_else(|| Error("Legacy 3D expression did not produce a finite number".into()))
    };
    let sample_count = if bake_camera || bake_rotation || has_expressions {
        frames + 1
    } else {
        1
    };
    for frame in 0..sample_count {
        let t = (frame as f64 / fps).min(duration);
        let yaw = numeric(sample("yaw", json!(28.), t)?)?.to_radians();
        let pitch = numeric(sample("pitch", json!(18.), t)?)?.to_radians();
        let distance = numeric(sample("distance", json!(4.2), t)?)?;
        let camera = [
            distance * pitch.cos() * yaw.sin(),
            distance * pitch.sin(),
            distance * pitch.cos() * yaw.cos(),
        ];
        for (a, key) in ["x", "y", "z"].iter().enumerate() {
            let c = &mut scene["camera"]["p"][key];
            if frame == 0 {
                c["v"] = json!(camera[a]);
            }
            if bake_camera {
                c["kf"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"t":t,"v":camera[a]}));
            }
        }
        for (old, key, d) in &mappings {
            let has_expr = params[*old]["expr"].as_str().is_some_and(|s| !s.is_empty());
            if !(has_expr || *key == "ry" && bake_rotation) {
                continue;
            }
            let mut value = sample(old, d.clone(), t)?;
            if *key == "ry" && bake_rotation && sample("autoRotate", json!(false), t)? == true {
                value = json!(numeric(value)? + t * numeric(sample("speed", json!(25.), t)?)?);
            }
            let channel = if *key == "lightColor" {
                &mut scene["lights"][0]["p"]["color"]
            } else if ["color", "roughness", "metalness"].contains(key) {
                &mut scene["objects"][0]["material"]["p"][key]
            } else {
                &mut scene["objects"][0]["p"][key]
            };
            if frame == 0 {
                channel["v"] = value.clone();
                channel["expr"] = Value::Null;
                channel["kf"] = json!([]);
            }
            channel["kf"]
                .as_array_mut()
                .unwrap()
                .push(json!({"t":t,"v":value}));
        }
    }
    scene["objects"][0]["p"]["sy"] = scene["objects"][0]["p"]["sx"].clone();
    scene["objects"][0]["p"]["sz"] = scene["objects"][0]["p"]["sx"].clone();
    schema::parse_scene(scene)
}

fn mark_failed(layer: &mut Value, reason: &str) {
    let original = layer.clone();
    archive(layer, &original);
    layer["d"]["definition"] = json!("powermove.3d.unupgraded");
    layer["native3dUpgradeDiagnostic"] = json!(reason);
    layer["native3dUpgradeIssue"] = json!(if [
        crate::presentation::MISSING_MODEL,
        crate::presentation::ANIMATED_BACKGROUND,
        crate::presentation::LONG_ANIMATION
    ]
    .contains(&reason)
    {
        crate::presentation::upgrade_error(reason)
    } else {
        // DESIGN-TODO(opus): a reason for unsupported historical/future 3D
        // fields and evaluator errors. Keep raw diagnostics in the Error Log;
        // use the approved neutral failure sentence in the model meanwhile.
        "This 3D layer couldn’t be upgraded.".into()
    });
}
/// Restore only the retained source in memory, then retry the normal upgrader.
/// The archive is never removed from a saved document by this operation.
pub fn retry(project: &mut Value, id: &str) -> Result<Report> {
    fn restore(comp: &mut Value, id: &str) -> bool {
        if let Some(layers) = comp["layers"].as_array_mut() {
            for layer in layers {
                if layer["id"] == id
                    && layer["d"]["definition"] == "powermove.3d.unupgraded"
                    && let Some(source) = layer["native3dArchive"].get("source").cloned()
                {
                    *layer = source;
                    return true;
                }
            }
        }
        false
    }
    let mut found = restore(project, id);
    if !found && let Some(comps) = project.get_mut("comps").and_then(Value::as_object_mut) {
        for (_, comp) in comps {
            if restore(comp, id) {
                found = true;
                break;
            }
        }
    }
    if !found {
        return Err(Error("No retained 3D upgrade source".into()));
    }
    upgrade_project(project)
}
