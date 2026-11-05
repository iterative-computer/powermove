//! Studio commands run entirely on the kernel scratch tree. No renderer or JS
//! callback participates in validation, history capture or publication.
use super::{
    Cx,
    channels::{self, Step, key},
    layers::{LayerCommand, LayerPatch, ParentChange},
    new_id,
    properties::{self, KeyMode, PropertyCommand, SetProperty},
    values,
};
use crate::error::CommandError;
use pm_model::{Node, Value};
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
    /// Automatic document upgrade is also available as an explicit undoable edit.
    Upgrade,
}
fn invalid(e: impl std::fmt::Display) -> CommandError {
    CommandError::Invalid(e.to_string())
}
impl Scene3dCommand {
    pub fn label(&self) -> String {
        "Edit 3D Scene".into()
    }
    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            Self::SetData { layer_id, data } => set_data(cx, layer_id, data.clone()),
            Self::Upgrade => {
                let value = cx.root().to_value();
                let mut json = pm_scene3d::from_model(&value).map_err(invalid)?;
                pm_scene3d::migration::upgrade_project(&mut json).map_err(invalid)?;
                cx.replace(vec![], pm_scene3d::to_model(&json))
            }
            Self::Edit { layer_id, ops } => {
                let raw = pm_scene3d::from_model(ops).map_err(invalid)?;
                let list = raw.as_array().cloned().unwrap_or_else(|| vec![raw]);
                for mut args in list {
                    if args.get("target").is_none() && args.get("id").is_none() {
                        args["target"] = serde_target(layer_id);
                    }
                    apply(cx, None, &pm_scene3d::to_model(&args), &[], 0., false)?;
                }
                Ok(())
            }
            Self::Apply {
                comp,
                args,
                selection,
                time,
                autokey,
            } => apply(cx, comp.as_deref(), args, selection, *time, *autokey),
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
                        && ["m.", "light.", "camera.", "environment."]
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
fn apply(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    args: &Value,
    selection: &[String],
    time: f64,
    autokey: bool,
) -> Result<(), CommandError> {
    if !time.is_finite() {
        return Err(invalid("Invalid 3D time"));
    }
    let json = pm_scene3d::from_model(&cx.comp_node(comp)?.to_value()).map_err(invalid)?;
    let args = pm_scene3d::from_model(args).map_err(invalid)?;
    let Plan { mut actions, .. } =
        pm_scene3d::operations::plan(&json, &args, selection, &mut new_id).map_err(invalid)?;
    // Content structural updates precede channel writes so a material-map edit
    // cannot restore the previous material channels later in the same plan.
    actions.sort_by_key(|a| {
        if matches!(a, Action::Data { .. }) {
            0
        } else {
            1
        }
    });
    for action in actions {
        match action {
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
                cx.insert_layer(comp, 0, pm_scene3d::to_model(&layer))?;
            }
            Action::Data { id, data } => set_data(cx, &id, pm_scene3d::to_model(&data))?,
            Action::Channel { id, path, value } => {
                guard(cx, &id, &path)?;
                if value.is_object() {
                    let (loc, mut layer) = cx.layer_mut_checked(&id)?;
                    let p = channels::materialize(&mut layer, &path)
                        .ok_or_else(|| invalid(format!("Unknown 3D channel: {path}")))?;
                    let mut value = pm_scene3d::to_model(&value);
                    fresh_keys(&mut value, false);
                    *channels::get_mut(&mut layer, &p).expect("resolved") = value;
                    cx.put_layer(&loc, layer)?;
                } else {
                    let mut p = SetProperty::new(&id, &path, pm_scene3d::to_model(&value), time);
                    if autokey {
                        p.mode = KeyMode::Keyframe;
                    }
                    PropertyCommand::Set(p).plan(cx)?;
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
    for l in cx.layers(comp) {
        validate_layer(&l)?;
    }
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
    let def = layer.get("d")?.get("definition")?.as_str()?;
    let (prefix, key_name) = channel.split_once('.')?;
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
    out
}
pub(crate) fn validate_layer(layer: &Value) -> Result<(), CommandError> {
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
pub(crate) fn upgrade_project(project: Value) -> Value {
    let Ok(mut j) = pm_scene3d::from_model(&project) else {
        return project;
    };
    if pm_scene3d::migration::upgrade_project(&mut j).is_ok() {
        pm_scene3d::to_model(&j)
    } else {
        project
    }
}
#[allow(dead_code)]
fn _node_is_send(_: Node) {}
