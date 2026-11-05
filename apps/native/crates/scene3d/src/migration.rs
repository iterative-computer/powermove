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
pub fn upgrade_project(project: &mut Value) -> Result<Report> {
    let mut report = Report {
        upgraded: 0,
        issues: vec![],
    };
    if let Some(comps) = project.get_mut("comps").and_then(Value::as_object_mut) {
        for (_, comp) in comps {
            upgrade_comp(comp, &mut report)?;
        }
    }
    if project.get("layers").is_some() {
        upgrade_comp(project, &mut report)?;
    }
    Ok(report)
}
fn upgrade_comp(comp: &mut Value, report: &mut Report) -> Result<()> {
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
            match schema::layer_data(role, l["d"]["data"].clone()) {
                Ok(data) => {
                    if data != l["d"]["data"] {
                        let prior = l.clone();
                        l["d"]["data"] = data;
                        archive(&mut l, &prior);
                        report.upgraded += 1;
                    }
                    l["threeD"] = json!(true);
                }
                Err(e) => {
                    report.issues.push(format!("{id}: {e}"));
                    l["native3dUpgradeIssue"] = json!(e.to_string());
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
            legacy_obj(&l)
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
                l["native3dUpgradeIssue"] = json!(e.to_string());
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
fn legacy_obj(l: &Value) -> Result<Value> {
    let params = &l["d"]["params"];
    for k in [
        "yaw",
        "pitch",
        "distance",
        "autoRotate",
        "speed",
        "background",
    ] {
        if params[k]["kf"].as_array().is_some_and(|a| !a.is_empty())
            || params[k]["expr"].as_str().is_some_and(|x| !x.is_empty())
            || k == "autoRotate" && params[k]["v"] == true
        {
            return Err(Error("Legacy animated orbit/autorotation/background requires exact expression-context migration; original retained".into()));
        }
    }
    let p = |k: &str, d: Value| params.get(k).cloned().unwrap_or_else(|| schema::channel(d));
    let n = |k: &str, d: f64| params[k]["v"].as_f64().unwrap_or(d);
    let yaw = n("yaw", 28.).to_radians();
    let pitch = n("pitch", 18.).to_radians();
    let distance = n("distance", 4.2);
    let asset = l["d"]["data"]["assetId"].as_str().ok_or_else(|| {
        Error("Legacy model asset reference is missing; original retained".into())
    })?;
    schema::parse_scene(
        json!({"objects":[{"id":"model","name":l["name"].as_str().unwrap_or("Object"),"source":{"assetId":asset},"p":{"rx":p("rotationX",json!(0.)),"ry":p("rotationY",json!(25.)),"rz":p("rotationZ",json!(0.)),"sx":p("size",json!(1.35)),"sy":p("size",json!(1.35)),"sz":p("size",json!(1.35))},"material":{"p":{"color":p("objectColor",json!("#C7C4FF")),"roughness":p("roughness",json!(0.25)),"metalness":p("metalness",json!(0.7))}}}],"lights":[{"id":"key","name":"Key","type":"sun","p":{"color":p("lightColor",json!("#FFB36B"))}}],"camera":{"p":{"x":distance*pitch.cos()*yaw.sin(),"y":distance*pitch.sin(),"z":distance*pitch.cos()*yaw.cos(),"fov":58.109208}},"environment":{"background":params["background"]["v"].as_str().unwrap_or("#0C0D12"),"p":{"ambient":0.4}}}),
    )
}
