use crate::ValueMut;
use crate::{Catalog, Error, Stamp, object, text};
use pm_kernel::{
    Call, Caller, Command, CompCommand, DocumentStore, KernelHandle, Origin, Pending, Request,
};
use pm_model::{Map, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edit {
    CreateFolder {
        id: String,
        name: String,
        parent: Option<String>,
    },
    RenameFolder {
        id: String,
        name: String,
    },
    DeleteFolder {
        id: String,
    },
    Move {
        assets: Vec<String>,
        folders: Vec<String>,
        target: Option<String>,
    },
    DeleteAssets {
        ids: Vec<String>,
        confirmed: bool,
    },
    DuplicateComposition {
        id: String,
        new_id: String,
    },
    DeleteComposition {
        id: String,
    },
}

/// An immutable, byte-store-backed result from the import worker. The host
/// creates this only after original bytes and any retained old version are
/// durably archived. The model does no path probing or file deletion.
#[derive(Clone, Debug)]
pub struct StoredAsset {
    pub id: String,
    pub metadata: Value,
    pub original_version: String,
}

/// A reviewable one-step edit, bound to a document lifetime and revision. Uses
/// kernel Restore's atomic identity guard (Transact currently has only a revision
/// guard). It refuses during another open edit instead of racing project reopen.
#[derive(Clone, Debug)]
pub struct PreparedEdit {
    pub stamp: Stamp,
    pub label: String,
    pub(crate) next: Value,
    pub created: Option<String>,
    pub removed_layers: Vec<String>,
}
impl PreparedEdit {
    pub fn request(&self, caller: Caller) -> Request {
        Request::new(
            caller,
            Call::Restore {
                project: self.next.clone(),
                label: self.label.clone(),
                expected_revision: self.stamp.revision,
                expected_document: self.stamp.document,
            },
        )
    }
    /// Queue the guarded transaction; awaiting/polling is the caller's job.
    pub fn submit(
        &self,
        kernel: &KernelHandle,
        caller: Caller,
    ) -> Pending<Result<pm_kernel::Ack, pm_kernel::KernelFailure>> {
        kernel.call(&self.stamp.project, self.request(caller))
    }
}

impl Catalog {
    pub fn prepare(&self, edit: Edit) -> Result<PreparedEdit, Error> {
        let mut next = (*self.root).clone();
        let mut created = None;
        let mut removed_layers = vec![];
        let label = match edit {
            Edit::CreateFolder { id, name, parent } => {
                if id.is_empty()
                    || self.folders.contains_key(&id)
                    || parent.is_some() && self.valid_folder(parent.as_deref()) != parent
                {
                    return Err(Error::InvalidTarget);
                }
                let name = unique_name(&next, &name, parent.as_deref());
                let mut f = Map::new();
                f.insert("id", Value::from(id.as_str()));
                f.insert("name", Value::from(name.as_str()));
                f.insert("parent", optional(parent.as_deref()));
                ensure_map(&mut next, "assetFolders").insert(id.as_str(), Value::Object(f));
                created = Some(id);
                "New Folder"
            }
            Edit::RenameFolder { id, name } => {
                if !self.folders.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                let name = pm_model::json::js_trim(&name);
                if !name.is_empty() {
                    field_mut(&mut next, "assetFolders", &id)?
                        .as_object_mut()
                        .ok_or(Error::InvalidMedia)?
                        .insert("name", Value::from(name));
                }
                "Rename Folder"
            }
            Edit::DeleteFolder { id } => {
                let folder = self.folders.get(&id).ok_or(Error::MissingItem)?;
                let parent = folder.parent.as_deref();
                for (_, a) in ensure_map(&mut next, "assets").iter_mut() {
                    if text(a, "folder") == Some(&id) {
                        set_folder(a, "folder", parent)?;
                    }
                }
                for (_, f) in ensure_map(&mut next, "assetFolders").iter_mut() {
                    if text(f, "parent") == Some(&id) {
                        set_folder(f, "parent", parent)?;
                    }
                }
                ensure_map(&mut next, "assetFolders").remove(&id);
                "Delete Folder"
            }
            Edit::Move {
                assets,
                folders,
                target,
            } => {
                if target.is_some() && self.valid_folder(target.as_deref()) != target {
                    return Err(Error::InvalidTarget);
                }
                if assets.iter().any(|id| !self.assets.contains_key(id))
                    || folders.iter().any(|id| !self.folders.contains_key(id))
                {
                    return Err(Error::MissingItem);
                }
                if folders.iter().any(|id| self.within(target.as_deref(), id)) {
                    return Err(Error::FolderCycle);
                }
                for id in assets.iter().collect::<HashSet<_>>() {
                    set_folder(
                        field_mut(&mut next, "assets", id)?,
                        "folder",
                        target.as_deref(),
                    )?;
                }
                let mut seen = HashSet::new();
                for id in folders.into_iter().filter(|id| seen.insert(id.clone())) {
                    if self.folders[&id].parent == target {
                        continue;
                    }
                    let name = unique_name(&next, &self.folders[&id].name, target.as_deref());
                    let f = field_mut(&mut next, "assetFolders", &id)?;
                    set_folder(f, "parent", target.as_deref())?;
                    f.as_object_mut()
                        .unwrap()
                        .insert("name", Value::from(name.as_str()));
                }
                "Move to Folder"
            }
            Edit::DeleteAssets { ids, confirmed } => {
                if ids.iter().any(|id| !self.assets.contains_key(id)) {
                    return Err(Error::MissingItem);
                }
                if !confirmed && ids.iter().any(|id| self.assets[id].references > 0) {
                    return Err(Error::ConfirmationRequired);
                }
                for id in ids.iter().collect::<HashSet<_>>() {
                    remove_asset(&mut next, id, &mut removed_layers);
                    ensure_map(&mut next, "assets").remove(id);
                }
                "Delete Media"
            }
            Edit::DuplicateComposition { id, new_id } => {
                if !self.compositions.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                if new_id.is_empty() || self.compositions.contains_key(&new_id) {
                    return Err(Error::InvalidTarget);
                }
                next = self.command_next(Command::Comp(CompCommand::Duplicate {
                    comp_id: id,
                    new_id: new_id.clone(),
                }))?;
                created = Some(new_id);
                "Duplicate Composition"
            }
            Edit::DeleteComposition { id } => {
                if !self.compositions.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                next = self.command_next(Command::Comp(CompCommand::Delete {
                    comp_id: id,
                    fallback: None,
                }))?;
                "Delete Composition"
            }
        };
        Ok(PreparedEdit {
            stamp: self.stamp.clone(),
            label: label.into(),
            next,
            created,
            removed_layers,
        })
    }
    fn command_next(&self, command: Command) -> Result<Value, Error> {
        let mut scratch = DocumentStore::new_untitled((*self.root).clone());
        scratch
            .execute(command, Origin::Interface)
            .map_err(|e| Error::Kernel(e.to_string()))?;
        scratch
            .snapshot()
            .document
            .to_value()
            .ok_or(Error::InvalidMedia)
    }
    /// Worker completion is document-bound. A changed bin or revision refuses
    /// atomically so the host can review/replan; archived bytes remain retained.
    pub fn prepare_import(
        &self,
        owner: &Stamp,
        items: Vec<StoredAsset>,
        folder: Option<String>,
        replace: Option<String>,
    ) -> Result<PreparedEdit, Error> {
        if owner != &self.stamp {
            return Err(Error::Stale);
        }
        if folder.is_some() && self.valid_folder(folder.as_deref()) != folder {
            return Err(Error::MissingItem);
        }
        if items.is_empty() || replace.is_some() && items.len() != 1 {
            return Err(Error::InvalidMedia);
        }
        if replace
            .as_ref()
            .is_some_and(|id| !self.assets.contains_key(id))
        {
            return Err(Error::MissingItem);
        }
        let mut next = (*self.root).clone();
        let mut seen = HashSet::new();
        for item in items {
            let id = replace.clone().unwrap_or(item.id);
            if id.is_empty()
                || !seen.insert(id.clone())
                || replace.is_none() && self.assets.contains_key(&id)
                || item.original_version.is_empty()
            {
                return Err(Error::InvalidMedia);
            }
            if item.metadata.as_object().is_none()
                || text(&item.metadata, "kind").is_none()
                || text(&item.metadata, "name").is_none()
            {
                return Err(Error::InvalidMedia);
            }
            let mut meta = if replace.is_some() {
                field_mut(&mut next, "assets", &id)?.clone()
            } else {
                Value::Object(Map::new())
            };
            // Merge verified metadata without losing unknown extension fields.
            for (k, v) in item.metadata.as_object().unwrap().iter() {
                if replace.is_some() && k.as_str() == Some("folder") {
                    continue;
                }
                meta.as_object_mut().unwrap().insert(k.clone(), v.clone());
            }
            let m = meta.as_object_mut().unwrap();
            m.insert("id", Value::from(id.as_str()));
            m.insert(
                "originalVersion",
                Value::from(item.original_version.as_str()),
            );
            if replace.is_none() {
                set_folder(&mut meta, "folder", folder.as_deref())?;
            }
            // Replacing an asset keeps its id: layer timing/trim/transforms and
            // source-time interpretation remain authored exactly as before.
            ensure_map(&mut next, "assets").insert(id.as_str(), meta);
        }
        Ok(PreparedEdit {
            stamp: self.stamp.clone(),
            label: if replace.is_some() {
                "Replace Media"
            } else {
                "Import"
            }
            .into(),
            next,
            created: None,
            removed_layers: vec![],
        })
    }
}

fn optional(s: Option<&str>) -> Value {
    s.map(Value::from).unwrap_or(Value::Null)
}
fn ensure_map<'a>(v: &'a mut Value, key: &str) -> &'a mut Map {
    let m = v.as_object_mut().expect("project object");
    if m.get(key).and_then(Value::as_object).is_none() {
        m.insert(key, Value::Object(Map::new()));
    }
    m.get_mut(key).unwrap().as_object_mut().unwrap()
}
fn field_mut<'a>(root: &'a mut Value, key: &str, id: &str) -> Result<&'a mut Value, Error> {
    root.get_mut(key)
        .and_then(|m| m.get_mut(id))
        .ok_or(Error::MissingItem)
}
fn set_folder(v: &mut Value, key: &str, id: Option<&str>) -> Result<(), Error> {
    let m = v.as_object_mut().ok_or(Error::InvalidMedia)?;
    if key == "parent" || id.is_some() {
        m.insert(key, optional(id));
    } else {
        m.remove(key);
    }
    Ok(())
}
fn unique_name(root: &Value, name: &str, parent: Option<&str>) -> String {
    let base = pm_model::json::js_trim(name);
    let base = if base.is_empty() {
        "Untitled Folder"
    } else {
        base
    };
    let taken: HashSet<_> = object(root, "assetFolders")
        .into_iter()
        .filter(|(_, f)| text(f, "parent") == parent)
        .filter_map(|(_, f)| text(f, "name").map(str::to_lowercase))
        .collect();
    if !taken.contains(&base.to_lowercase()) {
        return base.into();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|s| !taken.contains(&s.to_lowercase()))
        .unwrap()
}

/// Tagged media.ts:195–244, including extension object sources, scene object
/// descendants and texture maps. No byte-store operation is available here.
fn remove_asset(root: &mut Value, id: &str, removed: &mut Vec<String>) {
    fn strip_maps(o: &mut Value, id: &str) {
        if let Some(maps) = o
            .get_mut("material")
            .and_then(|m| m.get_mut("maps"))
            .and_then(Value::as_object_mut)
        {
            let keys: Vec<_> = maps
                .iter()
                .filter(|(_, v)| v.as_str() == Some(id))
                .map(|(k, _)| k.to_owned())
                .collect();
            for key in keys {
                maps.remove_key(&key);
            }
        }
    }
    fn visit(comp: &mut Value, id: &str, removed: &mut Vec<String>) {
        let Some(layers) = comp.get_mut("layers").and_then(Value::as_array_mut) else {
            return;
        };
        for l in layers.iter_mut() {
            if let Some(data) = l.get_mut("d").and_then(|d| d.get_mut("data")) {
                if let Some(o) = data.get_mut("object") {
                    strip_maps(o, id);
                }
                if let Some(objects) = data
                    .get_mut("scene")
                    .and_then(|s| s.get_mut("objects"))
                    .and_then(Value::as_array_mut)
                {
                    let mut doomed: HashSet<String> = objects
                        .iter()
                        .filter(|o| o.get("source").and_then(|s| text(s, "assetId")) == Some(id))
                        .filter_map(|o| text(o, "id").map(str::to_owned))
                        .collect();
                    loop {
                        let n = doomed.len();
                        for o in objects.iter() {
                            if text(o, "parent").is_some_and(|id| doomed.contains(id))
                                && let Some(id) = text(o, "id")
                            {
                                doomed.insert(id.into());
                            }
                        }
                        if n == doomed.len() {
                            break;
                        }
                    }
                    objects.retain(|o| !text(o, "id").is_some_and(|id| doomed.contains(id)));
                    for o in objects {
                        strip_maps(o, id);
                    }
                }
            }
        }
        layers.retain(|l| {
            if crate::catalog::layer_asset(l) == Some(id) {
                if let Some(id) = text(l, "id") {
                    removed.push(id.into());
                }
                false
            } else {
                true
            }
        });
        let doomed: HashSet<_> = removed.iter().collect();
        for l in layers {
            if text(l, "parent").is_some_and(|id| doomed.contains(&id.to_owned())) {
                l.as_object_mut().unwrap().insert("parent", Value::Null);
            }
        }
    }
    visit(root, id, removed);
    if let Some(comps) = root.get_mut("comps").and_then(Value::as_object_mut) {
        for (_, c) in comps.iter_mut() {
            visit(c, id, removed);
        }
    }
}
