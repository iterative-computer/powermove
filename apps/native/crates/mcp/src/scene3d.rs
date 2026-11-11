//! 3D MCP requests use the same kernel commands as native/extension clients.
use crate::{Error, Result, commands::Batch, inspection::Evaluation};
use pm_kernel::Scene3dCommand;
use pm_scene3d::{schema, service};
use serde_json::{Value, json};
pub fn batch(
    snapshot: &pm_kernel::Snapshot,
    args: &Value,
    selection: &[String],
    time: f64,
    assets: &dyn pm_scene3d::geometry::AssetGeometry,
) -> Result<Batch> {
    let command = if pm_scene3d::geometry::needs_geometry(args) {
        let geometry = pm_kernel::commands::scene3d::prepare_geometry(
            snapshot,
            None,
            &crate::model(args),
            selection,
            time,
            assets,
        )
        .map_err(Error::kernel)?;
        Scene3dCommand::ApplyPrepared {
            comp: None,
            args: crate::model(args),
            selection: selection.to_vec(),
            time,
            autokey: false,
            geometry,
        }
    } else {
        Scene3dCommand::Apply {
            comp: None,
            args: crate::model(args),
            selection: selection.to_vec(),
            time,
            autokey: false,
        }
    };
    Ok(Batch {
        commands: vec![command.into()],
        results: vec![],
        descriptions: vec!["Edit 3D Layers".into()],
        selection: selection.to_vec(),
    })
}

pub fn result_id(
    before: &Value,
    after: &Value,
    args: &Value,
    selection: &[String],
) -> Option<String> {
    let ls = after["layers"].as_array()?;
    let old = before["layers"].as_array()?;
    let new = ls
        .iter()
        .filter(|l| !old.iter().any(|b| b["id"] == l["id"]))
        .collect::<Vec<_>>();
    let preferred = match args["operation"].as_str() {
        Some("create" | "add_object") => Some(schema::Role::Object),
        Some("auto_studio" | "add_camera" | "set_camera" | "set_environment") => {
            Some(schema::Role::Camera)
        }
        Some("add_light" | "light_highlight") => Some(schema::Role::Light),
        _ => None,
    };
    let found = preferred
        .and_then(|r| new.iter().rev().find(|l| schema::role(l) == Some(r)))
        .or_else(|| new.last());
    if let Some(l) = found {
        return l["id"].as_str().map(str::to_owned);
    }
    let selected = args["id"]
        .as_str()
        .or(args["target"].as_str())
        .or_else(|| selection.first().map(String::as_str));
    let explicit = selected.and_then(|s| ls.iter().find(|l| l["id"] == s || l["name"] == s));
    let chosen = if matches!(preferred, Some(schema::Role::Camera)) {
        explicit
            .filter(|l| schema::role(l) == preferred)
            .or_else(|| {
                ls.iter()
                    .find(|l| schema::role(l) == preferred && l["on"] != false)
            })
    } else {
        explicit
    };
    chosen.and_then(|l| l["id"].as_str().map(str::to_owned))
}
struct Evaluated<'a> {
    ev: &'a mut Evaluation,
    layers: &'a [Value],
}
impl service::Evaluator for Evaluated<'_> {
    fn value(&mut self, l: &Value, path: &str, c: &Value, t: f64) -> pm_scene3d::Result<Value> {
        let i = self
            .layers
            .iter()
            .position(|n| n["id"] == l["id"])
            .ok_or_else(|| pm_scene3d::Error("Layer moved".into()))?;
        self.ev
            .query("evP", i, t, json!({"key":path,"prop":c}))
            .map_err(|e| pm_scene3d::Error(e.to_string()))
    }
    fn active(&mut self, l: &Value, t: f64) -> pm_scene3d::Result<bool> {
        let i = self
            .layers
            .iter()
            .position(|n| n["id"] == l["id"])
            .ok_or_else(|| pm_scene3d::Error("Layer moved".into()))?;
        self.ev
            .query("active", i, t, json!({}))
            .map(|v| v == true)
            .map_err(|e| pm_scene3d::Error(e.to_string()))
    }
}
pub fn evaluated_scene(p: &Value, time: f64, ev: &mut Evaluation) -> Result<Value> {
    let layers = p["layers"]
        .as_array()
        .ok_or_else(|| Error::invalid("No composition layers"))?;
    service::composition(p, time, &mut Evaluated { ev, layers })
        .map_err(|e| Error::invalid(e.to_string()))
}
pub fn properties(l: &Value) -> Vec<(String, Value)> {
    let mut out = vec![];
    if let Some(p) = l["p"].as_object() {
        for (k, v) in p {
            if v["kf"].is_array() {
                out.push((k.clone(), v.clone()));
            }
        }
    }
    let content = match schema::role(l) {
        Some(schema::Role::Object) => vec![("m", &l["d"]["data"]["object"]["material"]["p"])],
        Some(schema::Role::Light) => vec![("light", &l["d"]["data"]["light"]["p"])],
        Some(schema::Role::Camera) => vec![
            ("camera", &l["d"]["data"]["camera"]["p"]),
            ("environment", &l["d"]["data"]["environment"]["p"]),
        ],
        _ => vec![],
    };
    for (prefix, p) in content {
        if let Some(p) = p.as_object() {
            for (k, v) in p {
                out.push((format!("{prefix}.{k}"), v.clone()));
            }
        }
    }
    let node = if schema::role(l) == Some(schema::Role::Light) {
        &l["d"]["data"]["light"]
    } else {
        &l["d"]["data"]["object"]
    };
    out.extend(
        pm_scene3d::contract::channels(node)
            .into_iter()
            .map(|(path, prop)| (path, prop.clone())),
    );
    out.extend(pm_scene3d::extrusion::channels(l).into_iter().map(|(path,prop)|(path,prop.clone())));
    out
}
