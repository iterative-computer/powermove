//! Frozen media digests and metadata operations. Byte ownership stays with the
//! durable media host; metadata edits never unlink or collect an object.
use crate::{Error, Result, model};
use pm_kernel::{DocumentStore, Origin, WriteBody};
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Clone, Debug, Default)]
pub struct MediaState {
    pub available: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub cloud: bool,
    pub cloud_provider: Option<String>,
    pub peaks: Vec<f64>,
    pub audio_duration: Option<f64>,
    pub duration: Option<f64>,
    pub has_audio: bool,
}
pub trait MediaRegistry: Send + Sync {
    fn state(&self, project_id: &str, asset_id: &str) -> MediaState;
}
/// Whether a layer that uses this asset is locked, directly or through a parent.
pub fn has_locked_reference(p: &Value, asset: &str) -> bool {
    references(p, asset).iter().any(|r| r["locked"] == true)
}
/// The layer main adds for an imported asset. Models need a 3D definition instead.
pub fn layer_for(
    p: &Value,
    imported: &crate::services::ImportedMedia,
    at: f64,
) -> Option<(String, Value)> {
    let fps = p["fps"].as_f64().filter(|f| *f > 0.).unwrap_or(30.);
    let from = (at * fps).round() / fps;
    let span = (p["dur"].as_f64().unwrap_or(0.) - at).max(1. / fps);
    let (w, h) = (
        imported.width.or(p["w"].as_f64()),
        imported.height.or(p["h"].as_f64()),
    );
    let (kind, content) = match imported.kind.as_str() {
        "audio" => (
            "audio",
            json!({"asset":imported.asset,"trim":0,"gain":1,"fadeIn":0,"fadeOut":0}),
        ),
        "video" => (
            "video",
            json!({"asset":imported.asset,"trim":0,"speed":1,"embeddedAudio":imported.has_audio,"w":w,"h":h}),
        ),
        "image" => ("image", json!({"asset":imported.asset,"w":w,"h":h})),
        _ => return None,
    };
    let id = pm_kernel::new_id("layer");
    let command = json!({"type":"add_layer","id":id,"layerType":kind,"name":imported.name,
        "from":from,"duration":imported.duration.filter(|d| *d > 0.).unwrap_or(span),
        "content":content,"select":true});
    Some((id, command))
}
pub fn valid_folder<'a>(p: &'a Value, id: &'a Value) -> Option<&'a str> {
    id.as_str().filter(|s| p["assetFolders"][*s].is_object())
}
pub fn folders(p: &Value) -> Vec<Value> {
    p["assetFolders"].as_object().into_iter().flatten().filter(|(_, f)| f.is_object()).map(|(id,f)| {
        let parent = valid_folder(p, &f["parent"]).filter(|s| *s != id);
        json!({"id":id,"name":f["name"].as_str().filter(|s|!s.trim().is_empty()).unwrap_or("Untitled Folder"),"parent":parent})
    }).collect()
}
fn uses(v: &Value, asset: &str) -> bool {
    match v {
        Value::Object(o) => {
            v["asset"] == asset
                || v["assetId"] == asset
                || v["maps"]
                    .as_object()
                    .is_some_and(|m| m.values().any(|v| v == asset))
                || o.values().any(|v| uses(v, asset))
        }
        Value::Array(a) => a.iter().any(|v| uses(v, asset)),
        _ => false,
    }
}
fn locked(l: &Value, layers: &[Value], seen: &mut HashSet<String>) -> bool {
    let Some(id) = l["id"].as_str() else {
        return false;
    };
    if !seen.insert(id.into()) {
        return false;
    }
    l["lock"] == true
        || l["locked"] == true
        || ["group", "parent"].iter().any(|k| {
            layers
                .iter()
                .find(|other| other["id"] == l[*k] && l[*k].is_string())
                .is_some_and(|other| locked(other, layers, seen))
        })
}
pub fn references(p: &Value, asset: &str) -> Vec<Value> {
    fn walk(p: &Value, asset: &str, out: &mut Vec<Value>) {
        if let Some(layers) = p["layers"].as_array() {
            for l in layers.iter().filter(|l| uses(&l["d"], asset)) {
                out.push(json!({"compositionId":p["id"],"layerId":l["id"],"locked":locked(l,layers,&mut HashSet::new())}));
            }
        }
        if let Some(comps) = p["comps"].as_object() {
            for child in comps.values() {
                walk(child, asset, out);
            }
        }
    }
    let mut out = vec![];
    walk(p, asset, &mut out);
    out
}
pub fn list(p: &Value, args: &Value, registry: Option<&dyn MediaRegistry>) -> Value {
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let limit = args["limit"].as_u64().unwrap_or(100) as usize;
    let assets=crate::assets_in_order(p).into_iter().map(|meta| {
        let id=meta["id"].as_str().unwrap_or("");
        let refs=references(p,id);
        let runtime=registry.map(|r|r.state(p["id"].as_str().unwrap_or(""),id)).unwrap_or_default();
        let status=if runtime.loading {"loading"} else if runtime.error.is_some() {"failed"} else if runtime.available {"available"} else if runtime.cloud {"cloud"} else {"missing"};
        let mut row=json!({"folder":valid_folder(p,&meta["folder"]),"sourcePath":meta["sourcePath"].as_str().filter(|s|!s.is_empty()).or_else(||meta["path"].as_str().filter(|s|!s.is_empty())),"fingerprint":meta.get("fingerprint").filter(|v|!v.is_null()).cloned().unwrap_or(Value::Null),"persisted":meta["persisted"] == true,"status":status,"referenceCount":refs.len(),"lockedReferences":refs.iter().filter(|r|r["locked"] == true).count(),"references":refs.into_iter().take(50).collect::<Vec<_>>()});
        for (to,from) in [("id","id"),("name","name"),("kind","kind"),("size","size"),("width","w"),("height","h"),("duration","dur")] { if let Some(v)=meta.get(from) {row[to]=v.clone();} }
        if let Some(error)=runtime.error {row["error"]=json!(error);}
        row
    }).filter(|m| (!args["assetId"].is_string() || args["assetId"] == "" || args["assetId"] == m["id"]) && (args["missingOnly"] != true || ["missing","failed","cloud"].contains(&m["status"].as_str().unwrap_or("")))).collect::<Vec<_>>();
    json!({"assets":assets.iter().skip(offset).take(limit).collect::<Vec<_>>(),"total":assets.len(),"nextOffset":if offset.saturating_add(limit)<assets.len(){Some(offset+limit)}else{None},"folders":folders(p),"revision":p["revision"].as_u64().unwrap_or(0)})
}
/// Plan against a scratch document, returning id-keyed ops for one guarded
/// kernel transaction. Unknown metadata survives the diff unchanged.
pub fn body(before: &Value, after: &Value) -> Result<WriteBody> {
    let mut scratch = DocumentStore::new_untitled(model(before));
    let start = scratch.snapshot();
    scratch
        .restore_project(model(after), "MCP domain edit", Origin::Mcp)
        .map_err(Error::kernel)?;
    Ok(WriteBody::Ops(pm_model::ops::diff_node_ops(
        start.root(),
        scratch.snapshot().root(),
    )))
}
pub struct MediaPlan {
    pub body: WriteBody,
    pub result: Value,
    pub removed: Vec<String>,
}
fn title(a: &Value) -> Result<&str> {
    a["name"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.encode_utf16().count() <= 160)
        .map(str::trim)
        .ok_or_else(|| Error::invalid("Provide a name of 1–160 characters."))
}
fn unique_name(p: &Value, name: &str, parent: Option<&str>) -> String {
    let taken = folders(p)
        .into_iter()
        .filter(|f| f["parent"].as_str() == parent)
        .filter_map(|f| f["name"].as_str().map(str::to_lowercase))
        .collect::<HashSet<_>>();
    if !taken.contains(&name.to_lowercase()) {
        return name.into();
    }
    (2..)
        .map(|n| format!("{name} {n}"))
        .find(|s| !taken.contains(&s.to_lowercase()))
        .unwrap()
}
fn within(p: &Value, at: &str, ancestor: &str) -> bool {
    let mut current = Some(at);
    let mut seen = HashSet::new();
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        if !seen.insert(id) {
            break;
        }
        current = valid_folder(p, &p["assetFolders"][id]["parent"]);
    }
    false
}
fn remove_refs(p: &mut Value, asset: &str, removed: &mut Vec<String>) {
    if let Some(layers) = p["layers"].as_array_mut() {
        let doomed = layers
            .iter()
            .filter(|l| l["d"]["asset"] == asset)
            .filter_map(|l| l["id"].as_str().map(str::to_owned))
            .collect::<HashSet<_>>();
        removed.extend(
            layers
                .iter()
                .filter(|l| l["id"].as_str().is_some_and(|id| doomed.contains(id)))
                .filter_map(|l| l["id"].as_str().map(str::to_owned)),
        );
        layers.retain(|l| !l["id"].as_str().is_some_and(|id| doomed.contains(id)));
        for l in layers {
            if l["parent"].as_str().is_some_and(|id| doomed.contains(id)) {
                l["parent"] = Value::Null;
            }
        }
        // Preserve the tag's layer order in the returned removed IDs.
    }
    if let Some(comps) = p["comps"].as_object_mut() {
        for c in comps.values_mut() {
            remove_refs(c, asset, removed);
        }
    }
}
pub fn manage(p: &Value, a: &Value) -> Result<MediaPlan> {
    let op = a["operation"].as_str().unwrap_or("");
    let ids = a["assetIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if ids.iter().any(|id| !p["assets"][*id].is_object()) {
        return Err(Error::invalid(
            "Choose at most 100 existing assetIds from list_media.",
        ));
    }
    if ["rename", "move", "delete"].contains(&op) && ids.is_empty() {
        return Err(Error::invalid("Supply assetIds."));
    }
    if op == "rename" && ids.len() != 1 {
        return Err(Error::invalid("Rename one asset at a time."));
    }
    let folder = valid_folder(p, &a["folderId"]);
    if !a["folderId"].is_null() && folder.is_none() {
        return Err(Error::invalid("Choose an existing media folder."));
    }
    if ["rename_folder", "delete_folder", "move_folder"].contains(&op) && folder.is_none() {
        return Err(Error::invalid("Supply an existing folderId."));
    }
    let mut next = p.clone();
    let mut removed = vec![];
    let mut created = None;
    match op {
        "rename" => next["assets"][ids[0]]["name"] = json!(title(a)?),
        "move" => {
            for id in &ids {
                if valid_folder(p, &p["assets"][*id]["folder"]) != folder {
                    if let Some(f) = folder {
                        next["assets"][*id]["folder"] = json!(f)
                    } else {
                        next["assets"][*id]
                            .as_object_mut()
                            .unwrap()
                            .remove("folder");
                    }
                }
            }
        }
        "delete" => {
            for id in &ids {
                let refs = references(p, id);
                if refs.iter().any(|r| r["locked"] == true) {
                    return Err(Error::invalid(
                        "Unlock layers using this media before deleting it.",
                    ));
                }
                remove_refs(&mut next, id, &mut removed);
                if !references(&next, id).is_empty() {
                    return Err(Error::invalid(
                        "This media has structured references that cannot be safely removed. Remove those references before deleting it.",
                    ));
                }
                if !refs.is_empty() && a["removeReferenced"] != true {
                    return Err(Error::invalid(
                        "This media is used by layers. Set removeReferenced: true only when the user asked to remove those layers too.",
                    ));
                }
                next["assets"].as_object_mut().unwrap().remove(*id);
            }
        }
        "create_folder" => {
            let name = unique_name(p, title(a)?, folder);
            let id = pm_kernel::new_id("f");
            if !next["assetFolders"].is_object() {
                next["assetFolders"] = json!({});
            }
            next["assetFolders"][&id] = json!({"id":id,"name":name,"parent":folder});
            created = Some(id);
        }
        "rename_folder" => next["assetFolders"][folder.unwrap()]["name"] = json!(title(a)?),
        "move_folder" => {
            let id = folder.unwrap();
            let parent = valid_folder(p, &a["parentId"]);
            if !a["parentId"].is_null() && parent.is_none() {
                return Err(Error::invalid("Choose an existing parentId."));
            }
            if parent.is_some_and(|to| within(p, to, id)) {
                return Err(Error::invalid(
                    "A folder cannot move inside itself or a descendant.",
                ));
            }
            if valid_folder(p, &p["assetFolders"][id]["parent"]) != parent {
                let name = unique_name(
                    p,
                    p["assetFolders"][id]["name"]
                        .as_str()
                        .unwrap_or("Untitled Folder"),
                    parent,
                );
                next["assetFolders"][id]["parent"] = json!(parent);
                next["assetFolders"][id]["name"] = json!(name);
            }
        }
        "delete_folder" => {
            let id = folder.unwrap();
            let parent = valid_folder(p, &p["assetFolders"][id]["parent"]);
            if let Some(assets) = next["assets"].as_object_mut() {
                for asset in assets.values_mut().filter(|v| v["folder"] == id) {
                    if let Some(parent) = parent {
                        asset["folder"] = json!(parent)
                    } else if let Some(o) = asset.as_object_mut() {
                        o.remove("folder");
                    }
                }
            }
            for f in next["assetFolders"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .filter(|v| v["parent"] == id)
            {
                f["parent"] = json!(parent);
            }
            next["assetFolders"].as_object_mut().unwrap().remove(id);
        }
        _ => return Err(Error::invalid("Unknown media management operation.")),
    }
    let mut result = json!({"operation":op,"assetIds":ids,"removedLayerIds":removed});
    if let Some(id) = created {
        result["folderId"] = json!(id);
    }
    Ok(MediaPlan {
        body: body(p, &next)?,
        result,
        removed,
    })
}
