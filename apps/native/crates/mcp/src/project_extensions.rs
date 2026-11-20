//! Inert extension packages in a PMV envelope. Parsing a package only returns
//! bytes and a report. The app owns the consent prompt and installation.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use pm_model::{Node, NodeKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::extensions::{extension_id, project_tree};

const MANIFEST: &str = "extensions/manifest";
const FILES: &str = "extensions/files";
pub const STORE_MARKER: &str = ".powermove-store.json";
const MAX_PACKAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_BUNDLE_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILES: usize = 4_000;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Usage {
    pub composition: String,
    pub layer: String,
    pub kind: String,
    pub definition: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub source: String,
    pub store_id: Option<String>,
    pub hash: Option<String>,
    pub bytes: usize,
    pub items: Vec<Usage>,
    pub status: Status,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Included,
    StoreOnly,
    Unavailable,
    Excluded,
    Invalid,
}

#[derive(Clone, Debug, Default)]
pub struct Bundle {
    pub entries: Vec<Entry>,
    packages: BTreeMap<String, Files>,
}

type Files = BTreeMap<String, Vec<u8>>;

fn text<'a>(node: &'a Node, key: &str) -> Option<&'a str> {
    match node.get(key)?.kind() {
        NodeKind::String(s) => s.as_str(),
        _ => None,
    }
}

/// The definition lookups the renderer makes. Inline shader source lives in
/// the layer itself; GLSL resources used by a registered definition travel
/// with its provider's files.
pub fn references(root: &Node) -> BTreeSet<Usage> {
    let mut pending = vec![(
        root,
        text(root, "compId")
            .or_else(|| text(root, "id"))
            .unwrap_or(""),
    )];
    let mut items = BTreeSet::new();
    while let Some((comp, id)) = pending.pop() {
        if let Some(NodeKind::Object(children)) = comp.get("comps").map(Node::kind) {
            pending.extend(
                children
                    .iter()
                    .filter_map(|(id, node)| Some((node, id.as_str()?))),
            );
        }
        let Some(NodeKind::Array(layers)) = comp.get("layers").map(Node::kind) else {
            continue;
        };
        for layer in layers.iter() {
            let mut record = |kind: &str, definition: Option<&str>| {
                if let Some(definition) = definition.filter(|id| !id.is_empty()) {
                    items.insert(Usage {
                        composition: id.to_owned(),
                        layer: text(layer, "id").unwrap_or("").to_owned(),
                        kind: kind.to_owned(),
                        definition: definition.to_owned(),
                    });
                }
            };
            if let Some(NodeKind::Array(effects)) = layer.get("fx").map(Node::kind) {
                for effect in effects.iter() {
                    record("effect", text(effect, "type"));
                }
            }
            for key in ["transitionIn", "transitionOut"] {
                record(
                    "transition",
                    layer.get(key).and_then(|node| text(node, "type")),
                );
            }
            if text(layer, "type") == Some("extension") {
                record(
                    "layer",
                    layer.get("d").and_then(|node| text(node, "definition")),
                );
            }
        }
    }
    items
}

#[derive(Clone, Debug)]
pub struct Installed {
    pub directory: PathBuf,
    pub manifest: Option<Value>,
    pub store: Option<Value>,
}

/// Later roots win, as in the extension host. Reading never loads a realm.
pub fn installed(roots: &[PathBuf]) -> BTreeMap<String, Installed> {
    roots
        .iter()
        .filter_map(|root| fs::read_dir(root).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let id = entry.file_name().into_string().ok()?;
            if !extension_id(&id) || !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let directory = entry.path();
            if !fs::symlink_metadata(directory.join("manifest.json"))
                .ok()?
                .is_file()
            {
                return None;
            }
            let manifest = read_json(&directory.join("manifest.json"))
                .filter(|manifest| manifest["id"] == id && manifest["version"].is_string());
            let store = fs::symlink_metadata(directory.join(STORE_MARKER))
                .ok()
                .filter(|meta| meta.is_file())
                .map(|_| read_json(&directory.join(STORE_MARKER)).unwrap_or_default());
            Some((
                id,
                Installed {
                    directory,
                    manifest,
                    store,
                },
            ))
        })
        .collect()
}

fn read_json(path: &Path) -> Option<Value> {
    if fs::symlink_metadata(path).ok()?.len() > 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

impl Bundle {
    /// Provider ids from the active extension host beat the namespaced
    /// convention. Built-ins are excluded before matching possible providers.
    pub fn collect(
        items: BTreeSet<Usage>,
        installed: &BTreeMap<String, Installed>,
        owners: &BTreeMap<(String, String), String>,
        builtins: &BTreeSet<(String, String)>,
        unavailable: &BTreeSet<String>,
        include: bool,
    ) -> Self {
        let mut needed = BTreeMap::<String, BTreeSet<Usage>>::new();
        for item in items {
            let key = (item.kind.clone(), item.definition.clone());
            if builtins.contains(&key) {
                continue;
            }
            let name = item.definition.split_once('.').map(|(left, _)| left);
            let owner = owners
                .get(&key)
                .map(String::as_str)
                .or_else(|| {
                    installed
                        .contains_key(&item.definition)
                        .then_some(item.definition.as_str())
                })
                .or(name)
                .or(Some(item.definition.as_str()));
            if let Some(owner) = owner.filter(|owner| extension_id(owner)) {
                needed.entry(owner.to_owned()).or_default().insert(item);
            }
        }
        let mut pending: Vec<_> = needed
            .iter()
            .flat_map(|(id, items)| items.iter().map(|item| (id.clone(), item.clone())))
            .collect();
        while let Some((id, item)) = pending.pop() {
            let dependencies = installed
                .get(&id)
                .and_then(|entry| entry.manifest.as_ref())
                .and_then(|manifest| manifest["dependsOn"].as_array());
            for dependency in dependencies.into_iter().flatten().filter_map(Value::as_str) {
                // Built-ins arrive from the receiver's own app, even when a
                // provider uses them through the compatibility loader.
                if !extension_id(dependency) || builtin_dependency(dependency, builtins) {
                    continue;
                }
                let users = needed.entry(dependency.to_owned()).or_default();
                if users.insert(item.clone()) {
                    pending.push((dependency.to_owned(), item.clone()));
                }
            }
        }
        let mut bundle = Self::default();
        let mut total = 0;
        for (id, items) in needed {
            let found = installed.get(&id);
            let manifest = found.and_then(|found| found.manifest.as_ref());
            let field = |key: &str| {
                manifest
                    .and_then(|m| m[key].as_str())
                    .unwrap_or("")
                    .to_owned()
            };
            let store = found.and_then(|found| found.store.as_ref());
            let mut entry = Entry {
                name: manifest
                    .and_then(|m| m["name"].as_str())
                    .unwrap_or(&id)
                    .to_owned(),
                version: field("version"),
                author: field("author"),
                source: if store.is_some() { "store" } else { "local" }.into(),
                store_id: store.map(|m| {
                    m["storeId"]
                        .as_str()
                        .or_else(|| m["repoId"].as_str())
                        .or_else(|| m["id"].as_str())
                        .unwrap_or(&id)
                        .to_owned()
                }),
                id: id.clone(),
                hash: None,
                bytes: 0,
                items: items.into_iter().collect(),
                status: Status::Unavailable,
            };
            if store.is_some_and(|m| m["redistributable"] != true) {
                entry.status = Status::StoreOnly;
            } else if let Some(found) =
                found.filter(|_| manifest.is_some() && !unavailable.contains(&id))
            {
                if let Ok(files) = project_tree(&found.directory) {
                    let bytes: usize = files.values().map(Vec::len).sum();
                    if valid_package(&entry, &files) && total + bytes <= MAX_BUNDLE_BYTES {
                        entry.bytes = bytes;
                        if include {
                            entry.hash = Some(package_hash(&files));
                            entry.status = Status::Included;
                            bundle.packages.insert(id, files);
                            total += bytes;
                        } else {
                            entry.status = Status::Excluded;
                        }
                    }
                }
            }
            bundle.entries.push(entry);
        }
        bundle
    }

    pub fn exclusions(&self, library: &BTreeMap<String, Installed>) -> BTreeSet<String> {
        self.entries
            .iter()
            .filter(|entry| {
                entry.status == Status::Invalid
                    || (!entry.version.is_empty() && !available_locally(entry, library))
            })
            .map(|entry| entry.id.clone())
            .collect()
    }

    fn encoded_parts(&self) -> (Option<Value>, Option<Value>) {
        let packages: BTreeMap<_, BTreeMap<_, _>> = self
            .packages
            .iter()
            .map(|(id, files)| {
                (
                    id,
                    files
                        .iter()
                        .map(|(path, data)| (path, STANDARD.encode(data)))
                        .collect(),
                )
            })
            .collect();
        (
            (!self.entries.is_empty()).then(|| json!({"version":1,"entries":self.entries})),
            (!packages.is_empty()).then(|| json!(packages)),
        )
    }

    pub fn add_to(&self, plan: &mut pm_kernel::SavePlan) -> Result<(), String> {
        let (manifest, files) = self.encoded_parts();
        plan.set_export_extension_bundle(
            manifest.map(model_value).transpose()?,
            files.map(model_value).transpose()?,
        );
        Ok(())
    }

    /// A pending export keeps the exact bundle selected at enqueue time.
    pub fn queued(&self) -> Value {
        let (manifest, files) = self.encoded_parts();
        match manifest {
            Some(manifest) => json!({MANIFEST: manifest, FILES: files}),
            None => Value::Null,
        }
    }

    pub fn from_queue(value: &Value) -> Result<Self, String> {
        if value.is_null() {
            return Ok(Self::default());
        }
        let value = model_value(value.clone())?;
        Self::read(value.as_object().ok_or("Invalid queued extension bundle.")?)
    }

    pub fn read(extra: &pm_model::json::Map) -> Result<Self, String> {
        let Some(manifest) = extra.get(MANIFEST) else {
            return Ok(Self::default());
        };
        let manifest = wire(manifest)?;
        if manifest["version"] != 1 {
            return Err("Unsupported project extension manifest.".into());
        }
        let mut entries: Vec<Entry> =
            serde_json::from_value(manifest["entries"].clone()).map_err(|e| e.to_string())?;
        if entries.len() > MAX_FILES {
            return Err("Too many extensions in the project manifest.".into());
        }
        let raw = extra.get(FILES);
        let mut packages = BTreeMap::new();
        let mut seen = BTreeSet::new();
        let mut total = 0;
        for entry in &mut entries {
            if !extension_id(&entry.id) || !seen.insert(entry.id.clone()) {
                return Err("Invalid extension id in the project manifest.".into());
            }
            if entry.status != Status::Included {
                continue;
            }
            let verified = raw
                .and_then(|raw| raw.get(&entry.id))
                .and_then(pm_model::Value::as_object)
                .and_then(|encoded| {
                    if encoded.len() > MAX_FILES {
                        return None;
                    }
                    let mut files = BTreeMap::new();
                    let mut bytes = 0;
                    for (path, data) in encoded.iter() {
                        let path = path.as_str()?;
                        let encoded = data.as_str()?;
                        if !valid_path(path)
                            || encoded.len() > (MAX_PACKAGE_BYTES - bytes).div_ceil(3) * 4
                        {
                            return None;
                        }
                        let data = STANDARD.decode(encoded).ok()?;
                        bytes += data.len();
                        if bytes > MAX_PACKAGE_BYTES || total + bytes > MAX_BUNDLE_BYTES {
                            return None;
                        }
                        files.insert(path.to_owned(), data);
                    }
                    (bytes == entry.bytes
                        && valid_package(entry, &files)
                        && entry.hash.as_deref() == Some(package_hash(&files).as_str()))
                    .then_some((files, bytes))
                });
            if let Some((files, bytes)) = verified {
                // Consent names the verified manifest, including its author.
                if let Some(manifest) = files
                    .get("manifest.json")
                    .and_then(|raw| serde_json::from_slice::<Value>(raw).ok())
                {
                    entry.name = manifest["name"].as_str().unwrap_or(&entry.id).to_owned();
                    entry.author = manifest["author"].as_str().unwrap_or("").to_owned();
                }
                total += bytes;
                packages.insert(entry.id.clone(), files);
            } else {
                entry.status = Status::Invalid;
            }
        }
        Ok(Self { entries, packages })
    }

    pub fn needs_install(&self, entry: &Entry, library: &BTreeMap<String, Installed>) -> bool {
        entry.status == Status::Included && !available_locally(entry, library)
    }

    pub fn report(&self, library: &BTreeMap<String, Installed>) -> Value {
        let missing: Vec<_> = self
            .entries
            .iter()
            .filter(|item| item.status == Status::Invalid || !available_locally(item, library))
            .collect();
        if self.entries.is_empty() {
            return json!({});
        }
        json!({
            "requiredExtensions": self.entries,
            "missingExtensions": missing,
            "extensionConsentRequired": missing.iter().any(|e| e.status == Status::Included),
            "extensionNotice": (!missing.is_empty()).then_some(
                "Tell the person which extensions are missing. To install bundled extensions, ask them to open the project in Powermove and choose an installation option."
            ),
        })
    }

    /// Call only after the person accepts. Re-exposes only verified bytes.
    pub fn files(&self, id: &str) -> Option<&BTreeMap<String, Vec<u8>>> {
        self.packages.get(id)
    }
}

pub fn library_roots(profile: &Path) -> Vec<PathBuf> {
    vec![
        profile.join("extensions-shared"),
        profile.join("extensions"),
    ]
}

/// Provider metadata already collected by the Library hosts.
pub fn catalog(
    state: &pm_kernel::state::StateSnapshot,
) -> (BTreeMap<(String, String), String>, BTreeSet<String>) {
    let mut owners = BTreeMap::new();
    let mut unavailable = BTreeSet::new();
    for (id, entry) in state.extensions.iter() {
        let record = entry.source.get("hostRecord");
        if !entry.enabled
            || entry.stopped_this_run()
            || record
                .and_then(|r| r.get("health"))
                .and_then(|h| h.get("state"))
                .and_then(pm_model::Value::as_str)
                .is_some_and(|health| health != "ok")
        {
            unavailable.insert(id.clone());
        }
        if let Some(ids) = record
            .and_then(|r| r.get("provides"))
            .and_then(pm_model::Value::as_array)
        {
            for definition in ids.iter().filter_map(pm_model::Value::as_str) {
                for kind in ["effect", "transition", "layer"] {
                    owners.insert((kind.into(), definition.into()), id.to_owned());
                }
            }
        }
    }
    (owners, unavailable)
}

pub fn collect_from_library(
    root: &Node,
    state: &pm_kernel::state::StateSnapshot,
    profile: &Path,
    scope: &crate::app_host::ProjectExtensions,
) -> Bundle {
    let (mut owners, mut unavailable) = catalog(state);
    unavailable.extend(scope.excluded.iter().cloned());
    let mut roots = library_roots(profile);
    roots.extend(scope.root.iter().cloned());
    let library = installed(&roots);
    for (kind, registry) in [
        ("effect", "effects"),
        ("transition", "transitions"),
        ("layer", "layerTypes"),
    ] {
        for entry in state.registries.get(registry).into_iter().flatten() {
            let stored = entry
                .definition
                .get("owner")
                .and_then(pm_model::Value::as_str);
            let namespace = entry.id.split_once('.').map(|(id, _)| id);
            if let Some(owner) = stored.or(namespace).filter(|id| library.contains_key(*id)) {
                owners.insert((kind.into(), entry.id.clone()), owner.to_owned());
                unavailable.remove(owner);
            }
        }
    }
    let refs = references(root);
    let builtins = refs
        .iter()
        .filter(|item| {
            (item.kind == "effect"
                && (pm_fx_defs::catalog::main_catalog()
                    .current(&item.definition)
                    .is_some()
                    || pm_fx_defs::catalog::v120_catalog()
                        .current(&item.definition)
                        .is_some()))
                || (item.kind == "transition"
                    && pm_fx_defs::builtins()
                        .transitions
                        .iter()
                        .any(|d| d.id == item.definition))
                || item.definition.starts_with("powermove.")
        })
        .map(|item| (item.kind.clone(), item.definition.clone()))
        .collect();
    Bundle::collect(refs, &library, &owners, &builtins, &unavailable, true)
}

pub fn open_report(extra: &pm_model::json::Map, profile: &Path) -> (Value, BTreeSet<String>) {
    match Bundle::read(extra) {
        Ok(bundle) if !bundle.entries.is_empty() => {
            let library = installed(&library_roots(profile));
            (bundle.report(&library), bundle.exclusions(&library))
        }
        Ok(_) => (json!({}), BTreeSet::new()),
        Err(error) => (json!({"extensionNotice": error}), BTreeSet::new()),
    }
}

pub fn available_locally(entry: &Entry, library: &BTreeMap<String, Installed>) -> bool {
    library
        .get(&entry.id)
        .and_then(|found| found.manifest.as_ref())
        .is_some_and(|m| entry.version.is_empty() || m["version"] == entry.version)
}

fn builtin_dependency(id: &str, definitions: &BTreeSet<(String, String)>) -> bool {
    matches!(id, "viewer" | "effects-basic" | "transitions-basic")
        || definitions.iter().any(|(_, definition)| definition == id)
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && path.split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && !component.contains(['\\', ':', '\0'])
        })
}

fn valid_package(entry: &Entry, files: &Files) -> bool {
    let manifest = files
        .get("manifest.json")
        .and_then(|raw| serde_json::from_slice::<Value>(raw).ok());
    manifest.is_some_and(|m| {
        let entry_point = m["entry"].as_str().unwrap_or("index.ts");
        m["id"] == entry.id
            && m["version"] == entry.version
            && files.contains_key(entry_point.trim_start_matches("./"))
    }) && files.keys().all(|path| {
        valid_path(path) && !path.eq_ignore_ascii_case(".powermove-project-origin.json")
    }) && match entry.source.as_str() {
        "local" => !files
            .keys()
            .any(|path| path.eq_ignore_ascii_case(STORE_MARKER)),
        "store" => files
            .get(STORE_MARKER)
            .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
            .is_some_and(|m| m["redistributable"] == true),
        _ => false,
    }
}

fn package_hash(files: &Files) -> String {
    let mut hash = Sha256::new();
    hash.update(b"Powermove PMV extension 1\0");
    for (path, data) in files {
        hash.update((path.len() as u64).to_be_bytes());
        hash.update(path.as_bytes());
        hash.update((data.len() as u64).to_be_bytes());
        hash.update(data);
    }
    format!("{:x}", hash.finalize())
}

fn model_value(value: Value) -> Result<pm_model::Value, String> {
    pm_model::json::parse(&value.to_string()).map_err(|e| e.to_string())
}

fn wire(value: &pm_model::Value) -> Result<Value, String> {
    serde_json::from_str(&pm_model::json::to_canonical_string(value)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
