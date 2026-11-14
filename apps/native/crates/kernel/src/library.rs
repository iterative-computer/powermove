//! The v1.2.0 library reader (`<profile>/store/<key>.json`).
//!
//! `LibraryStore` is read-only; the locked, debounced write path is in
//! [`crate::persistence`], and shell workflows in [`crate::session`]. Key
//! families follow D-STATE §1 and v1.2.0's `shared/store-keys.ts`
//! (42 literal keys, 10 dynamic prefixes, ids `[A-Za-z0-9_-]{1,120}`).
//!
//! Loading a project ports `projects.ts` `getAsync` + `getStateAsync`:
//!
//! * `project.<id>`: `{v, proj}` envelope, a bare project, or a stringified
//!   one (`R.unwrap`); a missing slot falls back to the legacy `autosave`
//!   envelope when its project has that id.
//! * `projectJournal.<id>`: `[{revision, sequence?, patches}]` applied in order.
//!   v1.3 skips sequences covered by the envelope's `recoverySequence`; only
//!   old records/envelopes fall back to document revisions, with the legacy
//!   `applyPatches` semantics (silently skipping patches whose parent is
//!   missing, splicing arrays on delete).
//! * History: `projectHistory.<id>` when present, else the legacy combined
//!   `projectState.<id>`'s `history` (plain or packed dictionary).
//! * Session: `projectMeta.<id>` over `projectState.<id>` (minus history).
//! * Workspace: `projectWorkspace.<id>`.
//!
//! Unknown fields are preserved (the project value is kept whole), and
//! quarantined `*.json.corrupt-*` files and unregistered keys are listed,
//! never deleted (D-STATE decisions 2 and 6).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use pm_model::json::{Map, Value, parse};

/// `STATIC_KEYS` at v1.2.0.
pub const STATIC_KEYS: [&str; 42] = [
    "projects",
    "openWindows",
    "windowTabs",
    "lastActiveProject",
    "openTabs",
    "restoreWindows",
    "autoDownloadCloudMedia",
    "previewMemoryMiB",
    "previewResolution",
    "projectTrash",
    "takes",
    "workspaces",
    "workspace",
    "defaultWorkspace",
    "afterEffectsExtensions",
    "workspaceTrash",
    "panelVisibility",
    "theme",
    "themeMode",
    "exportOpts",
    "renderQueue",
    "renderPresets",
    "bootVersion",
    "autosave",
    "projectsSection",
    "projectsSort",
    "projectsView",
    "agentScope",
    "agentPresentation",
    "agentFloatingPosition",
    "agentBubblePosition",
    "agentProvider",
    "agentModel",
    "agentReasoningEffort",
    "agentAccessMode",
    "agentApprovalMode",
    "agentAutoApplyPanels",
    "extensions",
    "activeTheme",
    "extensionWorkspaceAppearance",
    "extensionThemeAppearance",
    "cloudAccount",
];

/// Keys only the native app writes (v1.2.0 leaves them alone): the person's
/// keymap (Settings › Shortcuts).
pub const NATIVE_KEYS: [&str; 1] = ["keymap"];

/// `DYNAMIC_PREFIXES` at v1.2.0.
pub const DYNAMIC_PREFIXES: [&str; 10] = [
    "project",
    "projectJournal",
    "projectState",
    "projectMeta",
    "projectHistory",
    "projectWorkspace",
    "agentThreads",
    "agentAttachment",
    "agentModel",
    "ext",
];

/// A file in the store directory.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StoreKey {
    Static(String),
    Dynamic {
        prefix: String,
        id: String,
    },
    /// A `.json` file whose key is not registered (kept as opaque input).
    Unregistered(String),
    /// `<key>.json.corrupt-<timestamp>`: quarantined, recoverable input.
    Quarantined(String),
}

fn valid_id(id: &str) -> bool {
    (1..=120).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `parseStoreKey`.
pub fn parse_key(key: &str) -> Option<StoreKey> {
    if STATIC_KEYS.contains(&key) || NATIVE_KEYS.contains(&key) {
        return Some(StoreKey::Static(key.to_owned()));
    }
    let (prefix, id) = key.split_once('.')?;
    (DYNAMIC_PREFIXES.contains(&prefix) && valid_id(id)).then(|| StoreKey::Dynamic {
        prefix: prefix.to_owned(),
        id: id.to_owned(),
    })
}

#[derive(Debug)]
pub enum LibraryError {
    Io(io::Error),
    /// A key's file is not valid JSON (the app would quarantine it).
    Json {
        key: String,
        message: String,
    },
    InvalidKey(String),
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LibraryError::Io(e) => write!(f, "{e}"),
            LibraryError::Json { key, message } => write!(f, "{key}: {message}"),
            LibraryError::InvalidKey(k) => write!(f, "not a store key: {k}"),
        }
    }
}

impl std::error::Error for LibraryError {}

impl From<io::Error> for LibraryError {
    fn from(e: io::Error) -> Self {
        LibraryError::Io(e)
    }
}

/// One row of the `projects` index (`{id, name, at, w, h, thumb?}`).
#[derive(Clone, Debug)]
pub struct LibraryEntry {
    pub id: String,
    pub name: String,
    pub at: Option<f64>,
    pub has_thumb: bool,
    /// The whole row, unknown fields included.
    pub raw: Value,
}

/// A library project as the old app would open it.
#[derive(Clone, Debug)]
pub struct LibraryProject {
    pub id: String,
    /// The stored-form project after its journal (unknown fields kept).
    pub project: Value,
    /// Journal entries applied / skipped as not newer than the slot.
    pub journal_applied: usize,
    pub journal_skipped: usize,
    /// The saved history (plain v1 or packed dictionary), if any.
    pub history: Option<Value>,
    /// `projectMeta` over `projectState` (without `history`).
    pub session: Option<Value>,
    pub workspace: Option<Value>,
    /// The slot came from the legacy `autosave` envelope.
    pub from_autosave: bool,
}

/// Counts per family (the read-only smoke test reports these only).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibrarySummary {
    pub files: usize,
    pub static_keys: usize,
    pub dynamic: std::collections::BTreeMap<String, usize>,
    pub unregistered: usize,
    pub quarantined: usize,
    pub index_rows: usize,
    pub trash_rows: usize,
    /// Project slots not in the index or trash (kept, never dropped).
    pub orphan_slots: usize,
    /// Index rows without a slot.
    pub missing_slots: usize,
}

/// The profile's file store, read-only.
#[derive(Clone, Debug)]
pub struct LibraryStore {
    dir: PathBuf,
}

impl LibraryStore {
    /// `path` is the profile (`…/Powermove`) or its `store` directory.
    pub fn open(path: &Path) -> LibraryStore {
        let store = path.join("store");
        LibraryStore {
            dir: if store.is_dir() {
                store
            } else {
                path.to_path_buf()
            },
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every file, classified. Temp files (`.tmp-`) are skipped.
    pub fn keys(&self) -> Result<Vec<StoreKey>, LibraryError> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if name.contains(".tmp-") {
                continue;
            }
            if name.contains(".json.corrupt-") {
                out.push(StoreKey::Quarantined(name));
                continue;
            }
            let Some(key) = name.strip_suffix(".json") else {
                continue;
            };
            out.push(parse_key(key).unwrap_or_else(|| StoreKey::Unregistered(key.to_owned())));
        }
        out.sort();
        Ok(out)
    }

    /// `PM.store.get(key)`: `None` when absent.
    pub fn read(&self, key: &str) -> Result<Option<Value>, LibraryError> {
        if parse_key(key).is_none() {
            return Err(LibraryError::InvalidKey(key.to_owned()));
        }
        let path = self.dir.join(format!("{key}.json"));
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let value = parse(&text).map_err(|e| LibraryError::Json {
            key: key.to_owned(),
            message: e.to_string(),
        })?;
        decode_disk_value(value, value_limit(key))
            .map(Some)
            .map_err(|message| LibraryError::Json {
                key: key.to_owned(),
                message,
            })
    }

    fn rows(&self, key: &str) -> Result<Vec<LibraryEntry>, LibraryError> {
        let Some(Value::Array(rows)) = self.read(key)? else {
            return Ok(Vec::new());
        };
        Ok(rows
            .into_iter()
            .filter_map(|raw| {
                let m = raw.as_object()?;
                let id = m.get("id")?.as_str()?.to_owned();
                Some(LibraryEntry {
                    name: m
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Untitled")
                        .to_owned(),
                    at: m.get("at").and_then(Value::as_f64),
                    has_thumb: m.get("thumb").is_some_and(|t| !t.is_null()),
                    id,
                    raw,
                })
            })
            .collect())
    }

    /// The `projects` index (`R.list`).
    pub fn projects(&self) -> Result<Vec<LibraryEntry>, LibraryError> {
        self.rows("projects")
    }

    /// `projectTrash` (`R.trashList`).
    pub fn trash(&self) -> Result<Vec<LibraryEntry>, LibraryError> {
        self.rows("projectTrash")
    }

    /// `R.getAsync(id)` + `R.getStateAsync(id)` + the workspace slot.
    pub fn load(&self, id: &str) -> Result<Option<LibraryProject>, LibraryError> {
        if !valid_id(id) {
            return Err(LibraryError::InvalidKey(id.to_owned()));
        }
        let mut from_autosave = false;
        let mut checkpoint_sequence = None;
        let mut project = match self.read(&format!("project.{id}"))? {
            Some(raw) => {
                let envelope = parse_project_envelope(raw);
                checkpoint_sequence = envelope
                    .as_ref()
                    .and_then(|v| v.get("recoverySequence"))
                    .cloned();
                envelope.and_then(unwrap_project)
            }
            None => {
                let old = self.read("autosave")?.and_then(unwrap_project);
                from_autosave = old
                    .as_ref()
                    .is_some_and(|p| p.get("id").and_then(Value::as_str) == Some(id));
                if from_autosave { old } else { None }
            }
        };
        let Some(mut current) = project.take() else {
            return Ok(None);
        };
        let mut applied = 0;
        let mut skipped = 0;
        if let Some(Value::Array(entries)) = self.read(&format!("projectJournal.{id}"))? {
            for entry in entries {
                // v1.3 journals use a sequence independent of the edit revision:
                // undo can rewind revisions. Preserve v1.2's fallback only when
                // neither side carries the new sequence contract (N3/N4).
                let sequence = entry.get("sequence").and_then(Value::as_f64);
                let checkpoint = checkpoint_sequence
                    .as_ref()
                    .map(crate::commands::js_number)
                    .filter(|v| !v.is_nan())
                    .unwrap_or(0.0);
                let revision = entry.get("revision").and_then(Value::as_f64);
                let slot_revision = current.get("revision").and_then(Value::as_f64);
                let old_revision = checkpoint_sequence.is_none()
                    && entry.get("sequence").is_none()
                    && matches!((revision, slot_revision), (Some(r), Some(s)) if r.is_finite() && s.is_finite() && r <= s);
                if sequence.is_some_and(|n| n.is_finite() && n <= checkpoint) || old_revision {
                    skipped += 1;
                    continue;
                }
                let mut root = Some(current);
                if let Some(patches) = entry.get("patches") {
                    legacy_apply_patches(&mut root, patches);
                }
                current = root.unwrap_or(Value::Null);
                applied += 1;
            }
        }
        // A v1 project reads its video as v1 showed it, as a file does.
        crate::project_io::adopt_electron_rendering(&mut current, false);
        let metadata = self.read(&format!("projectMeta.{id}"))?;
        let history = self.read(&format!("projectHistory.{id}"))?;
        let legacy = if metadata.is_none() || history.is_none() {
            self.read(&format!("projectState.{id}"))?
        } else {
            None
        };
        let mut session = match (legacy, metadata) {
            (Some(Value::Object(mut l)), Some(Value::Object(m))) => {
                for (k, v) in m.iter() {
                    l.insert(k.clone(), v.clone());
                }
                Some(Value::Object(l))
            }
            (_, Some(m)) => Some(m),
            (l, None) => l,
        };
        let history = match history {
            Some(h) => Some(h),
            None => session
                .as_mut()
                .and_then(Value::as_object_mut)
                .and_then(|s| s.remove("history")),
        };
        if let Some(Value::Object(s)) = &mut session {
            s.remove("history");
        }
        Ok(Some(LibraryProject {
            id: id.to_owned(),
            project: current,
            journal_applied: applied,
            journal_skipped: skipped,
            history: history.filter(|h| !h.is_null()),
            session,
            workspace: self.read(&format!("projectWorkspace.{id}"))?,
            from_autosave,
        }))
    }

    /// Counts per family, index reconciliation (no values).
    pub fn summary(&self) -> Result<LibrarySummary, LibraryError> {
        let keys = self.keys()?;
        let mut s = LibrarySummary {
            files: keys.len(),
            ..Default::default()
        };
        let mut slots = std::collections::BTreeSet::new();
        for k in &keys {
            match k {
                StoreKey::Static(_) => s.static_keys += 1,
                StoreKey::Dynamic { prefix, id } => {
                    *s.dynamic.entry(prefix.clone()).or_default() += 1;
                    if prefix == "project" {
                        slots.insert(id.clone());
                    }
                }
                StoreKey::Unregistered(_) => s.unregistered += 1,
                StoreKey::Quarantined(_) => s.quarantined += 1,
            }
        }
        let index = self.projects()?;
        let trash = self.trash()?;
        s.index_rows = index.len();
        s.trash_rows = trash.len();
        let listed: std::collections::BTreeSet<String> = index
            .iter()
            .chain(trash.iter())
            .map(|e| e.id.clone())
            .collect();
        s.orphan_slots = slots.difference(&listed).count();
        s.missing_slots = index.iter().filter(|e| !slots.contains(&e.id)).count();
        Ok(s)
    }
}

/// Per-key value ceilings (`shared/ipc.ts`): the heavy project keys 256 MiB,
/// `takes` 768 MiB, everything else 32 MiB.
pub(crate) fn value_limit(key: &str) -> usize {
    const MIB: usize = 1024 * 1024;
    match key.split_once('.').map_or(key, |(prefix, _)| prefix) {
        "project" | "projectState" | "projectHistory" => 256 * MIB,
        "takes" => 768 * MIB,
        _ => 32 * MIB,
    }
}

/// `decodeStorageJSON` (v1.3.0+, `main/storage-codec.ts`): values of 32 KiB
/// or more may be stored as `{__powermoveDiskValue: 1, encoding: "gzip",
/// bytes, data: base64(gzip(json))}` under the same file name. Not in
/// v1.2.0's D-STATE inventory, but present in real profiles; without it
/// those histories and projects would not open (N4).
pub fn decode_disk_value(value: Value, limit: usize) -> Result<Value, String> {
    let Some(map) = value.as_object() else {
        return Ok(value);
    };
    if map.get("__powermoveDiskValue").is_none() {
        return Ok(value);
    }
    let invalid = || "Invalid compressed recovery record.".to_owned();
    let bytes = map
        .get("bytes")
        .and_then(Value::as_f64)
        .ok_or_else(invalid)?;
    if map.get("__powermoveDiskValue") != Some(&Value::Number(1.0))
        || map.get("encoding").and_then(Value::as_str) != Some("gzip")
        || bytes.fract() != 0.0
        || bytes < 2.0
        || bytes > limit as f64
    {
        return Err(invalid());
    }
    let data = map
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let compressed = crate::project_io::base64::decode(data).ok_or_else(invalid)?;
    let expected = bytes as usize;
    let mut out = Vec::with_capacity(expected);
    {
        use std::io::Read;
        flate2::read::GzDecoder::new(compressed.as_slice())
            .take(expected as u64 + 1)
            .read_to_end(&mut out)
            .map_err(|_| invalid())?;
    }
    if out.len() != expected {
        return Err("The compressed recovery record is incomplete.".into());
    }
    let text = String::from_utf8(out).map_err(|_| invalid())?;
    parse(&text).map_err(|e| e.to_string())
}

/// `R.unwrap`: `{v, proj}`, a bare project, or a JSON string of either.
fn parse_project_envelope(raw: Value) -> Option<Value> {
    let raw = match raw {
        Value::String(s) => {
            let text = s.as_str()?;
            let mut raw = parse(text).ok()?;
            crate::asset_order::adopt_text(&mut raw, text);
            raw
        }
        other => other,
    };
    (!raw.is_null()).then_some(raw)
}

fn unwrap_project(raw: Value) -> Option<Value> {
    let raw = parse_project_envelope(raw)?;
    match raw.get("proj") {
        Some(p @ Value::Object(_)) => Some(p.clone()),
        _ => Some(raw),
    }
}

/// `applyPatches` from `projects.ts` (not `shared/patch.ts`): patches whose
/// parent is missing are skipped; a delete on an array index splices. Journal
/// entries also hold id-keyed operations; one its array does not match is
/// skipped the same way (`applyOpsSkipping`).
pub fn legacy_apply_patches(root: &mut Option<Value>, patches: &Value) {
    let Some(list) = patches.as_array() else {
        return;
    };
    for patch in list {
        let Some(path) = patch.get("path").and_then(Value::as_array) else {
            continue;
        };
        if patch.get("op").and_then(Value::as_str).is_some() {
            if let Some(root) = root.as_mut() {
                apply_id_op(root, path, patch);
            }
            continue;
        }
        let exists = patch
            .get("exists")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let value = patch.get("value").cloned();
        if path.is_empty() {
            *root = if exists {
                Some(value.unwrap_or(Value::Null))
            } else {
                None
            };
            continue;
        }
        let Some(parent) = root.as_mut().and_then(|r| walk(r, &path[..path.len() - 1])) else {
            continue;
        };
        let key = &path[path.len() - 1];
        match parent {
            Value::Array(items) => {
                let Some(i) = index_of(key) else { continue };
                if exists {
                    let v = value.unwrap_or(Value::Null);
                    if i < items.len() {
                        items[i] = v;
                    } else {
                        // JS `arr[i] = v` past the end pads with holes
                        // (serialized as null).
                        items.resize(i, Value::Null);
                        items.push(v);
                    }
                } else if i < items.len() && key.as_f64().is_some() {
                    items.remove(i);
                }
            }
            Value::Object(map) => {
                let k = property_key(key);
                if exists {
                    map.insert(k.as_str(), value.unwrap_or(Value::Null));
                } else {
                    map.remove(&k);
                }
            }
            _ => {}
        }
    }
}

/// `applyIdOp` from `shared/patch-ids.ts`; `None` is its `IdOpMismatch`.
fn apply_id_op(root: &mut Value, path: &[Value], op: &Value) -> Option<()> {
    let Value::Array(items) = walk(root, path)? else {
        return None;
    };
    let mut ids: Vec<String> = Vec::with_capacity(items.len());
    for item in items.iter() {
        let id = item.as_object()?.get("id")?.as_str()?;
        if ids.iter().any(|known| known == id) {
            return None;
        }
        ids.push(id.to_owned());
    }
    match op.get("op")?.as_str()? {
        "order" => {
            let picks: Vec<usize> = op
                .get("ids")?
                .as_array()?
                .iter()
                .map(|id| {
                    let id = id.as_str()?;
                    ids.iter().position(|known| known == id)
                })
                .collect::<Option<_>>()?;
            let mut named = picks.clone();
            named.sort_unstable();
            named.dedup();
            if picks.len() != items.len() || named.len() != items.len() {
                return None;
            }
            let mut slots: Vec<Option<Value>> =
                std::mem::take(items).into_iter().map(Some).collect();
            *items = picks.into_iter().filter_map(|i| slots[i].take()).collect();
        }
        "insert" => {
            let index = op.get("index")?.as_f64()?;
            let value = op.get("value")?;
            let id = value.as_object()?.get("id")?.as_str()?;
            let fits = index.fract() == 0. && index >= 0. && index <= items.len() as f64;
            if !fits || ids.iter().any(|known| known == id) {
                return None;
            }
            items.insert(index as usize, value.clone());
        }
        "delete" => {
            let id = op.get("id")?.as_str()?;
            let at = ids.iter().position(|known| *known == id)?;
            items.remove(at);
        }
        _ => return None,
    }
    Some(())
}

fn property_key(part: &Value) -> String {
    match part {
        Value::String(s) => s.as_str().unwrap_or_default().to_owned(),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e21 {
                format!("{}", *n as i64)
            } else {
                format!("{n}")
            }
        }
        other => pm_model::json::to_canonical_string(other),
    }
}

fn index_of(part: &Value) -> Option<usize> {
    match part {
        Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 => Some(*n as usize),
        Value::String(s) => s.as_str()?.parse().ok(),
        _ => None,
    }
}

/// The value at `path`, skipping (as `applyPatches` does) when any step is
/// missing or null.
fn walk<'a>(value: &'a mut Value, path: &[Value]) -> Option<&'a mut Value> {
    match path.split_first() {
        None => Some(value),
        Some((part, rest)) => {
            let child = child_mut(value, part)?;
            if child.is_null() {
                return None;
            }
            walk(child, rest)
        }
    }
}

fn child_mut<'a>(parent: &'a mut Value, part: &Value) -> Option<&'a mut Value> {
    match parent {
        Value::Array(items) => items.get_mut(index_of(part)?),
        Value::Object(map) => map.get_mut(&property_key(part)),
        _ => None,
    }
}

#[allow(dead_code)]
fn _unused(_: Map) {}
