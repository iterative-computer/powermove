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
                if current != pm_scene3d::to_model(&import_metadata(asset_id, receipt)) {
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
    let mut json = pm_scene3d::from_model(&cx.comp_node(comp)?.to_value()).map_err(invalid)?;
    let args = pm_scene3d::from_model(args).map_err(invalid)?;
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
pub(crate) fn upgrade_project(project: Value) -> Value {
    let mut j = match pm_scene3d::from_model(&project) {
        Ok(j) => j,
        Err(error) => return retain_unrepresentable_3d(project, &error.to_string()),
    };
    let canonical = crate::model::to_canonical(project.clone());
    let doc = pm_model::Document::new(Some(canonical), 1);
    let mut sampler = LegacySampler {
        queries: crate::queries::QueryContext::new(&doc),
    };
    if pm_scene3d::migration::upgrade_project_with(&mut j, &mut sampler).is_ok() {
        pm_scene3d::to_model(&j)
    } else {
        project
    }
}
// PM documents preserve lone UTF-16 surrogates, while the scene schema's
// JSON bridge cannot represent them. Keep every original code unit and mark
// only known 3D layers as failed, instead of accidentally leaving a legacy
// renderer definition active. Never traverse archived source records.
fn retain_unrepresentable_3d(mut project: Value, diagnostic: &str) -> Value {
    fn comp(value: &mut Value, diagnostic: &str) {
        let Some(map) = value.as_object_mut() else {
            return;
        };
        if let Some(layers) = map.get_mut("layers").and_then(Value::as_array_mut) {
            for layer in layers {
                let known = layer
                    .get("d")
                    .and_then(|d| d.get("definition"))
                    .and_then(Value::as_str)
                    .is_some_and(|s| {
                        matches!(
                            s,
                            schema::SCENE
                                | schema::OBJECT
                                | schema::LIGHT
                                | schema::CAMERA
                                | "powermove.3d.obj-model"
                        )
                    });
                if !known {
                    continue;
                }
                let original = layer.clone();
                let m = layer.as_object_mut().expect("known layer object");
                if m.get("native3dArchive").is_none() {
                    let mut archive = pm_model::Map::new();
                    archive.insert("version", Value::Number(1.));
                    archive.insert("source", original);
                    m.insert("native3dArchive", Value::Object(archive));
                }
                m.get_mut("d")
                    .and_then(Value::as_object_mut)
                    .unwrap()
                    .insert("definition", Value::from("powermove.3d.unupgraded"));
                m.insert("native3dUpgradeDiagnostic", Value::from(diagnostic));
                // DESIGN-TODO(opus): same unknown-reason placeholder as migration::mark_failed.
                m.insert(
                    "native3dUpgradeIssue",
                    Value::from("This 3D layer couldn’t be upgraded."),
                );
            }
        }
        if let Some(comps) = map.get_mut("comps").and_then(Value::as_object_mut) {
            for (_, value) in comps.iter_mut() {
                comp(value, diagnostic);
            }
        }
    }
    comp(&mut project, diagnostic);
    project
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

/// Upgrade each imported history state as well as the current document. Stable
/// deterministic migration IDs make before/after diffs preserve Undo and Redo.
/// A complete original logical history remains archived for recovery.
pub(crate) fn upgrade_document_history(
    document: pm_model::Document,
    history: crate::History,
) -> (pm_model::Document, crate::History) {
    fn contains(v: &Value) -> bool {
        match v {
            Value::Object(m) => {
                m.get("definition")
                    .and_then(Value::as_str)
                    .is_some_and(|s| {
                        matches!(
                            s,
                            schema::SCENE
                                | schema::OBJECT
                                | schema::LIGHT
                                | schema::CAMERA
                                | "powermove.3d.obj-model"
                        )
                    })
                    || m.iter().any(|(_, v)| contains(v))
            }
            Value::Array(a) => a.iter().any(contains),
            _ => false,
        }
    }
    let Some(root) = document.root() else {
        return (document, history);
    };
    let current = root.to_value();
    let relevant = contains(&current)
        || history
            .records()
            .iter()
            .flat_map(|r| r.forward.iter().chain(r.inverse.iter()))
            .any(|op| matches!(op,pm_model::Op::Leaf(p) if p.value.as_ref().is_some_and(contains)));
    if !relevant {
        return (document, history);
    }
    let upgraded = upgrade_project(current.clone());
    if current
        .get("native3dHistoryVersion")
        .and_then(Value::as_f64)
        == Some(1.)
    {
        return (
            pm_model::Document::new(Some(upgraded), document.revision()),
            history,
        );
    }
    if history.is_empty() {
        return (
            pm_model::Document::new(Some(upgraded), document.revision()),
            history,
        );
    }
    let saved = history.snapshot().to_saved();
    let mut original = document.clone();
    let cursor = history.applied();
    let mut revision = original.revision();
    let mut needed = upgraded != current;
    let run = (|| -> Result<crate::History, String> {
        for record in history.records()[..cursor].iter().rev() {
            revision += 1;
            original
                .apply_ops_at(&record.inverse, revision)
                .map_err(|e| e.to_string())?;
        }
        let prior = original
            .root()
            .ok_or("History removed the document")?
            .to_value();
        let mut before = upgrade_project(prior.clone());
        needed |= before != prior;
        let mut next = crate::History::with_limits(history.len().max(1), usize::MAX);
        for record in history.records() {
            if !record.is_document() {
                return Err("Imported 3D history contains a non-document entry".into());
            }
            revision += 1;
            original
                .apply_ops_at(&record.forward, revision)
                .map_err(|e| e.to_string())?;
            let prior = original
                .root()
                .ok_or("History removed the document")?
                .to_value();
            let after = upgrade_project(prior.clone());
            needed |= after != prior;
            let (forward, inverse) = pm_model::patch::diff_patches(Some(&before), Some(&after));
            next.push(
                record.label.clone(),
                record.origin,
                forward.into_iter().map(pm_model::Op::Leaf).collect(),
                inverse.into_iter().map(pm_model::Op::Leaf).collect(),
                None,
                0,
            );
            before = after;
        }
        while next.applied() > cursor {
            next.mark_undone();
        }
        next.set_max_bytes(history.stats().max_bytes.max(next.stats().bytes));
        Ok(next)
    })();
    if run.is_ok() && !needed {
        return (document, history);
    }
    let mut value = upgraded;
    if let Some(m) = value.as_object_mut() {
        if m.get("native3dHistoryArchive").is_none() {
            m.insert("native3dHistoryArchive", saved);
        }
        m.insert("native3dHistoryVersion", Value::Number(1.));
    }
    match run {
        Ok(next) => (
            pm_model::Document::new(Some(value), document.revision()),
            next,
        ),
        Err(reason) => {
            if let Some(m) = value.as_object_mut() {
                m.insert("native3dHistoryIssue", Value::from(reason));
            }
            (
                pm_model::Document::new(Some(value), document.revision()),
                crate::History::new(),
            )
        }
    }
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
    let json = pm_scene3d::from_model(&comp_node.to_value()).map_err(invalid)?;
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
    let records = pm_scene3d::from_model(
        &root
            .get("assets")
            .map(pm_model::Node::to_value)
            .unwrap_or(Value::Null),
    )
    .map_err(invalid)?;
    for id in ids {
        let loc =
            crate::model::find_layer(root, &id).ok_or_else(|| invalid("3D layer not found"))?;
        let layer =
            crate::model::layer_at(root, &loc).ok_or_else(|| invalid("3D layer not found"))?;
        let layer = pm_scene3d::from_model(&layer.to_value()).map_err(invalid)?;
        let request = |op: &str| {
            pm_scene3d::to_model(
                &serde_json::json!({"op":op,"comp":comp_id,"open":comp_id,"layer":loc.index,"t":time}),
            )
        };
        if queries.query(&request("active")).map_err(invalid)? != Value::Bool(true) {
            continue;
        }
        let local = pm_scene3d::geometry::local_bounds(
            &layer["d"]["data"]["object"]["source"],
            assets,
            &records,
            time - layer["from"].as_f64().unwrap_or(0.),
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
    let empty = cx.layers(comp).iter().all(|l| {
        !matches!(
            l.get("d")
                .and_then(|d| d.get("definition"))
                .and_then(Value::as_str),
            Some(schema::LIGHT | schema::CAMERA)
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
    if empty && setup_studio {
        let mut assets = pm_scene3d::geometry::LoadedAssets::default();
        assets.insert_shared(asset_id.into(), metadata, receipt.model());
        for operation in [
            serde_json::json!({"operation":"drop_to_floor","target":layer_id}),
            serde_json::json!({"operation":"auto_studio"}),
        ] {
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
    serde_json::json!({"id":id,"name":b.name(),"kind":"model","format":b.name().rsplit('.').next().unwrap_or(""),"size":b.length(),"persisted":true,"revision":b.hash(),"native3dSources":sources})
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
    struct Evaluator {
        queries: crate::queries::QueryContext,
        comp: String,
        layers: Vec<serde_json::Value>,
    }
    impl Evaluator {
        fn query(
            &mut self,
            op: &str,
            l: &serde_json::Value,
            t: f64,
            extra: serde_json::Value,
        ) -> pm_scene3d::Result<serde_json::Value> {
            let i = self
                .layers
                .iter()
                .position(|n| n["id"] == l["id"])
                .ok_or_else(|| pm_scene3d::Error("3D layer moved".into()))?;
            let mut request =
                serde_json::json!({"op":op,"comp":self.comp,"open":self.comp,"layer":i,"t":t});
            for (k, v) in extra.as_object().unwrap() {
                request[k] = v.clone();
            }
            let value = self
                .queries
                .query(&pm_scene3d::to_model(&request))
                .map_err(|e| pm_scene3d::Error(e.to_string()))?;
            pm_scene3d::from_model(&value)
        }
    }
    impl pm_scene3d::service::Evaluator for Evaluator {
        fn value(
            &mut self,
            l: &serde_json::Value,
            path: &str,
            c: &serde_json::Value,
            t: f64,
        ) -> pm_scene3d::Result<serde_json::Value> {
            self.query("evP", l, t, serde_json::json!({"key":path,"prop":c}))
        }
        fn active(&mut self, l: &serde_json::Value, t: f64) -> pm_scene3d::Result<bool> {
            self.query("active", l, t, serde_json::json!({}))
                .map(|v| v == true)
        }
    }
    impl pm_scene3d::service::WorldEvaluator for Evaluator {
        fn world(&mut self, l: &serde_json::Value, t: f64) -> pm_scene3d::Result<[f64; 16]> {
            pm_scene3d::geometry::matrix(&self.query("world3D", l, t, serde_json::json!({}))?)
        }
        fn parent(&mut self, l: &serde_json::Value, t: f64) -> pm_scene3d::Result<[f64; 16]> {
            pm_scene3d::geometry::matrix(&self.query("parent3D", l, t, serde_json::json!({}))?)
        }
        fn opacity(&mut self, l: &serde_json::Value, t: f64) -> pm_scene3d::Result<f64> {
            self.query("worldOpacity", l, t, serde_json::json!({}))?
                .as_f64()
                .ok_or_else(|| pm_scene3d::Error("Invalid opacity".into()))
        }
    }
    let root = snapshot.root().ok_or_else(|| invalid("No project"))?;
    let comp = comp.map(str::to_owned).or_else(|| snapshot.open_comp());
    let c =
        crate::model::comp_node(root, comp.as_deref()).ok_or_else(|| invalid("No composition"))?;
    let c = pm_scene3d::from_model(&c.to_value()).map_err(invalid)?;
    let comp = comp.unwrap_or_else(|| {
        format!(
            "@root:{}",
            root.get("id").and_then(crate::model::str_of).unwrap_or("")
        )
    });
    let layers = c["layers"]
        .as_array()
        .ok_or_else(|| invalid("No layers"))?
        .clone();
    pm_scene3d::service::resolve(
        &c,
        time,
        &mut Evaluator {
            queries: crate::queries::QueryContext::new(&snapshot.document),
            comp,
            layers,
        },
    )
    .map_err(invalid)
}
