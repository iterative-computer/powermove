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
    let prepared = prepare_import_scale(snapshot, args, time, assets)?;
    let args = &prepared;
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
        descriptions: vec![label(args)],
        selection: selection.to_vec(),
    })
}

/// Opt-in source scale is an ordinary layer transform. This leaves old
/// projects and the renderer's shared normalized model resources unchanged.
fn prepare_import_scale(
    snapshot: &pm_kernel::Snapshot,
    args: &Value,
    time: f64,
    assets: &dyn pm_scene3d::geometry::AssetGeometry,
) -> Result<Value> {
    let mut prepared = args.clone();
    let Some(keep) = prepared
        .as_object_mut()
        .and_then(|v| v.remove("keepRealWorldScale"))
    else {
        return Ok(prepared);
    };
    if args["operation"] != "add_object" || !keep.is_boolean() {
        return Err(Error::invalid(
            "keepRealWorldScale is a boolean option for add_object",
        ));
    }
    if keep == false {
        return Ok(prepared);
    }
    let id = args["object"]["source"]["assetId"]
        .as_str()
        .ok_or_else(|| {
            Error::invalid("keepRealWorldScale requires an imported glTF/GLB assetId")
        })?;
    // Apply further transforms after import, so the source's shared origin
    // and units cannot be confused with already-normalized transform values.
    if args["object"]
        .get("p")
        .is_some_and(|p| p.as_object().is_none_or(|p| !p.is_empty()))
    {
        return Err(Error::invalid(
            "Add with keepRealWorldScale before applying object transforms",
        ));
    }
    let record = snapshot
        .root()
        .and_then(|r| r.get("assets"))
        .and_then(|a| a.get(id))
        .map(|v| pm_scene3d::from_model(&v.to_value()))
        .transpose()
        .map_err(Error::kernel)?
        .ok_or_else(|| Error::invalid("Imported model asset was not found"))?;
    let format = record["format"]
        .as_str()
        .or_else(|| {
            record["name"]
                .as_str()
                .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
        })
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["glb", "gltf"].contains(&format.as_str()) {
        return Err(Error::invalid(
            "Real-world import scale requires glTF/GLB; other source formats do not declare reliable metre units",
        ));
    }
    let fit = assets.source_fit(id, &record).map_err(Error::kernel)?;
    let resolved =
        pm_kernel::commands::scene3d::resolve_scene(snapshot, None, time).map_err(Error::kernel)?;
    let value = |v: &Value| v.get("v").unwrap_or(v).clone();
    let world = resolved.scene["objects"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|o| o.get("world").is_some() && value(&o["p"]["visible"]) != false)
        .map(|o| &o["world"])
        .or_else(|| {
            resolved.scene["lights"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|l| l["type"] == "environment" && value(&l["p"]["visible"]) != false)
                .map(|l| &l["world"])
        });
    let meters_per_unit = world
        .map(|w| value(&w["p"]["metersPerUnit"]))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.1);
    if !meters_per_unit.is_finite() || meters_per_unit <= 0. {
        return Err(Error::invalid("World scale must be finite and positive"));
    }
    let scale = fit.radius / meters_per_unit;
    let center = fit.center.map(|v| v / meters_per_unit);
    prepared["object"]["p"] =
        json!({"x":center[0],"y":center[1],"z":center[2],"sx":scale,"sy":scale,"sz":scale});
    Ok(prepared)
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

/// The Undo name for an agent's `edit_3d` when it gives none: what the
/// operation does, in the words the 3D Studio's own controls use.
pub fn label(args: &Value) -> String {
    let op = args["operation"].as_str().unwrap_or("");
    let light = |t: &Value| match t.as_str() {
        Some("area") => "Softbox",
        Some("spot") => "Spotlight",
        Some("point") => "Bulb",
        _ => "Sun",
    };
    match op {
        "add_object" | "create" => "Add 3D Model".into(),
        "add_light" => format!("Add {}", light(&args["light"]["type"])),
        "add_camera" => "Add Camera".into(),
        "update_object" => "Edit 3D Model".into(),
        "update_light" => "Edit Light".into(),
        "set_camera" => "Edit Camera".into(),
        "set_environment" => "Environment".into(),
        "set_world" => "Sky".into(),
        "add_fog" => "Add Fog Volume".into(),
        "update_fog" => "Edit Fog Volume".into(),
        "add_terrain" => "Add Terrain".into(),
        "update_terrain" => "Edit Terrain".into(),
        "add_scatter" => "Add Scatter".into(),
        "update_scatter" => "Edit Scatter".into(),
        "set_material_options" => "Material Options".into(),
        "set_render_quality" => "3D Quality".into(),
        "apply_material" => "Finish".into(),
        "light_highlight" => "Aim Light".into(),
        "shot" => "Add Shot".into(),
        "set_studio" => "Studio Set".into(),
        "auto_studio" => "Set Up Studio".into(),
        "drop_to_floor" => "Drop to Floor".into(),
        "place_on" => "Place on Surface".into(),
        "explode" => "Explode".into(),
        "soften" => "Soften Light".into(),
        "remove" => "Delete 3D Layers".into(),
        "duplicate" => "Duplicate 3D Layers".into(),
        _ => "Edit 3D Layers".into(),
    }
}
