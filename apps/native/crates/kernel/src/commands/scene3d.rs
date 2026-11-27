//! Studio commands run entirely on the kernel scratch tree. No renderer or JS
//! callback participates in validation, history capture or publication.
use super::{
    Cx,
    channels::{self, Step, key},
    layers::{LayerCommand, LayerPatch, ParentChange},
    new_id,
    properties::{self, KeyMode, SetProperty},
    values,
};
use crate::error::CommandError;
#[path="scene3d_import.rs"]
mod import_placement;
use pm_model::Value;
use pm_scene3d::{
    operations::{Action, Plan},
    schema,
};
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Scene3dCommand {
    SetData {
        layer_id: String,
        data: Value,
    },
    /// Legacy command envelope: one operation or an array, addressed by layer id.
    Edit {
        layer_id: String,
        ops: Value,
    },
    /// Composition-level edit, including creation and Studio's product-film ops.
    Apply {
        comp: Option<String>,
        args: Value,
        selection: Vec<String>,
        time: f64,
        autokey: bool,
    },
    /// Bytes were validated and stored immutably on an import worker. One
    /// command adds the asset, model, and (when empty) the source's studio rig.
    Import {
        comp: Option<String>,
        asset_id: String,
        layer_id: String,
        receipt: std::sync::Arc<pm_scene3d::import::archive::Receipt>,
        time: f64,
        setup_studio: bool,
    },
    /// Reattach verified immutable sources after library/journal recovery; no
    /// document edit or Undo entry. The host resolves the archive on a worker.
    BindImport {
        asset_id: String,
        receipt: std::sync::Arc<pm_scene3d::import::archive::Receipt>,
    },
    /// A geometry preflight tied to an immutable snapshot, prepared on a worker.
    ApplyPrepared {
        comp: Option<String>,
        args: Value,
        selection: Vec<String>,
        time: f64,
        autokey: bool,
        geometry: pm_scene3d::geometry::EvaluatedGeometry,
    },
}
fn invalid(e: impl std::fmt::Display) -> CommandError {
    CommandError::Invalid(e.to_string())
}
impl Scene3dCommand {
    pub fn label(&self) -> String {
        match self {
            Self::Import { receipt, .. } => format!("Import {}", receipt.primary().name()),
            _ => "Edit 3D Scene".into(),
        }
    }
    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            Self::BindImport { asset_id, receipt } => {
                let current = cx
                    .lookup(&["assets".into(), asset_id.as_str().into()])
                    .ok_or_else(|| invalid("3D import metadata is missing"))?
                    .to_value();
                let mut expected=import_metadata(asset_id,receipt);
                if current.get("native3dBounds").is_none(){expected.as_object_mut().unwrap().remove("native3dBounds");}
                if current.get("native3dMaterials").is_none(){expected.as_object_mut().unwrap().remove("native3dMaterials");}
                if current != pm_scene3d::to_model(&expected) {
                    return Err(invalid("3D import source changed; recover again"));
                }
                Ok(())
            }
            Self::Import {
                comp,
                asset_id,
                layer_id,
                receipt,
                time,
                setup_studio,
            } => import(
                cx,
                comp.as_deref(),
                asset_id,
                layer_id,
                receipt,
                *time,
                *setup_studio,
            ),
            Self::SetData { layer_id, data } => set_data(cx, layer_id, data.clone()),
            Self::Edit { layer_id, ops } => {
                let raw = pm_scene3d::from_model(ops).map_err(invalid)?;
                let list = raw.as_array().cloned().unwrap_or_else(|| vec![raw]);
                for mut args in list {
                    if args.get("target").is_none() && args.get("id").is_none() {
                        args["target"] = serde_target(layer_id);
                    }
                    apply(cx, None, &pm_scene3d::to_model(&args), &[], 0., false, None)?;
                }
                Ok(())
            }
            Self::Apply {
                comp,
                args,
                selection,
                time,
                autokey,
            } => apply(cx, comp.as_deref(), args, selection, *time, *autokey, None),
            Self::ApplyPrepared {
                comp,
                args,
                selection,
                time,
                autokey,
                geometry,
            } => apply(
                cx,
                comp.as_deref(),
                args,
                selection,
                *time,
                *autokey,
                Some(geometry),
            ),
        }
    }
}
fn serde_target(s: &str) -> serde_json::Value {
    serde_json::Value::String(s.into())
}
fn guard(cx: &Cx<'_>, id: &str, path: &str) -> Result<(), CommandError> {
    let (_, l) = cx.layer_mut_checked(id)?;
    if cx.origin().is_generated()
        && (properties::locked_intent(&l, path) || properties::locked_intent(&l, "3d"))
    {
        return Err(CommandError::PreservedHandEdit {
            layer_id: id.into(),
            channel: path.into(),
        });
    }
    Ok(())
}
fn set_data(cx: &mut Cx<'_>, id: &str, data: Value) -> Result<(), CommandError> {
    guard(cx, id, "3d")?;
    let (loc, mut l) = cx.layer(id)?;
    let json = pm_scene3d::from_model(&l).map_err(invalid)?;
    let role =
        schema::role(&json).ok_or_else(|| invalid("Choose a model, light or camera layer"))?;
    // Whole-content replacement may not overwrite any preserved content channel.
    if cx.origin().is_generated()
        && l.get("locked_intent")
            .and_then(Value::as_object)
            .is_some_and(|m| {
                m.iter().any(|(k, v)| {
                    v.is_truthy()
                        && [
                            "m.",
                            "light.",
                            "camera.",
                            "environment.",
                            "world.",
                            "fog.",
                            "terrain.",
                            "scatter.",
                            "materialOptions.",
                            "mi.",
                            "pt.",
                        ]
                        .iter()
                        .any(|p| k.to_string_lossy().starts_with(p))
                })
            })
    {
        return Err(invalid("Preserved hand-edited 3D content"));
    }
    let checked = schema::layer_data(role, pm_scene3d::from_model(&data).map_err(invalid)?)
        .map_err(invalid)?;
    values::obj(
        values::obj(&mut l)
            .get_mut("d")
            .ok_or_else(|| invalid("Missing content"))?,
    )
    .insert("data", pm_scene3d::to_model(&checked));
    cx.put_layer(&loc, l)
}
// The JSON planners need layer identities and ancestry for targeting and
// live-source validation. Keep ordinary artwork payloads on the document.
fn planning_composition(
    comp: &pm_model::Node,
    include_controls: bool,
) -> Result<serde_json::Value, CommandError> {
    use pm_scene3d::node_fields;
    let mut fields = node_fields(comp, &["id", "w", "h", "fps", "dur", "studioQuality"]);
    let layers = composition_layers(comp)?;
    let layers = layers
        .iter()
        .map(|layer| {
            let mut fields = node_fields(
                layer,
                &[
                    "id", "name", "type", "parent", "group", "threeD", "lock", "on", "from", "dur",
                ],
            );
            if let Some(d) = layer.get("d") {
                let role = pm_scene3d::layer_role(layer);
                let mut data = node_fields(d, &["definition"]);
                if include_controls {
                    if role.is_some() {
                        if let Some(content) = d.get("data") {
                            data.insert("data", content.to_value());
                        }
                        for (key, value) in node_fields(layer, &["p", "locked_intent"]) {
                            fields.insert(key, value);
                        }
                    } else if let Some(content) = d.get("data") {
                        // World/map validation also recognizes object data on older layers.
                        let object = node_fields(content, &["object"]);
                        if !object.is_empty() {
                            data.insert("data", Value::Object(object));
                        }
                    }
                    if d.get("extrusion").is_some()
                        || matches!(
                            layer.get("type").and_then(crate::model::str_of),
                            Some("shape" | "text")
                        )
                    {
                        for (key, value) in node_fields(d, &["extrusion", "color"]) {
                            data.insert(key, value);
                        }
                    }
                }
                if !data.is_empty() {
                    fields.insert("d", Value::Object(data));
                }
            }
            Value::Object(fields)
        })
        .collect();
    fields.insert("layers", Value::Array(layers));
    pm_scene3d::from_model(&Value::Object(fields)).map_err(invalid)
}

fn composition_layers(comp: &pm_model::Node) -> Result<&[pm_model::Node], CommandError> {
    pm_eval::node::array(comp.get("layers")).ok_or_else(|| invalid("No composition layers"))
}

#[allow(clippy::too_many_arguments)]
fn apply(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    args: &Value,
    selection: &[String],
    time: f64,
    autokey: bool,
    geometry: Option<&pm_scene3d::geometry::EvaluatedGeometry>,
) -> Result<(), CommandError> {
    if !time.is_finite() {
        return Err(invalid("Invalid 3D time"));
    }
    let composition = cx.comp_node(comp)?;
    let mut json = planning_composition(composition, true)?;
    let mut args = pm_scene3d::from_model(args).map_err(invalid)?;
    if args["operation"] == "add_camera"
        && args.get("camera").is_none()
        && args.get("patch").is_none()
        && pm_eval::space3d::uses_default_25d_camera(crate::model::layers_of(composition))
        && !crate::model::layers_of(composition).any(|layer| {
            pm_eval::parenting::Role3d::of(layer) == Some(pm_eval::parenting::Role3d::Camera)
        })
    {
        let width = json["w"].as_f64().unwrap_or(1920.);
        let height = json["h"].as_f64().unwrap_or(1080.);
        let camera = pm_eval::space3d::default_camera_25d(width, height);
        args["camera"] =
            serde_json::json!({"p":{"z":camera.position[2],"fov":camera.fov.clamp(1., 150.)}});
    }
    if pm_scene3d::geometry::needs_geometry(&args) {
        let geometry = geometry
            .ok_or_else(|| invalid("Prepare evaluated 3D geometry on a worker before editing"))?;
        if geometry.source != *cx.root()
            || geometry.comp.as_deref() != comp
            || geometry.time != time
        {
            return Err(invalid("3D geometry snapshot changed; prepare again"));
        }
        json["_scene3dBounds"] = geometry.bounds.clone();
        json["_scene3dParentInverse"] = geometry.parent_inverse.clone();
    }
    json["_scene3dTime"] = serde_json::json!(time);
    let Plan { mut actions, .. } =
        pm_scene3d::operations::plan(&json, &args, selection, &mut new_id).map_err(invalid)?;
    // Content structural updates precede channel writes so a material-map edit
    // cannot restore the previous material channels later in the same plan.
    actions.sort_by_key(|a| {
        if matches!(a, Action::Data { .. } | Action::StagedData { .. } | Action::Extrusion { .. }) {
            0
        } else {
            1
        }
    });
    for action in actions {
        match action {
            Action::Extrusion { id, data, enable_three_d } => {
                guard(cx, &id, "3d")?;
                let (loc, mut layer)=cx.layer_mut_checked(&id)?;
                let before=layer.clone();
                let content=values::obj(&mut layer).get_mut("d").ok_or_else(||invalid("Missing layer content"))?;
                values::obj(content).insert("extrusion",pm_scene3d::to_model(&data));
                if cx.origin().is_generated() {
                    for (name,path) in all(&before) {
                        if properties::locked_intent(&before,&name) && channels::get(&before,&path)!=channels::get(&layer,&path) {
                            return Err(CommandError::PreservedHandEdit {layer_id:id.clone(),channel:name});
                        }
                    }
                }
                normalize_key_ids(&mut layer);cx.put_layer(&loc,layer)?;
                if enable_three_d { LayerCommand::Set {layer_id:id,patch:LayerPatch{three_d:Some(true),..Default::default()}}.plan(cx)?; }
            }
            Action::RenderQuality { quality } => {
                let quality = pm_scene3d::quality::validate(quality).map_err(invalid)?;
                let mut path = cx.comp_path(comp);
                path.push("studioQuality".into());
                cx.set(path, pm_scene3d::to_model(&quality))?;
            }
            Action::StagedData { id, data } => {
                guard(cx, &id, "3d")?;
                let (loc, mut layer) = cx.layer_mut_checked(&id)?;
                let before = layer.clone();
                values::obj(
                    values::obj(&mut layer)
                        .get_mut("d")
                        .ok_or_else(|| invalid("Missing 3D content"))?,
                )
                .insert("data", pm_scene3d::to_model(&data));
                if cx.origin().is_generated() {
                    for (name, path) in all(&before) {
                        if properties::locked_intent(&before, &name)
                            && channels::get(&before, &path) != channels::get(&layer, &path)
                        {
                            return Err(CommandError::PreservedHandEdit {
                                layer_id: id.clone(),
                                channel: name,
                            });
                        }
                    }
                }
                normalize_key_ids(&mut layer);
                cx.put_layer(&loc, layer)?;
            }
            Action::Add {
                id,
                name,
                role,
                data,
                properties,
                visible,
                parent,
            } => {
                if cx.all_layers().iter().any(|loc| {
                    cx.layer_node(loc)
                        .ok()
                        .and_then(|l| l.get("id"))
                        .and_then(crate::model::str_of)
                        == Some(&id)
                }) {
                    return Err(invalid("Layer ID already exists"));
                }
                if let Some(p) = parent.as_deref() {
                    let loc = cx.find(p)?;
                    cx.check_lock(&loc, p)?;
                    if loc.comp_id != cx.comp_id(comp) {
                        return Err(invalid("Parent is in another composition"));
                    }
                }
                let layer = pm_scene3d::service::new_layer(
                    &json, &id, &name, role, data, properties, visible, parent, time,
                )
                .map_err(invalid)?;
                let mut layer = pm_scene3d::to_model(&layer);
                normalize_key_ids(&mut layer);
                cx.insert_layer(comp, 0, layer)?;
            }
            Action::Data { id, data } => set_data(cx, &id, pm_scene3d::to_model(&data))?,
            Action::Channel { id, path, value } => {
                guard(cx, &id, &path)?;
                if value.is_object() {
                    let (loc, mut layer) = cx.layer_mut_checked(&id)?;
                    let p = channels::materialize(&mut layer, &path)
                        .ok_or_else(|| invalid(format!("Unknown 3D channel: {path}")))?;
                    let mut value = pm_scene3d::to_model(&value);
                    if let Some(expr) = value
                        .get("expr")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        properties::check_expression(expr)?;
                    }
                    fresh_keys(&mut value, false);
                    *channels::get_mut(&mut layer, &p).expect("resolved") = value;
                    normalize_key_ids(&mut layer);
                    cx.put_layer(&loc, layer)?;
                } else {
                    let mut p = SetProperty::new(&id, &path, pm_scene3d::to_model(&value), time);
                    if autokey {
                        p.mode = KeyMode::Keyframe;
                    }
                    properties::set_scene_property(cx, &p)?;
                }
            }
            Action::Metadata { id, patch } => {
                guard(cx, &id, "3d")?;
                LayerCommand::Set {
                    layer_id: id,
                    patch: LayerPatch {
                        name: patch["name"].as_str().map(str::to_owned),
                        visible: patch["visible"].as_bool(),
                        parent: patch.get("parent").map(|p| ParentChange {
                            parent: p.as_str().map(str::to_owned),
                            time,
                        }),
                        ..Default::default()
                    },
                }
                .plan(cx)?;
            }
            Action::Remove { id } => {
                guard(cx, &id, "3d")?;
                LayerCommand::Delete {
                    layer_ids: vec![id],
                }
                .plan(cx)?;
            }
            Action::Duplicate { id, new_id, name } => {
                guard(cx, &id, "3d")?;
                LayerCommand::Duplicate {
                    layer_ids: vec![id],
                    new_ids: vec![new_id.clone()],
                }
                .plan(cx)?;
                let (loc, mut copy) = cx.layer(&new_id)?;
                fresh_keys(&mut copy, true);
                cx.put_layer(&loc, copy)?;
                if let Some(name) = name {
                    LayerCommand::Set {
                        layer_id: new_id,
                        patch: LayerPatch {
                            name: Some(name),
                            ..Default::default()
                        },
                    }
                    .plan(cx)?;
                }
            }
        }
    }
    // Validate coupled camera values only after all channel edits. An authored
    // near+far pair may be temporarily invalid during the scratch sequence.
    let current = cx.comp_node(comp)?;
    for layer in crate::model::layers_of(current).filter(|layer| is_scene3d_layer(layer)) {
        let keys: &[_] = if pm_scene3d::layer_role(layer).is_some() {
            &["type", "p"]
        } else {
            &["type"]
        };
        let mut fields = pm_scene3d::node_fields(layer, keys);
        if let Some(d) = layer.get("d") {
            fields.insert(
                "d",
                Value::Object(pm_scene3d::node_fields(
                    d,
                    &["definition", "data", "extrusion"],
                )),
            );
        }
        validate_layer(&Value::Object(fields))?;
    }
    pm_scene3d::contract::validate_composition(&planning_composition(current, true)?)
        .map_err(invalid)?;
    Ok(())
}
fn fresh_keys(v: &mut Value, replace: bool) {
    match v {
        Value::Object(m) => {
            if let Some(Value::Array(a)) = m.get_mut("kf") {
                for k in a {
                    if let Some(m) = k.as_object_mut()
                        && (replace || m.get("i").is_none())
                    {
                        m.insert("i", Value::from(new_id("k")));
                    }
                }
            }
            for (_, v) in m.iter_mut() {
                fresh_keys(v, replace);
            }
        }
        Value::Array(a) => {
            for v in a {
                fresh_keys(v, replace);
            }
        }
        _ => {}
    }
}
pub(crate) fn path(layer: &Value, channel: &str) -> Option<Vec<Step>> {
    let parts:Vec<_>=channel.split('.').collect();
    let extrusion=match parts.as_slice() {
        ["geo",field]=>Some(vec![key("d"),key("extrusion"),key("p"),key(field)]),
        ["ms",surface,field] if pm_scene3d::extrusion::SURFACES.contains(surface)=>Some(vec![key("d"),key("extrusion"),key("surfaces"),key(surface),key("properties"),key(field)]),
        _=>None,
    };
    if let Some(path)=extrusion {return channels::get(layer,&path).is_some().then_some(path);}
    let def = layer.get("d")?.get("definition")?.as_str()?;
    let (prefix, key_name) = channel.split_once('.')?;
    let parts: Vec<_> = channel.split('.').collect();
    let mut extra = vec![
        key("d"),
        key("data"),
        key(if def == schema::LIGHT {
            "light"
        } else {
            "object"
        }),
    ];
    let known = if def == schema::OBJECT || def == schema::LIGHT && prefix == "world" {
        match parts.as_slice() {
            [kind, field] if pm_scene3d::contract::GROUPS.contains(kind) => {
                extra.extend([key(kind), key("p"), key(field)]);
                true
            }
            ["terrain", region, field] if region.starts_with("region") => {
                if let Ok(index) = region[6..].parse::<usize>() {
                    extra.extend([
                        key("terrain"),
                        key("regions"),
                        Step::Index(index),
                        key("p"),
                        key(field),
                    ]);
                    true
                } else {
                    false
                }
            }
            ["mi", field] => {
                extra.extend([key("materialProperties"), key(field)]);
                true
            }
            ["mi", index, field] => {
                extra.extend([key("sourceMaterialProperties"), key(index), key(field)]);
                true
            }
            ["pt", part, field] => {
                extra.extend([key("parts"), key(part), key("p"), key(field)]);
                true
            }
            _ => false,
        }
    } else {
        false
    };
    if known {
        return channels::get(layer, &extra).is_some().then_some(extra);
    }
    if key_name.contains('.') {
        return None;
    }
    let base: Vec<Step> = match (def, prefix) {
        (schema::OBJECT, "m") => vec![
            key("d"),
            key("data"),
            key("object"),
            key("material"),
            key("p"),
        ],
        (schema::LIGHT, "light") => vec![key("d"), key("data"), key("light"), key("p")],
        (schema::CAMERA, "camera" | "environment") => {
            vec![key("d"), key("data"), key(prefix), key("p")]
        }
        _ => return None,
    };
    let mut p = base;
    p.push(key(key_name));
    channels::get(layer, &p).is_some().then_some(p)
}
pub(crate) fn all(layer: &Value) -> Vec<(String, Vec<Step>)> {
    let mut out = vec![];
    if layer.get("d").and_then(|d|d.get("extrusion")).is_some() && let Ok(json)=pm_scene3d::from_model(layer) {
        for (name,_) in pm_scene3d::extrusion::channels(&json) {
            if let Some(p)=path(layer,&name){out.push((name,p));}
        }
    }
    for prefix in ["m", "light", "camera", "environment"] {
        let base = match prefix {
            "m" => vec![
                key("d"),
                key("data"),
                key("object"),
                key("material"),
                key("p"),
            ],
            _ => vec![key("d"), key("data"), key(prefix), key("p")],
        };
        if let Some(m) = channels::get(layer, &base).and_then(Value::as_object) {
            for (k, _) in m.iter() {
                let s = format!("{prefix}.{}", k.to_string_lossy());
                if let Some(p) = path(layer, &s) {
                    out.push((s, p));
                }
            }
        }
    }
    if let Some(object) = layer.get("d").and_then(|d| d.get("data")).and_then(|d| {
        d.get(
            if layer
                .get("d")
                .and_then(|d| d.get("definition"))
                .and_then(Value::as_str)
                == Some(schema::LIGHT)
            {
                "light"
            } else {
                "object"
            },
        )
    }) && let Ok(object) = pm_scene3d::from_model(object)
    {
        for (name, _) in pm_scene3d::contract::channels(&object) {
            if let Some(p) = path(layer, &name) {
                out.push((name, p));
            }
        }
    }
    out
}
/// Whether [`validate_layer`] has anything to check on a stored layer: only
/// 3D object, light and camera layers do. Read on the tree, so a transaction's
/// validation never copies the other layers of a large composition.
pub(crate) fn is_scene3d_layer(layer: &pm_model::Node) -> bool {
    layer.get("d").and_then(|d| d.get("extrusion")).is_some()
        || pm_scene3d::layer_role(layer).is_some()
}

pub(crate) fn validate_layer(layer: &Value) -> Result<(), CommandError> {
    if let Some(data) = layer.get("d").and_then(|d| d.get("extrusion")) {
        if !matches!(
            layer.get("type").and_then(Value::as_str),
            Some("text" | "shape")
        ) {
            return Err(invalid("Only text and shape layers support extrusion"));
        }
        pm_scene3d::extrusion::parse(&pm_scene3d::from_model(data).map_err(invalid)?)
            .map_err(invalid)?;
    }
    let definition = layer
        .get("d")
        .and_then(|d| d.get("definition"))
        .and_then(Value::as_str);
    if !matches!(
        definition,
        Some(schema::OBJECT | schema::LIGHT | schema::CAMERA)
    ) {
        return Ok(());
    }
    let j = pm_scene3d::from_model(layer).map_err(invalid)?;
    if let Some(role) = schema::role(&j) {
        schema::layer_data(role, j["d"]["data"].clone()).map_err(invalid)?;
        for (alias, path) in schema::TRANSFORMS {
            if let Some(v) = j["p"].get(*path) {
                let mut v = v.clone();
                if alias.starts_with('s') {
                    if let Some(x) = v["v"].as_f64() {
                        v["v"] = serde_json::json!(x / 100.);
                    }
                    if let Some(a) = v["kf"].as_array_mut() {
                        for k in a {
                            if let Some(x) = k["v"].as_f64() {
                                k["v"] = serde_json::json!(x / 100.);
                            }
                        }
                    }
                }
                schema::validate_channel("object", alias, v).map_err(invalid)?;
            }
        }
    }
    Ok(())
}
/// Load-time upgrade, preserving the entire original layer in the document.
/// Unrecognized data stays present with a diagnostic; it is never replaced by
/// blank geometry or a capability claim.
fn upgrade_project(document: &pm_model::Document) -> pm_model::Document {
    fn projection(comp: &pm_model::Node) -> Option<pm_model::Map> {
        let layers = pm_eval::node::array(comp.get("layers"))?;
        if !layers.iter().any(old_3d_layer) {
            return None;
        }
        let mut ids = std::collections::HashSet::new();
        let keyed = layers.iter().all(|layer| {
            layer
                .get("id")
                .and_then(crate::model::str_of)
                .is_some_and(|id| ids.insert(id))
        });
        let layers = layers
            .iter()
            .map(|layer| {
                // A malformed stack may need a whole-array patch. Keep its input
                // intact so such a patch preserves the other layers.
                if !keyed || old_3d_layer(layer) {
                    layer.to_value()
                } else {
                    // The migration reads ordinary IDs to avoid collisions.
                    Value::Object(pm_scene3d::node_fields(layer, &["id"]))
                }
            })
            .collect();
        let mut fields = pm_scene3d::node_fields(comp, &["fps", "dur", "w", "h"]);
        fields.insert("layers", Value::Array(layers));
        Some(fields)
    }
    let Some(root) = document.root() else {
        return document.clone();
    };
    let mut fields = projection(root).unwrap_or_default();
    if let Some(pm_model::NodeKind::Object(comps)) = root.get("comps").map(pm_model::Node::kind) {
        let comps: pm_model::Map = comps
            .iter()
            .filter_map(|(id, comp)| Some((id.clone(), Value::Object(projection(comp)?))))
            .collect();
        if !comps.is_empty() {
            fields.insert("comps", Value::Object(comps));
        }
    }
    if fields.is_empty() {
        return document.clone();
    }
    if has_unpaired_utf16(root) {
        return retain_unrepresentable_3d(
            document,
            "Studio 3D cannot read unpaired UTF-16 surrogates",
        );
    }
    let projected = Value::Object(fields);
    let mut json = match pm_scene3d::from_model(&projected) {
        Ok(json) => json,
        Err(error) => return retain_unrepresentable_3d(document, &error.to_string()),
    };
    let mut sampler = LegacySampler {
        queries: crate::queries::QueryContext::new(document),
    };
    if pm_scene3d::migration::upgrade_project_with(&mut json, &mut sampler).is_err() {
        return document.clone();
    }
    let before = pm_model::Node::from_value(projected, 0);
    let after = pm_model::Node::from_value(pm_scene3d::to_model(&json), 0);
    let ops = pm_model::ops::diff_node_ops(Some(&before), Some(&after));
    if ops.is_empty() {
        return document.clone();
    }
    let mut upgraded = document.clone();
    if upgraded.apply_ops(&ops).is_err() {
        return document.clone();
    }
    pm_model::Document::reconciled_node(document, upgraded.root().cloned(), document.revision())
}
// Keep the load-time diagnostic for any string the migration's JSON bridge
// cannot read. The scan borrows the existing strings and never copies paths.
fn has_unpaired_utf16(node: &pm_model::Node) -> bool {
    use pm_model::NodeKind;
    match node.kind() {
        NodeKind::String(s) => s.as_str().is_none(),
        NodeKind::Array(items) => items.iter().any(has_unpaired_utf16),
        NodeKind::Object(fields) => fields
            .iter()
            .any(|(key, value)| key.as_str().is_none() || has_unpaired_utf16(value)),
        _ => false,
    }
}

fn retain_unrepresentable_3d(
    document: &pm_model::Document,
    diagnostic: &str,
) -> pm_model::Document {
    use pm_model::patch::{Patch, PathPart};

    fn mark_layers(
        comp: &pm_model::Node,
        prefix: &[PathPart],
        diagnostic: &str,
        patches: &mut Vec<Patch>,
    ) {
        for (index, layer) in crate::model::layers_of(comp).enumerate() {
            if !old_3d_layer(layer) {
                continue;
            }
            let mut base = prefix.to_vec();
            base.extend(["layers".into(), PathPart::Index(index as f64)]);
            let set = |keys: &[&str], value| {
                let mut path = base.clone();
                path.extend(keys.iter().map(|&key| PathPart::from(key)));
                Patch::set(path, value)
            };
            if layer.get("native3dArchive").is_none() {
                let mut archive = pm_model::Map::new();
                archive.insert("version", Value::Number(1.));
                archive.insert("source", layer.to_value());
                patches.push(set(&["native3dArchive"], Value::Object(archive)));
            }
            patches.push(set(
                &["d", "definition"],
                Value::from("powermove.3d.unupgraded"),
            ));
            patches.push(set(&["native3dUpgradeDiagnostic"], Value::from(diagnostic)));
            patches.push(set(
                &["native3dUpgradeIssue"],
                Value::from("This 3D layer couldn’t be upgraded."),
            ));
        }
    }

    let Some(mut root) = document.root().cloned() else {
        return document.clone();
    };
    let mut patches = Vec::new();
    mark_layers(&root, &[], diagnostic, &mut patches);
    if let Some(pm_model::NodeKind::Object(comps)) = root.get("comps").map(pm_model::Node::kind) {
        for (id, comp) in comps.iter() {
            mark_layers(
                comp,
                &["comps".into(), PathPart::Key(id.clone())],
                diagnostic,
                &mut patches,
            );
        }
    }
    if root.apply_patches(&patches, document.revision()).is_err() {
        return document.clone();
    }
    pm_model::Document::reconciled_node(document, Some(root), document.revision())
}
struct LegacySampler {
    queries: crate::queries::QueryContext,
}
impl pm_scene3d::migration::LegacySampler for LegacySampler {
    fn sample(
        &mut self,
        id: &str,
        key: &str,
        prop: &serde_json::Value,
        time: f64,
    ) -> pm_scene3d::Result<serde_json::Value> {
        let root = self
            .queries
            .document()
            .root()
            .ok_or_else(|| pm_scene3d::Error("No migration document".into()))?;
        let loc = crate::model::find_layer(root, id)
            .ok_or_else(|| pm_scene3d::Error("Migration layer not found".into()))?;
        let comp = loc
            .comp_id
            .or_else(|| crate::model::open_comp_id(root))
            .unwrap_or_else(|| {
                format!(
                    "@root:{}",
                    root.get("id").and_then(crate::model::str_of).unwrap_or("")
                )
            });
        let request = serde_json::json!({"op":"evP","comp":comp,"open":comp,"layer":loc.index,"key":key,"prop":prop,"t":time});
        let value = self
            .queries
            .query(&pm_scene3d::to_model(&request))
            .map_err(|e| pm_scene3d::Error(e.to_string()))?;
        pm_scene3d::from_model(&value)
    }
}

/// Whether the document or its history holds old 3D objects, read in place
/// (a large project is not copied out just to learn it has none).
pub(crate) fn needs_upgrade(document: &pm_model::Document, history: &crate::History) -> bool {
    fn value(v: &Value) -> bool {
        match v {
            Value::Object(m) => {
                m.get("definition")
                    .and_then(Value::as_str)
                    .is_some_and(is_old_3d)
                    || m.iter().any(|(_, v)| value(v))
            }
            Value::Array(a) => a.iter().any(value),
            _ => false,
        }
    }
    document.root().is_some_and(|root| {
        let has_old_layers = |comp| crate::model::layers_of(comp).any(old_3d_layer);
        has_old_layers(root)
            || matches!(root.get("comps").map(pm_model::Node::kind),
                Some(pm_model::NodeKind::Object(comps)) if comps.iter().any(|(_, comp)| has_old_layers(comp)))
    })
        || history
            .records()
            .iter()
            .filter(|r| r.forward.may_have_old_3d() || r.inverse.may_have_old_3d())
            .flat_map(|r| r.forward.iter().chain(r.inverse.iter()))
            .any(|op| matches!(op, pm_model::Op::Leaf(p) if p.value.as_ref().is_some_and(value)))
}

fn old_3d_layer(layer: &pm_model::Node) -> bool {
    layer
        .get("d")
        .and_then(|d| d.get("definition"))
        .and_then(crate::model::str_of)
        .is_some_and(is_old_3d)
}

fn is_old_3d(definition: &str) -> bool {
    matches!(
        definition,
        schema::SCENE | schema::OBJECT | schema::LIGHT | schema::CAMERA | "powermove.3d.obj-model"
    )
}

/// Upgrade each imported history state as well as the current document. Stable
/// deterministic migration IDs make before/after diffs preserve Undo and Redo.
/// Migrated undo is kept in the bounded history store, never in scene metadata.
pub(crate) fn upgrade_document_history(
    document: pm_model::Document,
    history: crate::History,
) -> (pm_model::Document, crate::History) {
    if !needs_upgrade(&document, &history) {
        return (document, history);
    }
    let upgraded = upgrade_project(&document);
    if pm_eval::node::number(
        document
            .root()
            .and_then(|root| root.get("native3dHistoryVersion")),
    ) == Some(1.)
        || history.is_empty()
    {
        return (upgraded, history);
    }
    let mut original = document.clone();
    let cursor = history.applied();
    let mut revision = original.revision();
    let mut needed = upgraded.root() != document.root();
    let run = (|| -> Result<crate::History, String> {
        for record in history.records()[..cursor].iter().rev() {
            revision += 1;
            original
                .apply_ops_at(&record.inverse.to_vec(), revision)
                .map_err(|e| e.to_string())?;
        }
        if original.root().is_none() {
            return Err("History removed the document".into());
        }
        let mut before = upgrade_project(&original);
        needed |= before.root() != original.root();
        let mut next = crate::History::with_limits(history.len().max(1), usize::MAX);
        for record in history.records() {
            if !record.is_document() {
                return Err("Imported 3D history contains a non-document entry".into());
            }
            revision += 1;
            original
                .apply_ops_at(&record.forward.to_vec(), revision)
                .map_err(|e| e.to_string())?;
            if original.root().is_none() {
                return Err("History removed the document".into());
            }
            let after = upgrade_project(&original);
            needed |= after.root() != original.root();
            let forward = pm_model::ops::diff_node_ops(before.root(), after.root());
            let inverse = pm_model::ops::diff_node_ops(after.root(), before.root());
            next.push(
                record.label.clone(),
                record.origin,
                forward,
                inverse,
                None,
                0,
            );
            before = after;
        }
        while next.applied() > cursor {
            next.mark_undone();
        }
        next.set_max_bytes(history.stats().max_bytes);
        Ok(next)
    })();
    if run.is_ok() && !needed {
        return (document, history);
    }
    let mut root = upgraded.root().cloned().expect("checked");
    let revision = upgraded.revision();
    let mut patches = vec![pm_model::patch::Patch::set(
        vec!["native3dHistoryVersion".into()],
        Value::Number(1.),
    )];
    let history = match run {
        Ok(next) => next,
        Err(reason) => {
            patches.push(pm_model::patch::Patch::set(
                vec!["native3dHistoryIssue".into()],
                Value::from(reason),
            ));
            crate::History::new()
        }
    };
    root.apply_patches(&patches, revision)
        .expect("project metadata");
    (
        pm_model::Document::reconciled_node(&upgraded, Some(root), revision),
        history,
    )
}

/// Prepare geometry without blocking the actor or accessing JS. Asset providers
/// must answer for the supplied durable metadata revision, or return an error.
pub fn prepare_geometry(
    snapshot: &crate::Snapshot,
    comp: Option<String>,
    args: &Value,
    selection: &[String],
    time: f64,
    assets: &dyn pm_scene3d::geometry::AssetGeometry,
) -> Result<pm_scene3d::geometry::EvaluatedGeometry, CommandError> {
    let root = snapshot.root().ok_or_else(|| invalid("No project"))?;
    let resolved_comp = comp.clone().or_else(|| snapshot.open_comp());
    let comp_node = crate::model::comp_node(root, resolved_comp.as_deref())
        .ok_or_else(|| invalid("No composition"))?;
    let json = planning_composition(comp_node, false)?;
    let args = pm_scene3d::from_model(args).map_err(invalid)?;
    let ids = pm_scene3d::geometry::required_ids(&json, &args, selection).map_err(invalid)?;
    let mut queries = crate::queries::QueryContext::new(&snapshot.document);
    let comp_id = comp
        .clone()
        .or_else(|| snapshot.open_comp())
        .unwrap_or_else(|| {
            format!(
                "@root:{}",
                root.get("id").and_then(crate::model::str_of).unwrap_or("")
            )
        });
    let mut bounds = serde_json::json!({});
    let mut parents = serde_json::json!({});
    let mut records = serde_json::json!({});
    for id in ids {
        let loc =
            crate::model::find_layer(root, &id).ok_or_else(|| invalid("3D layer not found"))?;
        let layer =
            crate::model::layer_at(root, &loc).ok_or_else(|| invalid("3D layer not found"))?;
        let request = |op: &str| {
            pm_scene3d::to_model(
                &serde_json::json!({"op":op,"comp":comp_id,"open":comp_id,"layer":loc.index,"t":time}),
            )
        };
        if queries.query(&request("active")).map_err(invalid)? != Value::Bool(true) {
            continue;
        }
        let source = layer
            .get("d")
            .and_then(|d| d.get("data")?.get("object")?.get("source"));
        let source = pm_scene3d::from_model(&source.map_or(Value::Null, pm_model::Node::to_value))
            .map_err(invalid)?;
        if let Some(asset) = source["assetId"].as_str()
            && records.get(asset).is_none()
            && let Some(record) = root.get("assets").and_then(|assets| assets.get(asset))
        {
            records[asset] = pm_scene3d::from_model(&record.to_value()).map_err(invalid)?;
        }
        let local = pm_scene3d::geometry::local_bounds(
            &source,
            assets,
            &records,
            time - pm_eval::node::number(layer.get("from")).unwrap_or(0.),
        )
        .map_err(invalid)?;
        let world = pm_scene3d::geometry::matrix(
            &pm_scene3d::from_model(&queries.query(&request("world3D")).map_err(invalid)?)
                .map_err(invalid)?,
        )
        .map_err(invalid)?;
        let parent = pm_scene3d::geometry::matrix(
            &pm_scene3d::from_model(&queries.query(&request("parent3D")).map_err(invalid)?)
                .map_err(invalid)?,
        )
        .map_err(invalid)?;
        bounds[&id] = pm_scene3d::geometry::bounds_json(
            pm_scene3d::geometry::transform(local, world).map_err(invalid)?,
        );
        parents[&id] = serde_json::json!(pm_scene3d::geometry::inverse(parent).map_err(invalid)?);
    }
    Ok(pm_scene3d::geometry::EvaluatedGeometry {
        source: root.clone(),
        comp,
        time,
        bounds,
        parent_inverse: parents,
    })
}

fn import(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    asset_id: &str,
    layer_id: &str,
    receipt: &pm_scene3d::import::archive::Receipt,
    time: f64,
    setup_studio: bool,
) -> Result<(), CommandError> {
    if cx.lookup(&["assets".into(), asset_id.into()]).is_some() {
        return Err(invalid("3D import asset ID already exists"));
    }
    let empty = crate::model::layers_of(cx.comp_node(comp)?).all(|l| {
        !matches!(
            pm_scene3d::layer_role(l),
            Some(schema::Role::Light | schema::Role::Camera)
        )
    });
    let metadata = import_metadata(asset_id, receipt);
    super::MediaCommand::AddAsset {
        asset_id: asset_id.into(),
        asset: pm_scene3d::to_model(&metadata),
    }
    .plan(cx)?;
    let args = serde_json::json!({"operation":"add_object","id":layer_id,"name":receipt.primary().name(),"object":{"source":{"assetId":asset_id},"useSourceMaterials":!receipt.primary().name().to_ascii_lowercase().ends_with(".obj")}});
    apply(
        cx,
        comp,
        &pm_scene3d::to_model(&args),
        &[],
        time,
        false,
        None,
    )?;
    import_placement::place(cx, comp, layer_id, receipt, time, setup_studio)?;
    if empty && setup_studio {
        let assets = import_placement::AssetBounds;
        {
            let operation = serde_json::json!({"operation":"auto_studio","target":layer_id});
            let snapshot = crate::Snapshot {
                doc_id: 0,
                revision: 0,
                document: std::sync::Arc::new(pm_model::Document::from_root(
                    Some(cx.root().clone()),
                    0,
                )),
            };
            let args = pm_scene3d::to_model(&operation);
            let geometry = prepare_geometry(
                &snapshot,
                comp.map(str::to_owned),
                &args,
                &[],
                time,
                &assets,
            )?;
            apply(cx, comp, &args, &[], time, false, Some(&geometry))?;
        }
    }
    Ok(())
}
fn original_ids(id: &str, receipt: &pm_scene3d::import::archive::Receipt) -> Vec<String> {
    let mut by_hash = std::collections::HashMap::from([(receipt.primary().hash(), id.to_string())]);
    receipt
        .originals()
        .iter()
        .enumerate()
        .map(|(i, b)| {
            by_hash
                .entry(b.hash())
                .or_insert_with(|| format!("{id}-source-{i}"))
                .clone()
        })
        .collect()
}
fn import_metadata(id: &str, receipt: &pm_scene3d::import::archive::Receipt) -> serde_json::Value {
    let b = receipt.primary();
    let sources=receipt.originals().iter().zip(original_ids(id,receipt)).map(|(b,source_id)|serde_json::json!({"id":source_id,"name":b.name(),"mime":b.mime(),"size":b.length(),"sha256":b.hash()})).collect::<Vec<_>>();
    serde_json::json!({"id":id,"name":b.name(),"kind":"model","format":b.name().rsplit('.').next().unwrap_or(""),"size":b.length(),"persisted":true,"revision":b.hash(),"native3dSources":sources,"native3dBounds":{"version":1,"bounds":pm_scene3d::geometry::bounds_json(receipt.bounds())},"native3dMaterials":pm_scene3d::import::material_slots_json(&receipt.model())})
}
pub(crate) fn import_media(command: &crate::Command) -> Vec<crate::project_io::MediaEntry> {
    let (asset_id, receipt) = match command {
        crate::Command::Scene3d(
            Scene3dCommand::Import {
                asset_id, receipt, ..
            }
            | Scene3dCommand::BindImport { asset_id, receipt },
        ) => (asset_id, receipt),
        _ => return vec![],
    };
    let make = |id: String, b: &pm_scene3d::import::archive::Blob| crate::project_io::MediaEntry {
        id: id.as_str().into(),
        media_type: b.mime().into(),
        revision: b.hash().into(),
        source: crate::project_io::MediaSource::File {
            path: b.path().to_owned(),
            length: b.length(),
            sha256: Some(b.hash().into()),
        },
    };
    let mut media = vec![make(asset_id.clone(), receipt.primary())];
    let mut seen = std::collections::HashSet::from([asset_id.clone()]);
    for (b, id) in receipt
        .originals()
        .iter()
        .zip(original_ids(asset_id, receipt))
    {
        if seen.insert(id.clone()) {
            media.push(make(id, b));
        }
    }
    media
}
pub(crate) fn collect_original_assets(value: &Value, ids: &mut std::collections::HashSet<String>) {
    match value {
        Value::Object(m) => {
            if let Some(Value::Array(sources)) = m.get("native3dSources") {
                for source in sources {
                    if let Some(id) = source.get("id").and_then(Value::as_str) {
                        ids.insert(id.into());
                    }
                }
            }
            for (_, v) in m.iter() {
                collect_original_assets(v, ids);
            }
        }
        Value::Array(a) => {
            for v in a {
                collect_original_assets(v, ids);
            }
        }
        _ => {}
    }
}

/// [`collect_original_assets`] over the document's own nodes: the same ids,
/// without first copying the whole project into a value.
pub(crate) fn collect_original_assets_in(
    node: &pm_model::Node,
    ids: &mut std::collections::HashSet<String>,
) {
    use pm_model::NodeKind;
    match node.kind() {
        NodeKind::Object(map) => {
            if let Some(NodeKind::Array(sources)) = node.get("native3dSources").map(|n| n.kind()) {
                for source in sources.iter() {
                    if let Some(NodeKind::String(id)) = source.get("id").map(|n| n.kind())
                        && let Some(id) = id.as_str()
                    {
                        ids.insert(id.into());
                    }
                }
            }
            for (_, child) in map.iter() {
                collect_original_assets_in(child, ids);
            }
        }
        NodeKind::Array(items) => {
            for child in items.iter() {
                collect_original_assets_in(child, ids);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[test]
fn the_node_walk_finds_the_same_originals_as_the_value_walk() {
    let value = pm_model::json::parse(
        r#"{"layers":[{"id":"a","native3dSources":[{"id":"o1"},{"id":2},{"x":1}]}],
            "comps":{"c":{"layers":[{"d":{"native3dSources":[{"id":"o2"}]}},
            {"native3dSources":{"id":"not-an-array"}}]}},"assets":{"o3":{}}}"#,
    )
    .unwrap();
    let (mut a, mut b) = (Default::default(), Default::default());
    collect_original_assets(&value, &mut a);
    collect_original_assets_in(&pm_model::Node::from_value(value, 1), &mut b);
    assert_eq!(a, b);
    assert_eq!(a.len(), 2);
}

pub(crate) fn is_known_layer(layer: &Value) -> bool {
    matches!(
        layer
            .get("d")
            .and_then(|d| d.get("definition"))
            .and_then(Value::as_str),
        Some(schema::OBJECT | schema::LIGHT | schema::CAMERA)
    )
}
pub(crate) fn make_known_layer(
    id: &str,
    name: Option<&str>,
    comp: (f64, f64, f64),
    content: &pm_model::Map,
    properties: &pm_model::Map,
) -> Option<Result<Value, CommandError>> {
    let role = match content.get("definition").and_then(Value::as_str) {
        Some(schema::OBJECT) => schema::Role::Object,
        Some(schema::LIGHT) => schema::Role::Light,
        Some(schema::CAMERA) => schema::Role::Camera,
        _ => return None,
    };
    Some((|| {
        if content
            .get("version")
            .and_then(Value::as_f64)
            .is_some_and(|v| v != 1.)
        {
            return Err(invalid("Unsupported 3D definition version"));
        }
        let data = match content.get("data") {
            Some(d) => pm_scene3d::from_model(d).map_err(invalid)?,
            None => match role {
                schema::Role::Object => {
                    serde_json::json!({"object":{"source":{"primitive":"box"}}})
                }
                schema::Role::Light => serde_json::json!({"light":{}}),
                schema::Role::Camera => serde_json::json!({"camera":{}}),
            },
        };
        let props = pm_scene3d::from_model(&Value::Object(properties.clone())).map_err(invalid)?;
        let mut layer = pm_scene3d::service::new_layer(
            &serde_json::json!({"w":comp.0,"h":comp.1,"dur":comp.2}),
            id,
            name.unwrap_or(match role {
                schema::Role::Object => "3D Model",
                schema::Role::Light => "Sun",
                schema::Role::Camera => "Camera",
            }),
            role,
            data,
            props,
            true,
            None,
            0.,
        )
        .map_err(invalid)?;
        for k in ["w", "h"] {
            if let Some(v) = content.get(k) {
                let n = v
                    .as_f64()
                    .filter(|n| n.is_finite() && *n > 0.)
                    .ok_or_else(|| invalid("Invalid 3D dimensions"))?;
                layer["d"][k] = serde_json::json!(n);
            }
        }
        let mut layer = pm_scene3d::to_model(&layer);
        normalize_key_ids(&mut layer);
        Ok(layer)
    })())
}
pub(crate) fn set_known_content(
    cx: &mut Cx<'_>,
    id: &str,
    patch: &pm_model::Map,
) -> Result<(), CommandError> {
    for (k, _) in patch.iter() {
        if !matches!(k.as_str(), Some("data" | "w" | "h" | "parameters")) {
            return Err(invalid("Unknown 3D content field"));
        }
    }
    if let Some(data) = patch.get("data") {
        set_data(cx, id, data.clone())?;
    }
    if let Some(params) = patch.get("parameters")
        && params.as_object().is_none_or(|m| !m.is_empty())
    {
        return Err(invalid("3D definitions have no extension parameters"));
    }
    let (loc, mut layer) = cx.layer_mut_checked(id)?;
    for k in ["w", "h"] {
        if let Some(value) = patch.get(k) {
            let n = value
                .as_f64()
                .filter(|v| v.is_finite() && *v > 0.)
                .ok_or_else(|| invalid("Invalid 3D dimensions"))?;
            values::obj(
                values::obj(&mut layer)
                    .get_mut("d")
                    .ok_or_else(|| invalid("Missing 3D content"))?,
            )
            .insert(k, Value::Number(n));
        }
    }
    cx.put_layer(&loc, layer)
}

fn normalize_key_ids(layer: &mut Value) {
    use sha2::{Digest, Sha256};
    let id = layer
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let paths = channels::all(layer);
    let mut seen = std::collections::HashSet::new();
    for (channel, path) in paths {
        if let Some(prop) = channels::get_mut(layer, &path)
            && let Some(keys) = prop
                .as_object_mut()
                .and_then(|m| m.get_mut("kf"))
                .and_then(Value::as_array_mut)
        {
            for (i, key) in keys.iter_mut().enumerate() {
                let valid = key
                    .get("i")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .is_some_and(|s| seen.insert(s.to_string()));
                if !valid {
                    let mut suffix = 0;
                    let next = loop {
                        let hash =
                            Sha256::digest(format!("{id}\0{channel}\0{i}\0{suffix}").as_bytes());
                        let k = format!(
                            "k{}",
                            hash[..12]
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<String>()
                        );
                        if seen.insert(k.clone()) {
                            break k;
                        }
                        suffix += 1;
                    };
                    if let Some(m) = key.as_object_mut() {
                        m.insert("i", Value::from(next));
                    }
                }
            }
        }
    }
}
/// Renderer/headless service projection, evaluated from one immutable snapshot.
/// Call on a worker; it neither mutates the actor nor loads platform UI code.
pub fn resolve_scene(
    snapshot: &crate::Snapshot,
    comp: Option<&str>,
    time: f64,
) -> Result<pm_scene3d::service::ResolvedScene, CommandError> {
    resolve_scene_with_camera(snapshot, comp, time, None)
}
/// A transient viewer camera selection; authored data and history are untouched.
pub fn resolve_scene_with_camera(
    snapshot: &crate::Snapshot,
    comp: Option<&str>,
    time: f64,
    camera: Option<&str>,
) -> Result<pm_scene3d::service::ResolvedScene, CommandError> {
    struct Evaluator<'a, 'scope, 'd> {
        document: &'d pm_model::Document,
        scope: Option<&'a mut pm_eval::Scope<'scope, 'd>>,
        queries: Option<crate::queries::QueryContext>,
        comp: &'a str,
        camera: Option<&'a str>,
        layer_indices: std::collections::HashMap<&'d str, usize>,
    }
    impl<'d> Evaluator<'_, '_, 'd> {
        fn read<T>(
            &mut self,
            layer: &serde_json::Value,
            read: impl FnOnce(
                &mut pm_eval::Scope<'_, 'd>,
                &'d pm_model::Node,
            ) -> pm_eval::js_sem::JsResult<T>,
        ) -> Option<pm_scene3d::Result<T>> {
            let index = *self.layer_indices.get(layer["id"].as_str()?)?;
            let scope = self.scope.as_deref_mut()?;
            let node = scope.layers().get(index)?;
            Some(read(scope, node).map_err(|e| pm_scene3d::Error(e.message)))
        }

        fn query(
            &mut self,
            op: &str,
            l: &serde_json::Value,
            t: f64,
            extra: serde_json::Value,
        ) -> pm_scene3d::Result<serde_json::Value> {
            let i = self
                .layer_indices
                .get(l["id"].as_str().unwrap_or_default())
                .ok_or_else(|| pm_scene3d::Error("3D layer moved".into()))?;
            let mut request =
                serde_json::json!({"op":op,"comp":self.comp,"open":self.comp,"layer":i,"t":t});
            for (k, v) in extra.as_object().unwrap() {
                request[k] = v.clone();
            }
            let value = self
                .queries
                .get_or_insert_with(|| crate::queries::QueryContext::new(self.document))
                .query(&pm_scene3d::to_model(&request))
                .map_err(|e| pm_scene3d::Error(e.to_string()))?;
            pm_scene3d::from_model(&value)
        }
    }
    impl pm_scene3d::service::Evaluator for Evaluator<'_, '_, '_> {
        fn value(
            &mut self,
            l: &serde_json::Value,
            path: &str,
            c: &serde_json::Value,
            t: f64,
        ) -> pm_scene3d::Result<serde_json::Value> {
            // QueryContext has no channel overrides. Mirror evP's constant case
            // without a WASM/JSON round trip for every default material/control.
            // Animated, expressed and malformed channels retain the evaluator.
            if c.get("expr")
                .is_none_or(|e| e.is_null() || e.as_str() == Some(""))
                && c.get("kf")
                    .is_none_or(|k| k.as_array().is_some_and(Vec::is_empty))
                && let Some(value) = c.get("v")
            {
                return Ok(value.clone());
            }
            self.query("evP", l, t, serde_json::json!({"key":path,"prop":c}))
        }
        fn active(&mut self, l: &serde_json::Value, t: f64) -> pm_scene3d::Result<bool> {
            if let Some(camera) = &self.camera
                && pm_scene3d::schema::role(l) == Some(pm_scene3d::schema::Role::Camera)
            {
                return Ok(l["id"] == *camera);
            }
            if let Some(result) = self.read(l, |scope, node| {
                if node.get("on").and_then(crate::model::bool_of) == Some(false)
                    || pm_eval::evaluator::outside_layer_span(node, t)
                {
                    return Ok(false);
                }
                scope.active(node, t)
            }) {
                return result;
            }
            self.query("active", l, t, serde_json::json!({}))
                .map(|v| v == true)
        }
    }
    // With no parent or group, no stack lookup is needed to evaluate the
    // model's world transform. The local transform still uses the evaluator.
    fn has_ancestor(layer: &pm_model::Node) -> bool {
        pm_eval::node::truthy(layer.get("parent")) || pm_eval::node::truthy(layer.get("group"))
    }

    impl pm_scene3d::service::WorldEvaluator for Evaluator<'_, '_, '_> {
        fn world(&mut self, l: &serde_json::Value, t: f64) -> pm_scene3d::Result<[f64; 16]> {
            if let Some(result) = self.read(l, |scope, node| {
                use pm_eval::space3d::{IDENTITY4, multiply};
                if has_ancestor(node) {
                    scope.world3d(node, t, false)
                } else {
                    scope.local3d(node, t).map(|local| multiply(&IDENTITY4, &local))
                }
            }) {
                return result;
            }
            pm_scene3d::geometry::matrix(&self.query("world3D", l, t, serde_json::json!({}))?)
        }
        fn parent(&mut self, l: &serde_json::Value, t: f64) -> pm_scene3d::Result<[f64; 16]> {
            if let Some(result) = self.read(l, |scope, node| {
                if has_ancestor(node) {
                    scope.parent3d(node, t, None)
                } else {
                    Ok(pm_eval::space3d::IDENTITY4)
                }
            }) {
                return result;
            }
            pm_scene3d::geometry::matrix(&self.query("parent3D", l, t, serde_json::json!({}))?)
        }
        fn opacity(&mut self, l: &serde_json::Value, t: f64) -> pm_scene3d::Result<f64> {
            if let Some(result) = self.read(l, |scope, node| {
                if pm_eval::node::truthy(node.get("group")) {
                    scope.world_opacity(node, t)
                } else {
                    scope.num(node, "opacity", t).map(|n| (n / 100.).clamp(0., 1.))
                }
            }) {
                return result.and_then(|opacity| {
                    if opacity.is_finite() {
                        Ok(opacity)
                    } else {
                        Err(pm_scene3d::Error("Invalid opacity".into()))
                    }
                });
            }
            self.query("worldOpacity", l, t, serde_json::json!({}))?
                .as_f64()
                .ok_or_else(|| pm_scene3d::Error("Invalid opacity".into()))
        }
    }
    let root = snapshot.root().ok_or_else(|| invalid("No project"))?;
    let comp = comp.map(str::to_owned).or_else(|| snapshot.open_comp());
    let c =
        crate::model::comp_node(root, comp.as_deref()).ok_or_else(|| invalid("No composition"))?;
    // The resolver consumes Studio roles, 3D nulls, and extrusion settings.
    // Their evaluator still sees the full stack through the original indices,
    // including flat parents and groups that are absent from this projection.
    let source_layers = composition_layers(c)?;
    let mut pending_ids = std::collections::HashSet::new();
    let mut layers = Vec::new();
    for layer in source_layers {
        let content = layer.get("d");
        let studio = content.and_then(pm_scene3d::role_from_content).is_some();
        let extrusion = content.and_then(|d| d.get("extrusion"));
        let null = pm_eval::node::is_str(layer.get("type"), "null")
            && layer.get("threeD").and_then(crate::model::bool_of) == Some(true);
        if !studio && extrusion.is_none() && !null {
            continue;
        }
        if let Some(id) = layer.get("id").and_then(crate::model::str_of) {
            pending_ids.insert(id);
        }
        let mut fields = pm_scene3d::node_fields(layer, &["id", "type", "threeD", "name"]);
        let mut data = pm_model::Map::new();
        if studio {
            for (key, value) in pm_scene3d::node_fields(layer, &["from", "dur", "p", "parent"]) {
                fields.insert(key, value);
            }
            data = pm_scene3d::node_fields(
                content.expect("Studio definition"),
                &["definition", "data"],
            );
            // Resolve only mask visibility. The world evaluator still sees
            // the original masks and their geometry.
            if let Some(masks) = pm_eval::node::array(layer.get("masks")) {
                fields.insert(
                    "masks",
                    Value::Array(
                        masks
                            .iter()
                            .map(|mask| Value::Object(pm_scene3d::node_fields(mask, &["id", "on"])))
                            .collect(),
                    ),
                );
            }
        }
        if let Some(extrusion) = extrusion {
            data.insert("extrusion", extrusion.to_value());
        }
        if !data.is_empty() {
            fields.insert("d", Value::Object(data));
        }
        layers.push(Value::Object(fields));
    }
    let mut evaluated = pm_scene3d::node_fields(c, &["studioQuality"]);
    evaluated.insert("layers", Value::Array(layers));
    let c = pm_scene3d::from_model(&Value::Object(evaluated)).map_err(invalid)?;
    let comp = comp.unwrap_or_else(|| {
        format!(
            "@root:{}",
            root.get("id").and_then(crate::model::str_of).unwrap_or("")
        )
    });
    let layers = c["layers"].as_array().ok_or_else(|| invalid("No layers"))?;
    if let Some(camera) = camera
        && !layers.iter().any(|l| {
            l["id"] == camera
                && pm_scene3d::schema::role(l) == Some(pm_scene3d::schema::Role::Camera)
        })
    {
        return Err(invalid("View camera layer is missing"));
    }
    let mut layer_indices = std::collections::HashMap::with_capacity(pending_ids.len());
    for (index, layer) in source_layers.iter().enumerate() {
        if pending_ids.is_empty() {
            break;
        }
        // Keep the first occurrence, even if an imported stack has duplicate
        // IDs and the first layer for this ID had no Studio controls.
        if let Some(id) = layer.get("id").and_then(crate::model::str_of)
            && pending_ids.remove(id)
        {
            layer_indices.insert(id, index);
        }
    }
    // Share one evaluator for activity, transforms and opacity. Animated
    // detached properties can still use the raw query path when needed.
    let address = pm_eval::Comp::resolve(&snapshot.document, &comp).ok();
    let mut eval = pm_eval::Evaluator::new()
        .with_expressions(std::sync::Arc::new(pm_wasm::expr::PmExpressions));
    let mut scope = address.as_ref().map(|comp| {
        let mut scope = pm_eval::Scope::open(&mut eval, comp);
        scope.memoize_transforms();
        scope
    });
    pm_scene3d::service::resolve(
        &c,
        time,
        &mut Evaluator {
            document: &snapshot.document,
            scope: scope.as_mut(),
            queries: None,
            comp: &comp,
            camera,
            layer_indices,
        },
    )
    .map_err(invalid)
}
