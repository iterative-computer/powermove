//! Undo/redo records.
//!
//! One [`HistoryRecord`] per committed transaction or finished gesture. A
//! record holds compact canonical bytes for the *forward* and *inverse* operations
//! captured when it was committed, decoding only for a scoped use. Undo applies the inverse as a **new,
//! increasing document revision** (never a rewind), so render, audio and
//! query readers that follow revisions see an ordinary edit.
//!
//! # Compared with the old app (`legacy/core/history.ts`)
//!
//! | Old app | Here |
//! |---|---|
//! | 1,000 entries, 256 MB serialized budget | Same ([`MAX_ENTRIES`], [`DEFAULT_MAX_BYTES`]); bytes are the canonical JSON size of the operations |
//! | Entries hold leaf patches (`forward`/`backward`) | Records hold `Op`s (leaf and id-keyed structural); a structural record also keeps its leaf-patch form for the file |
//! | Trim drops the oldest entries; a lower budget drops distant redo first | Same |
//! | New entry discards redo | Same |
//! | `squash(mark)` combines an agent run | [`DocumentStore::squash_since`](crate::DocumentStore::squash_since) |
//! | `export()`/`import()` as `{ version: 1, paths: 'composition', index, entries }` | [`HistorySnapshot::to_saved`] / [`History::from_saved`], same format (root-relative histories are canonicalized on import, dictionary-packed ones unpacked) |
//! | "Selection" entries while at the top of the stack | Selection-only entries, excluded from persistence |
//! | `external` closure entries | [`HistoryRecord::external`]: `(realm, generation, handle)`, undone by the realm through a kernel continuation (`docs/native/kernel.md`); not exported, as before |
//! | Cleanup closures when entries fall off | [`History::take_retired`] hands the kernel every retired external entry with a cleanup |
//! | Nested `begin`/`commit` depth | The kernel's open entry (`crate::tx`); [`transact`](crate::DocumentStore::transact) takes the whole command list |
//! | `packHistory` (`projectHistory.<id>` dictionary) | [`HistorySnapshot::to_saved_packed`] (content-interned records) |

use std::sync::Arc;

use pm_model::canonical::canonicalize_patches;
use pm_model::ops::{Op, ops_to_value};
use pm_model::patch::{Patch, PathPart};
use pm_model::{JsString, Map, Value};

use crate::commands::Origin;

/// Most records kept (`MAX_ENTRIES`).
pub const MAX_ENTRIES: usize = 1_000;
/// Default byte budget (`DEFAULT_MAX_BYTES`).
pub const DEFAULT_MAX_BYTES: usize = 256 * 1024 * 1024;

/// An entry whose undo/redo runs in an extension realm (`history.external`).
/// The closures live in the realm; the kernel holds only this handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExternalRef {
    pub realm: u32,
    pub generation: u32,
    pub handle: u64,
    /// The realm registered a cleanup closure (`options.cleanup`), run when
    /// the entry retires ([`History::take_retired`]).
    pub cleanup: bool,
}

/// Immutable canonical JSON, compressed only when that saves space. No decoded
/// cache: a scan or save must not permanently expand the entire undo stack.
#[derive(Clone, Debug, PartialEq)]
pub struct CompactJson {
    data: Arc<[u8]>,
    compressed: bool,
    len: usize,
    old_3d: bool,
    exact: Option<Arc<Value>>,
    entries: Option<Arc<[CompactJson]>>,
}
impl CompactJson {
    pub fn new(value: &Value) -> Self {
        let mut stored = Self::text(pm_model::json::to_canonical_string(value));
        // Runtime edits can contain non-finite numbers or signed zero. JSON
        // normalizes those, so retain their exact form in this rare case.
        if needs_exact(value) {
            stored.exact = Some(Arc::new(value.clone()));
        }
        stored
    }
    /// Preserve a plain history entry by entry: large envelopes serialize on
    /// workers instead of imposing a second serial whole-history pass on open.
    pub(crate) fn history(value: &Value) -> Self {
        let Some(entries) = value.get("entries").and_then(Value::as_array) else {
            return Self::new(value);
        };
        let mut metadata = Map::new();
        for (key, value) in value.as_object().unwrap().iter() {
            if key.as_str() != Some("entries") {
                metadata.insert(key.clone(), value.clone());
            }
        }
        let entries = pm_model::parallel::map(entries.len(), |i| Self::new(&entries[i]));
        let mut stored = Self::new(&Value::Object(metadata));
        stored.entries = Some(entries.into());
        stored
    }
    fn text(text: String) -> Self {
        let len = text.len();
        let old_3d = text.contains("powermove.3d.");
        let packed = zstd::bulk::compress(text.as_bytes(), 1).expect("history compression");
        let compressed = packed.len() < len;
        let data = if compressed {
            packed
        } else {
            text.into_bytes()
        };
        Self {
            data: data.into(),
            compressed,
            len,
            old_3d,
            exact: None,
            entries: None,
        }
    }
    pub fn value(&self) -> Value {
        if let Some(value) = &self.exact {
            return (**value).clone();
        }
        let bytes = if self.compressed {
            zstd::bulk::decompress(&self.data, self.len).expect("internal history bytes")
        } else {
            self.data.to_vec()
        };
        let mut value = pm_model::json::parse_file_text(&bytes).expect("canonical history JSON");
        if let Some(entries) = &self.entries {
            value.as_object_mut().unwrap().insert(
                "entries",
                Value::Array(entries.iter().map(Self::value).collect()),
            );
        }
        value
    }
}

fn needs_exact(value: &Value) -> bool {
    match value {
        Value::Number(n) => !n.is_finite() || (*n == 0.0 && n.is_sign_negative()),
        Value::Array(items) => items.iter().any(needs_exact),
        Value::Object(map) => map.iter().any(|(_, v)| needs_exact(v)),
        _ => false,
    }
}

/// Operations whose decoded lifetime is scoped to an undo, redo or scan.
#[derive(Clone, Debug)]
pub struct StoredOps(CompactJson);
impl PartialEq for StoredOps {
    fn eq(&self, other: &Self) -> bool {
        self.to_vec() == other.to_vec()
    }
}
impl From<Vec<Op>> for StoredOps {
    fn from(ops: Vec<Op>) -> Self {
        Self(CompactJson::new(&ops_to_value(&ops)))
    }
}
impl StoredOps {
    pub(crate) fn may_have_old_3d(&self) -> bool {
        self.0.old_3d
    }
    pub fn to_vec(&self) -> Vec<Op> {
        pm_model::ops::ops_from_value(&self.value()).expect("validated history operations")
    }
    fn value(&self) -> Value {
        self.0.value()
    }
    pub fn iter(&self) -> std::vec::IntoIter<Op> {
        self.to_vec().into_iter()
    }
    pub fn len(&self) -> usize {
        self.iter().count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One undoable step.
#[derive(Clone, Debug)]
pub struct HistoryRecord {
    /// Unique for the process, never reused. Also names the document state
    /// right after this step (see [`History::state_token`]).
    pub id: u64,
    /// The undo-menu label.
    pub label: String,
    pub origin: Origin,
    /// Operations that redo the step from the state right after undo.
    pub forward: StoredOps,
    /// Operations that undo the step from the state right after it.
    pub inverse: StoredOps,
    /// Leaf-patch form (`forward`, `backward`) for the saved file, present
    /// when `forward`/`inverse` contain structural operations.
    pub(crate) saved: Option<CompactJson>,
    /// The document revision the step was committed at (0 when imported).
    pub revision: u64,
    /// Canonical serialized size, for the compatible trimming budget. Normal
    /// records occupy no more payload bytes than this after compression.
    pub bytes: usize,
    /// `Some` for an external (closure) entry: no operations, undone by its
    /// realm. Never exported (the old `export()` kept document entries only).
    pub external: Option<ExternalRef>,
    pub selection: Option<Arc<(crate::selection::Selection, crate::selection::Selection)>>,
    /// The digest of the form the file stores for this step, computed once
    /// and shared by every copy ([`History::identity`]).
    digest: Arc<std::sync::OnceLock<String>>,
    /// The assets the step refers to ([`Self::referenced_assets`]), found
    /// once and shared by every copy.
    assets: Arc<std::sync::OnceLock<Box<[String]>>>,
    /// The step's entry exactly as the saved history writes it
    /// ([`Self::saved_text`]), made once and shared by every copy.
    saved_text: Arc<std::sync::OnceLock<SavedText>>,
}

/// One step's saved entry as canonical JSON text, compressed: what
/// `to_saved` writes for it before its label. A save writes these bytes
/// instead of rebuilding and serializing the step again.
#[derive(Debug)]
pub(crate) struct SavedText {
    data: Box<[u8]>,
    len: usize,
    /// What reading the entry back finds (`History::import` of a history
    /// saved with composition paths, which never consults the open
    /// composition), for the file guard.
    import: Result<ImportFacts, String>,
}
/// What `History::import` makes of one saved entry, as the file guard asks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImportFacts {
    /// The imported record's byte size (the import budget's measure).
    pub bytes: usize,
    /// The entry is saved as id-keyed operations rather than leaf patches.
    pub operations: bool,
}
impl SavedText {
    fn text(&self) -> String {
        let bytes = zstd::bulk::decompress(&self.data, self.len).expect("internal history bytes");
        String::from_utf8(bytes).expect("canonical history text")
    }
}

impl HistoryRecord {
    /// SHA-256 of the forward and backward lists the file stores for this
    /// step (`to_saved`'s entry, label aside).
    fn digest(&self) -> &str {
        self.digest.get_or_init(|| {
            let (forward, backward) = match self.saved_form() {
                Some((f, b)) => (
                    Value::Array(f.iter().map(Patch::to_value).collect()),
                    Value::Array(b.iter().map(Patch::to_value).collect()),
                ),
                None => (self.forward.value(), self.inverse.value()),
            };
            let form = Value::Array(vec![forward, backward]);
            pm_model::hash::sha256_hex(pm_model::json::to_canonical_string(&form).as_bytes())
        })
    }

    pub fn is_document(&self) -> bool {
        self.external.is_none() && self.selection.is_none()
    }

    /// The leaf patches the file stores for this record (`forward`,
    /// `backward`). `None` for a structural edit too large to keep whole: it
    /// saves as operations, and its entry costs what it changed.
    /// The asset ids the step's operations refer to: what its patches add
    /// under `assets`, and the originals of anything undoing or redoing it
    /// would put back (a deleted layer's value). A save keeps those media
    /// live. The step's operations never change, so they are read once: a
    /// save asks every step, and inflating a long history's stored
    /// operations on each save made every save slow.
    pub(crate) fn referenced_assets(&self) -> &[String] {
        self.assets.get_or_init(|| {
            let mut ids = std::collections::HashSet::new();
            let mut add_keys = |v: Option<&Value>, ids: &mut std::collections::HashSet<String>| {
                if let Some(Value::Object(map)) = v {
                    ids.extend(map.keys().filter_map(|k| k.as_str().map(str::to_owned)));
                }
            };
            let ops: Vec<Op> = self.forward.iter().chain(self.inverse.iter()).collect();
            if self.saved.is_some()
                && let Some((f, b)) = self.saved_form()
            {
                for p in f.iter().chain(b.iter()) {
                    crate::project_io::collect_patch_assets(p, &mut ids, &mut add_keys);
                }
            } else {
                for op in &ops {
                    if let Op::Leaf(p) = op {
                        crate::project_io::collect_patch_assets(p, &mut ids, &mut add_keys);
                    }
                }
            }
            for op in &ops {
                // A deleted layer's value is what Undo would reinsert.
                let value = match op {
                    Op::Leaf(p) => p.value.as_ref(),
                    Op::Insert { value, .. } => Some(value),
                    _ => None,
                };
                if let Some(value) = value {
                    crate::commands::scene3d::collect_original_assets(value, &mut ids);
                }
            }
            let mut ids: Vec<String> = ids.into_iter().collect();
            ids.sort();
            ids.into()
        })
    }
    /// Finds what a save asks of this step (the assets it reaches, the
    /// digest of its saved form) ahead of the save. Both are found once and
    /// shared by every copy of the step, so this may run on any thread.
    pub(crate) fn warm(&self) {
        self.referenced_assets();
        if self.is_document() {
            self.digest();
            self.saved_text();
        }
    }
    /// The entry `to_saved` writes for this step, less its label: the
    /// canonical `{"backward":…,"forward":…` exactly as that export
    /// serializes it. Built once from the same values the export builds.
    pub(crate) fn saved_text(&self) -> &SavedText {
        self.saved_text.get_or_init(|| {
            let (forward, backward) = self.saved_sides();
            let mut entry = Map::new();
            entry.insert("label", Value::from(self.label.as_str()));
            entry.insert("forward", forward);
            entry.insert("backward", backward);
            let entry = Value::Object(entry);
            // The file guard's test for id-keyed operations, on this entry.
            let operations = ["forward", "backward"].iter().any(|side| {
                entry
                    .get(side)
                    .and_then(Value::as_array)
                    .is_some_and(|ops| {
                        ops.iter().any(|op| {
                            let op = op.get("patch").unwrap_or(op);
                            op.get("op").and_then(Value::as_str).is_some()
                        })
                    })
            });
            let import = import_entry(&entry, false, None)
                .map(|(_, _, _, bytes)| ImportFacts { bytes, operations });
            let mut text = String::from("{\"backward\":");
            pm_model::json::write::write_canonical(
                entry.get("backward").expect("backward"),
                &mut text,
            );
            text.push_str(",\"forward\":");
            pm_model::json::write::write_canonical(
                entry.get("forward").expect("forward"),
                &mut text,
            );
            let len = text.len();
            SavedText {
                data: zstd::bulk::compress(text.as_bytes(), 3)
                    .expect("history compression")
                    .into(),
                len,
                import,
            }
        })
    }
    /// The forward and backward lists `to_saved` stores for this step.
    fn saved_sides(&self) -> (Value, Value) {
        let list = |p: Vec<Patch>| Value::Array(p.iter().map(Patch::to_value).collect());
        match self.saved_form() {
            Some((forward, backward)) => (list(forward), list(backward)),
            None => (self.forward.value(), self.inverse.value()),
        }
    }
    /// What `History::import` makes of this step's saved entry.
    pub(crate) fn import_facts(&self) -> Result<ImportFacts, String> {
        self.saved_text().import.clone()
    }
    pub(crate) fn saved_form(&self) -> Option<(Vec<Patch>, Vec<Patch>)> {
        if let Some(saved) = &self.saved {
            let value = saved.value();
            let sides = value.as_array().expect("patch pair");
            let patches = |v: &Value| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|p| Patch::from_value(p).unwrap())
                    .collect()
            };
            return Some((patches(&sides[0]), patches(&sides[1])));
        }
        let leaves = |ops: &[Op]| {
            ops.iter()
                .map(|op| match op {
                    Op::Leaf(p) => Some(p.clone()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
        };
        Some((
            leaves(&self.forward.to_vec())?,
            leaves(&self.inverse.to_vec())?,
        ))
    }
}

pub(crate) fn ops_bytes(ops: &[Op]) -> usize {
    pm_model::json::to_canonical_string(&ops_to_value(ops)).len()
}

fn patches_bytes(patches: &[Patch]) -> usize {
    let value = Value::Array(patches.iter().map(Patch::to_value).collect());
    pm_model::json::to_canonical_string(&value).len()
}

/// Process-unique ids for records and document states.
pub(crate) fn next_token() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// What a [`History`] holds, for menus and diagnostics (`PM.hist.stats()`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryStats {
    pub entries: usize,
    pub applied: usize,
    pub bytes: usize,
    pub max_bytes: usize,
}

/// The undo/redo stack. The first `cursor` records are applied.
#[derive(Clone, Debug)]
pub struct History {
    entries: Vec<HistoryRecord>,
    cursor: usize,
    total_bytes: usize,
    max_entries: usize,
    max_bytes: usize,
    /// The state token of "no record applied": the opened document, or the
    /// state after the newest record trimmed from the bottom.
    base_token: u64,
    /// External entries with a cleanup that fell off the stack, waiting for
    /// the kernel to ask their realm to run it.
    retired: Vec<ExternalRef>,
    /// The file guard's verdict on document states it has judged, by state
    /// token, each with the outside-write count it was judged under.
    guard_states: GuardStates,
}
/// Shared by a history and its snapshots: whether a document state (named
/// by its token, judged under an outside-write count) needs the native
/// reader (the project file guard).
pub(crate) type GuardStates = Arc<std::sync::Mutex<GuardMemo>>;
#[derive(Debug, Default)]
pub(crate) struct GuardMemo {
    pub states: std::collections::HashMap<u64, (u64, bool)>,
    /// Saves that had to replay the whole history (diagnostics, tests).
    pub full_replays: u64,
}

impl Default for History {
    fn default() -> Self {
        History::new()
    }
}

/// A position to squash back to ([`History::mark`]; `PM.hist.mark()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryMark {
    top_id: Option<u64>,
}

/// A frozen copy of the stack for a save running on another thread.
#[derive(Clone, Debug)]
pub struct HistorySnapshot {
    records: Vec<HistoryRecord>,
    cursor: usize,
    base_token: u64,
    guard_states: GuardStates,
}

impl History {
    pub fn new() -> History {
        History::with_limits(MAX_ENTRIES, DEFAULT_MAX_BYTES)
    }

    pub fn with_limits(max_entries: usize, max_bytes: usize) -> History {
        History {
            entries: Vec::new(),
            cursor: 0,
            total_bytes: 0,
            max_entries,
            max_bytes,
            base_token: next_token(),
            guard_states: Default::default(),
            retired: Vec::new(),
        }
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.entries.len()
    }

    /// The record undo would revert.
    pub fn peek_undo(&self) -> Option<&HistoryRecord> {
        self.cursor.checked_sub(1).and_then(|i| self.entries.get(i))
    }

    /// The record redo would reapply.
    pub fn peek_redo(&self) -> Option<&HistoryRecord> {
        self.entries.get(self.cursor)
    }

    /// Label of the step undo would revert (`PM.hist.label()`).
    pub fn undo_label(&self) -> Option<&str> {
        self.peek_undo().map(|r| r.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.peek_redo().map(|r| r.label.as_str())
    }

    /// Labels, oldest first (`PM.hist.list()`).
    pub fn labels(&self) -> Vec<&str> {
        self.entries.iter().map(|r| r.label.as_str()).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of applied records (the undo depth).
    pub fn applied(&self) -> usize {
        self.cursor
    }

    pub fn records(&self) -> &[HistoryRecord] {
        &self.entries
    }

    pub fn stats(&self) -> HistoryStats {
        HistoryStats {
            entries: self.entries.len(),
            applied: self.cursor,
            bytes: self.total_bytes,
            max_bytes: self.max_bytes,
        }
    }

    /// Names the document state the stack is at: the id of the newest
    /// applied record, or the base token. Two moments with the same token
    /// hold the same document content (that is how "dirty" is decided).
    pub fn state_token(&self) -> u64 {
        self.entries[..self.cursor]
            .iter()
            .rev()
            .find(|r| r.selection.is_none())
            .map_or(self.base_token, |r| r.id)
    }

    /// Lowers (or raises) the byte budget and trims to it (`Memory.trim`):
    /// distant redo first, then the oldest steps; the newest step stays.
    pub fn set_max_bytes(&mut self, max_bytes: usize) {
        let before = self.entries.len();
        self.max_bytes = max_bytes;
        while self.total_bytes > self.max_bytes && self.entries.len() > 1 {
            if self.cursor < self.entries.len() {
                let removed = self.entries.pop().expect("non-empty");
                self.retire(&removed);
                self.total_bytes -= removed.bytes;
            } else {
                self.drop_oldest();
            }
        }
        if self.entries.len() < before {
            crate::memory::pressure_relief();
        }
    }

    fn retire(&mut self, removed: &HistoryRecord) {
        if let Some(ext) = removed.external
            && ext.cleanup
        {
            self.retired.push(ext);
        }
    }

    /// External entries with a cleanup that left the stack since the last
    /// call (trimmed, discarded redo, cleared). Oldest first.
    pub fn take_retired(&mut self) -> Vec<ExternalRef> {
        std::mem::take(&mut self.retired)
    }

    /// `PM.hist.clear()`: drops every entry (cleanups retire).
    pub fn clear(&mut self) {
        let entries = std::mem::take(&mut self.entries);
        for r in &entries {
            self.retire(r);
        }
        self.total_bytes = 0;
        self.cursor = 0;
        self.base_token = next_token();
    }

    fn drop_oldest(&mut self) {
        let removed = self.entries.remove(0);
        self.retire(&removed);
        self.total_bytes -= removed.bytes;
        if self.cursor > 0 {
            self.cursor -= 1;
            // "No record applied" now means "after the removed one".
            if removed.selection.is_none() {
                self.base_token = removed.id;
            }
        } else {
            // A redo record went; the base state is unchanged.
        }
    }

    /// `trim()`: entries over the count, or over the byte budget while more
    /// than one remains, are dropped oldest first.
    fn trim(&mut self) {
        while self.entries.len() > self.max_entries
            || self.total_bytes > self.max_bytes && self.entries.len() > 1
        {
            self.drop_oldest();
        }
        self.cursor = self.cursor.min(self.entries.len());
    }

    fn discard_redo(&mut self) {
        let removed: Vec<HistoryRecord> = self.entries.drain(self.cursor..).collect();
        for r in &removed {
            self.retire(r);
            self.total_bytes -= r.bytes;
        }
    }

    /// Pushes a committed step, discarding redo. Returns the record id.
    pub(crate) fn push(
        &mut self,
        label: String,
        origin: Origin,
        forward: Vec<Op>,
        inverse: Vec<Op>,
        saved: Option<(Vec<Patch>, Vec<Patch>)>,
        revision: u64,
    ) -> u64 {
        self.discard_redo();
        let bytes = match &saved {
            Some((f, b)) => {
                patches_bytes(f) + patches_bytes(b) + ops_bytes(&forward) + ops_bytes(&inverse)
            }
            None => ops_bytes(&forward) + ops_bytes(&inverse),
        };
        let id = next_token();
        self.entries.push(HistoryRecord {
            id,
            label,
            origin,
            forward: forward.into(),
            inverse: inverse.into(),
            saved: saved.map(|(f, b)| {
                CompactJson::new(&Value::Array(vec![
                    Value::Array(f.iter().map(Patch::to_value).collect()),
                    Value::Array(b.iter().map(Patch::to_value).collect()),
                ]))
            }),
            revision,
            bytes,
            external: None,
            selection: None,
            digest: Default::default(),
            assets: Default::default(),
            saved_text: Default::default(),
        });
        self.total_bytes += bytes;
        self.cursor = self.entries.len();
        self.trim();
        id
    }

    /// `history.external(label, undo, redo, options)`: pushes a closure entry
    /// (discarding redo, as any push). `bytes` is `max(256, options.bytes)`.
    pub(crate) fn push_external(
        &mut self,
        label: String,
        origin: Origin,
        external: ExternalRef,
        bytes: usize,
        revision: u64,
    ) -> u64 {
        self.discard_redo();
        let bytes = bytes.max(256);
        let id = next_token();
        self.entries.push(HistoryRecord {
            id,
            label,
            origin,
            forward: Vec::new().into(),
            inverse: Vec::new().into(),
            saved: None,
            revision,
            bytes,
            external: Some(external),
            selection: None,
            digest: Default::default(),
            assets: Default::default(),
            saved_text: Default::default(),
        });
        self.total_bytes += bytes;
        self.cursor = self.entries.len();
        self.trim();
        id
    }

    pub(crate) fn push_selection(
        &mut self,
        before: crate::selection::Selection,
        after: crate::selection::Selection,
    ) -> Option<u64> {
        if before == after || self.can_redo() {
            return None;
        }
        let bytes = before.encoded_bytes() + after.encoded_bytes();
        let id = next_token();
        self.entries.push(HistoryRecord {
            id,
            label: "Selection".into(),
            origin: Origin::Interface,
            forward: Vec::new().into(),
            inverse: Vec::new().into(),
            saved: None,
            revision: 0,
            bytes,
            external: None,
            selection: Some(Arc::new((before, after))),
            digest: Default::default(),
            assets: Default::default(),
            saved_text: Default::default(),
        });
        self.total_bytes += bytes;
        self.cursor = self.entries.len();
        self.trim();
        Some(id)
    }

    /// Shared retention: distant redo first, then the oldest undo; keep
    /// the latest edit in every project even when it exceeds the budget.
    pub(crate) fn trim_one_shared(&mut self) -> bool {
        if self.entries.len() <= 1 {
            return false;
        }
        if self.can_redo() {
            let removed = self.entries.pop().expect("non-empty");
            self.retire(&removed);
            self.total_bytes -= removed.bytes;
        } else {
            self.drop_oldest();
        }
        true
    }

    /// Removes `n` records from the top (journal replay of a squash).
    pub(crate) fn pop_top(&mut self, n: usize) {
        for _ in 0..n.min(self.entries.len()) {
            let removed = self.entries.pop().expect("non-empty");
            self.total_bytes -= removed.bytes;
        }
        self.cursor = self.cursor.min(self.entries.len());
    }

    pub(crate) fn mark_undone(&mut self) {
        debug_assert!(self.cursor > 0);
        self.cursor -= 1;
    }

    pub(crate) fn mark_redone(&mut self) {
        debug_assert!(self.cursor < self.entries.len());
        self.cursor += 1;
    }

    /// A position to squash back to later (`PM.hist.mark()`).
    pub fn mark(&self) -> HistoryMark {
        HistoryMark {
            top_id: self.peek_undo().map(|r| r.id),
        }
    }

    /// `squash(mark, label)`: combines every record pushed since `mark` into
    /// one. `None` unless the mark is still in the stack and nothing is
    /// undone. Returns the new record id (or the single record's id when
    /// there was only one).
    pub(crate) fn squash_since(&mut self, mark: &HistoryMark, label: &str) -> Option<u64> {
        let base = match mark.top_id {
            None => 0,
            Some(id) => self.entries.iter().position(|r| r.id == id)? + 1,
        };
        if base > self.cursor || self.cursor != self.entries.len() || base == self.entries.len() {
            return None;
        }
        if self.entries[base..].iter().any(|r| !r.is_document()) {
            return None;
        }
        let tail: Vec<HistoryRecord> = self.entries.drain(base..).collect();
        for r in &tail {
            self.total_bytes -= r.bytes;
        }
        self.cursor = self.entries.len();
        let forward: Vec<Op> = tail.iter().flat_map(|r| r.forward.iter()).collect();
        let inverse: Vec<Op> = tail.iter().rev().flat_map(|r| r.inverse.iter()).collect();
        // Whole-value forms join only when every step has one.
        let saved = if tail.iter().any(|r| r.saved.is_some()) {
            tail.iter()
                .map(HistoryRecord::saved_form)
                .collect::<Option<Vec<_>>>()
                .map(|forms| {
                    let backward = forms.iter().rev().flat_map(|(_, b)| b.iter().cloned());
                    let backward = backward.collect();
                    (
                        forms.iter().flat_map(|(f, _)| f.clone()).collect(),
                        backward,
                    )
                })
        } else {
            None
        };
        let revision = tail.last().map_or(0, |r| r.revision);
        Some(self.push(
            label.to_owned(),
            tail[0].origin,
            forward,
            inverse,
            saved,
            revision,
        ))
    }

    /// Identity of the undo history a save writes: equal for this history
    /// and the one a file it saved opens with (the steps `to_saved()` keeps,
    /// their labels and forms, and how many are applied). Each step's form
    /// is hashed once.
    pub fn identity(&self) -> String {
        let mut text = String::new();
        let applied = self.entries[..self.cursor]
            .iter()
            .filter(|r| r.is_document())
            .count();
        text.push_str(&applied.to_string());
        for r in self.entries.iter().filter(|r| r.is_document()) {
            text.push('\n');
            text.push_str(r.digest());
            text.push_str(&pm_model::json::to_canonical_string(&Value::from(
                r.label.as_str(),
            )));
        }
        pm_model::hash::sha256_hex(text.as_bytes())
    }

    /// A copy for a save on another thread (records are shared, not copied).
    /// Whether both save as the same `to_saved()` form, compared record by
    /// record without building it. A false answer may still mean equal saves
    /// (an entry kept as operations on one side), never the reverse.
    pub fn saves_as(&self, other: &History) -> bool {
        fn documents(records: &[HistoryRecord]) -> impl Iterator<Item = &HistoryRecord> {
            records.iter().filter(|r| r.is_document())
        }
        documents(&self.entries).count() == documents(&other.entries).count()
            && documents(&self.entries[..self.cursor]).count()
                == documents(&other.entries[..other.cursor]).count()
            && documents(&self.entries)
                .zip(documents(&other.entries))
                .all(|(a, b)| {
                    a.label == b.label
                        && ((a.forward == b.forward && a.inverse == b.inverse)
                            || a.saved_form() == b.saved_form())
                })
    }
    /// Whether `self`'s document steps are `saved`'s, consecutive runs of
    /// them merged into one step as a squash merges them (operations
    /// concatenated in order), with nothing to redo on either side. Every
    /// undo state of `self` is then one of `saved`'s.
    pub fn regroups(&self, saved: &History) -> bool {
        fn documents(records: &[HistoryRecord]) -> Vec<&HistoryRecord> {
            records.iter().filter(|r| r.is_document()).collect()
        }
        fn merged(run: &[&HistoryRecord], step: &HistoryRecord) -> bool {
            let forward: Vec<Op> = run.iter().flat_map(|r| r.forward.iter()).collect();
            let inverse: Vec<Op> = run.iter().rev().flat_map(|r| r.inverse.iter()).collect();
            if step.forward.to_vec() == forward && step.inverse.to_vec() == inverse {
                return true;
            }
            let forms: Option<Vec<_>> = run.iter().map(|r| r.saved_form()).collect();
            forms.zip(step.saved_form()).is_some_and(|(forms, (f, b))| {
                let fwd: Vec<_> = forms.iter().flat_map(|(f, _)| f.iter().cloned()).collect();
                let bwd: Vec<_> = forms
                    .iter()
                    .rev()
                    .flat_map(|(_, b)| b.iter().cloned())
                    .collect();
                f == fwd && b == bwd
            })
        }
        if self.can_redo() || saved.can_redo() {
            return false;
        }
        let (mine, theirs) = (documents(&self.entries), documents(&saved.entries));
        let mut next = 0;
        for step in mine {
            let start = next;
            loop {
                if next == theirs.len() {
                    return false;
                }
                next += 1;
                if merged(&theirs[start..next], step) {
                    break;
                }
            }
        }
        next == theirs.len()
    }
    /// Saves of this history whose file guard replayed the whole history.
    #[cfg(test)]
    pub(crate) fn full_replays(&self) -> u64 {
        self.guard_states.lock().expect("guard states").full_replays
    }
    pub fn snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            records: self.entries.clone(),
            cursor: self.cursor,
            base_token: self.base_token,
            guard_states: self.guard_states.clone(),
        }
    }
}

// ── the saved format ────────────────────────────────────────────────────────

const DICTIONARY_FORMAT: &str = "powermove-history-dictionary";

fn forbidden(key: &JsString) -> bool {
    matches!(
        key.as_str(),
        Some("__proto__" | "prototype" | "constructor")
    )
}

/// `validPatches` for one path: string parts other than the prototype keys,
/// or safe non-negative integers.
fn valid_path(path: &[PathPart]) -> bool {
    path.iter().all(|part| match part {
        PathPart::Key(k) => !forbidden(k),
        PathPart::Index(n) => n.fract() == 0.0 && *n >= 0.0 && *n <= 9_007_199_254_740_991.0,
    })
}

/// `isHistoryRecordArrayPath`.
fn is_record_array_path(path: &[Value]) -> bool {
    matches!(
        path.last().and_then(Value::as_str),
        Some("layers" | "edits")
    ) && (path.len() == 1
        || path.len() == 3
            && path[0].as_str() == Some("comps")
            && path[1].as_str().is_some_and(|s| !s.is_empty()))
}

/// `unpackHistory`. `None` when the dictionary is malformed.
pub(crate) fn unpack(value: &Value) -> Option<Value> {
    let Some(map) = value.as_object() else {
        return Some(value.clone());
    };
    if map.get("format").and_then(Value::as_str) != Some(DICTIONARY_FORMAT) {
        return Some(value.clone());
    }
    if map.get("version") != Some(&Value::Number(1.0)) {
        return None;
    }
    let records = map.get("records")?.as_array()?;
    let history = map.get("history")?.as_object()?;
    let entries = history.get("entries")?.as_array()?;
    let unpack_patch = |item: &Value| -> Option<Value> {
        let item = item.as_object()?;
        let patch = item.get("patch")?.as_object()?;
        let Some(refs) = item.get("refs") else {
            return Some(Value::Object(patch.clone()));
        };
        let path = patch.get("path")?.as_array()?;
        if !is_record_array_path(path) || patch.get("exists") != Some(&Value::Bool(true)) {
            return None;
        }
        let values: Option<Vec<Value>> = refs
            .as_array()?
            .iter()
            .map(|r| {
                let n = r.as_f64()?;
                (n.fract() == 0.0 && n >= 0.0).then_some(())?;
                records.get(n as usize).cloned()
            })
            .collect();
        let mut out = patch.clone();
        out.insert("value", Value::Array(values?));
        Some(Value::Object(out))
    };
    let mut new_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut e = entry.as_object()?.clone();
        for side in ["forward", "backward"] {
            let list: Option<Vec<Value>> =
                e.get(side)?.as_array()?.iter().map(unpack_patch).collect();
            e.insert(side, Value::Array(list?));
        }
        new_entries.push(Value::Object(e));
    }
    let mut out = history.clone();
    out.insert("entries", Value::Array(new_entries));
    Some(Value::Object(out))
}

impl History {
    /// `PM.hist.import(saved)`: the envelope's `history` member. `None` for
    /// anything the old app would also refuse (not version 1, too many
    /// entries, a bad index, an invalid patch). Histories saved before
    /// compositions (`paths` other than `"composition"`) are canonicalized
    /// against `open_comp`, as the old app did.
    pub fn from_saved(saved: &Value, open_comp: Option<&str>) -> Option<History> {
        History::import(saved, open_comp).ok()
    }

    /// [`from_saved`](Self::from_saved) with the reason for a refusal
    /// (generic wording; never quotes the history's content).
    pub fn import(saved: &Value, open_comp: Option<&str>) -> Result<History, String> {
        // Plain histories can be read in place. `unpack` clones its input;
        // doing that here used to copy a complete large JSON tree on open.
        let unpacked;
        let source = if saved.get("format").and_then(Value::as_str) == Some(DICTIONARY_FORMAT) {
            unpacked = unpack(saved).ok_or("invalid history dictionary")?;
            &unpacked
        } else {
            saved
        };
        let map = source.as_object().ok_or("history is not an object")?;
        if map.get("version") != Some(&Value::Number(1.0)) {
            return Err("history version is not 1".into());
        }
        let entries = map
            .get("entries")
            .and_then(Value::as_array)
            .ok_or("history has no entries list")?;
        if entries.len() > MAX_ENTRIES {
            return Err(format!("history has more than {MAX_ENTRIES} entries"));
        }
        let index = map
            .get("index")
            .and_then(Value::as_f64)
            .ok_or("history has no index")?;
        if index.fract() != 0.0 || index < -1.0 || index >= entries.len() as f64 {
            return Err("history index is out of range".into());
        }
        let root_relative = map.get("paths").and_then(Value::as_str) != Some("composition");
        // Entries parse and are measured on several threads (measuring
        // serializes every patch: most of a large history's import, on the
        // path to a project's first frame); records are made in order.
        let parsed = pm_model::parallel::map(entries.len(), |i| {
            import_entry(&entries[i], root_relative, open_comp)
        });
        let mut history = History::new();
        for entry in parsed {
            let (label, forward, backward, bytes) = entry?;
            history.entries.push(HistoryRecord {
                id: next_token(),
                label,
                origin: Origin::Interface,
                forward,
                inverse: backward,
                saved: None,
                revision: 0,
                bytes,
                external: None,
                selection: None,
                digest: Default::default(),
                assets: Default::default(),
                saved_text: Default::default(),
            });
            history.total_bytes += bytes;
        }
        history.cursor = (index + 1.0) as usize;
        // Reopening under a smaller budget drops distant redo first.
        while history.total_bytes > history.max_bytes && history.entries.len() > history.cursor {
            let removed = history.entries.pop().expect("non-empty");
            history.total_bytes -= removed.bytes;
        }
        history.trim();
        Ok(history)
    }
}

/// How many entries `History::import` keeps of a saved history with these
/// imported record sizes and `applied` applied: the same budget steps as
/// import (distant redo first, then the oldest), on sizes alone.
pub(crate) fn import_kept(bytes: &[usize], applied: usize) -> usize {
    let (max_entries, max_bytes) = (MAX_ENTRIES, DEFAULT_MAX_BYTES);
    let mut len = bytes.len();
    let mut total: usize = bytes.iter().sum();
    while total > max_bytes && len > applied {
        len -= 1;
        total -= bytes[len];
    }
    let mut first = 0;
    while len - first > max_entries || total > max_bytes && len - first > 1 {
        total -= bytes[first];
        first += 1;
    }
    len - first
}

/// One saved history entry: its label, forward and backward operations
/// (canonicalized against `open_comp` when saved before compositions), and
/// their bytes.
/// The asset records a saved patch at `path` restores, as opening reads
/// them: an Undo or Redo of an edit made before (a v1 project's history)
/// never brings a v1 video asset back without its v1 video reading
/// (`videoColor`, `project_io::adopt_electron_asset`). `None` when the
/// patch holds no asset record.
fn restored_assets(path: &[PathPart], value: &Value) -> Option<Value> {
    let key = |part: &PathPart, name: &str| matches!(part, PathPart::Key(k) if k == name);
    // Only these paths hold asset records; nothing else is copied.
    let shape = match path {
        [] => 0,
        [assets] if key(assets, "assets") => 1,
        [assets, _] if key(assets, "assets") => 2,
        _ => return None,
    };
    let mut value = value.clone();
    match shape {
        0 => crate::project_io::adopt_electron_rendering(&mut value, false),
        1 => {
            for (_, asset) in value.as_object_mut()?.iter_mut() {
                crate::project_io::adopt_electron_asset(asset, false);
            }
        }
        _ => crate::project_io::adopt_electron_asset(&mut value, false),
    }
    Some(value)
}

fn import_entry(
    entry: &Value,
    root_relative: bool,
    open_comp: Option<&str>,
) -> Result<(String, StoredOps, StoredOps, usize), String> {
    let e = entry
        .as_object()
        .ok_or("a history entry is not an object")?;
    let label = e
        .get("label")
        .and_then(Value::as_str)
        .ok_or("a history entry has no label")?
        .to_owned();
    if !root_relative && !needs_exact(entry) {
        let operations = |side: &str| -> Result<StoredOps, String> {
            let items = e
                .get(side)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("a history entry has no {side} list"))?;
            let mut text = String::from("[");
            for (i, item) in items.iter().enumerate() {
                let source = item
                    .as_object()
                    .ok_or_else(|| format!("a {side} patch is malformed"))?;
                let mut shape = Map::new();
                for (key, value) in source.iter() {
                    shape.insert(
                        key.clone(),
                        if key.as_str() == Some("value") {
                            Value::Null
                        } else {
                            value.clone()
                        },
                    );
                }
                let op = Op::from_value(&Value::Object(shape))
                    .map_err(|_| format!("a {side} patch is malformed"))?;
                if !valid_path(op.path()) {
                    return Err(format!("a {side} patch has an invalid path"));
                }
                let restored = source
                    .get("value")
                    .and_then(|v| restored_assets(op.path(), v));
                let normalized = op.to_value();
                let mut members: Vec<_> = normalized.as_object().unwrap().iter().collect();
                members.sort_by_key(|(a, _)| *a);
                if i > 0 {
                    text.push(',');
                }
                text.push('{');
                for (j, (key, value)) in members.into_iter().enumerate() {
                    if j > 0 {
                        text.push(',');
                    }
                    pm_model::json::write_canonical(&Value::String(key.clone()), &mut text);
                    text.push(':');
                    let value = if key.as_str() == Some("value") {
                        restored.as_ref().or(source.get("value")).unwrap_or(value)
                    } else {
                        value
                    };
                    pm_model::json::write_canonical(value, &mut text);
                }
                text.push('}');
            }
            text.push(']');
            Ok(StoredOps(CompactJson::text(text)))
        };
        let (forward, backward) = (operations("forward")?, operations("backward")?);
        let bytes = forward.0.len + backward.0.len;
        return Ok((label, forward, backward, bytes));
    }
    let operations = |side: &str| -> Result<Vec<Op>, String> {
        e.get(side)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("a history entry has no {side} list"))?
            .iter()
            .map(|p| {
                let op = Op::from_value(p).map_err(|_| format!("a {side} patch is malformed"))?;
                if valid_path(op.path()) {
                    Ok(op)
                } else {
                    Err(format!("a {side} patch has an invalid path"))
                }
            })
            .collect()
    };
    let (mut forward, mut backward) = (operations("forward")?, operations("backward")?);
    if root_relative {
        let leaves = |ops: &[Op]| {
            ops.iter()
                .map(|op| match op {
                    Op::Leaf(patch) => Ok(patch.clone()),
                    _ => Err("a history entry needs its composition paths"),
                })
                .collect::<Result<Vec<_>, _>>()
        };
        let (f, b) = canonicalize_patches(&leaves(&forward)?, &leaves(&backward)?, open_comp);
        forward = f.into_iter().map(Op::Leaf).collect();
        backward = b.into_iter().map(Op::Leaf).collect();
    }
    for op in forward.iter_mut().chain(backward.iter_mut()) {
        if let Op::Leaf(patch) = op
            && let Some(value) = patch.value.as_ref()
            && let Some(restored) = restored_assets(&patch.path, value)
        {
            patch.value = Some(restored);
        }
    }
    let forward = StoredOps::from(forward);
    let backward = StoredOps::from(backward);
    let bytes = forward.0.len + backward.0.len;
    Ok((label, forward, backward, bytes))
}

impl HistorySnapshot {
    /// `PM.hist.export()`: `{ version: 1, paths: "composition", index,
    /// entries: [{ label, forward, backward }] }`.
    pub fn to_saved(&self) -> Value {
        let mut out = Map::new();
        out.insert("version", Value::Number(1.0));
        out.insert("paths", Value::from("composition"));
        // Document entries only; the index counts applied document entries
        // (`export()` filtered closure entries the same way).
        let applied = self.records[..self.cursor]
            .iter()
            .filter(|r| r.is_document())
            .count();
        out.insert("index", Value::Number(applied as f64 - 1.0));
        let entries = self
            .records
            .iter()
            .filter(|r| r.is_document())
            .map(|r| {
                let list = |p: Vec<Patch>| Value::Array(p.iter().map(Patch::to_value).collect());
                let (forward, backward) = match r.saved_form() {
                    Some((forward, backward)) => (list(forward), list(backward)),
                    None => (r.forward.value(), r.inverse.value()),
                };
                let mut e = Map::new();
                e.insert("label", Value::from(r.label.as_str()));
                e.insert("forward", forward);
                e.insert("backward", backward);
                Value::Object(e)
            })
            .collect();
        out.insert("entries", Value::Array(entries));
        Value::Object(out)
    }

    pub fn records(&self) -> &[HistoryRecord] {
        &self.records
    }

    /// Writes exactly `write_canonical(&self.to_saved())`, each step from
    /// the entry text it made once ([`HistoryRecord::saved_text`]) rather
    /// than rebuilding the whole history as values on every save.
    pub fn write_saved<S: pm_model::json::write::Sink>(&self, sink: &mut S) {
        use pm_model::json::write::write_canonical;
        let applied = self.records[..self.cursor]
            .iter()
            .filter(|r| r.is_document())
            .count();
        // Keys in canonical order: entries, index, paths, version.
        sink.push_str("{\"entries\":[");
        for (i, r) in self.records.iter().filter(|r| r.is_document()).enumerate() {
            if i > 0 {
                sink.push_str(",");
            }
            sink.push_str(&r.saved_text().text());
            sink.push_str(",\"label\":");
            write_canonical(&Value::from(r.label.as_str()), sink);
            sink.push_str("}");
        }
        sink.push_str("],\"index\":");
        write_canonical(&Value::Number(applied as f64 - 1.0), sink);
        sink.push_str(",\"paths\":");
        write_canonical(&Value::from("composition"), sink);
        sink.push_str(",\"version\":");
        write_canonical(&Value::Number(1.0), sink);
        sink.push_str("}");
    }

    /// The document records in order, the base state's token and how many
    /// are applied: the states the file guard reconstructs. `None` when a
    /// realm's step is on the stack: it changes the document outside the
    /// saved history, so the states around it are not what replaying the
    /// saved steps makes.
    pub(crate) fn guard_chain(&self) -> Option<(Vec<&HistoryRecord>, u64, usize)> {
        if self.records.iter().any(|r| r.external.is_some()) {
            return None;
        }
        let documents: Vec<&HistoryRecord> =
            self.records.iter().filter(|r| r.is_document()).collect();
        let applied = self.records[..self.cursor]
            .iter()
            .filter(|r| r.is_document())
            .count();
        Some((documents, self.base_token, applied))
    }
    pub(crate) fn guard_states(&self) -> &GuardStates {
        &self.guard_states
    }

    /// `packHistory(export())`: the dictionary form `projectHistory.<id>`
    /// stores. Equal layer/edit records are stored once (the old packer
    /// shared them by object identity after interning by content; equal
    /// content gives the same dictionary). Returns the plain form when
    /// nothing is packable, as the old packer did.
    pub fn to_saved_packed(&self) -> Value {
        pack(self.to_saved())
    }
}

/// `packHistory`.
pub fn pack(saved: Value) -> Value {
    use std::collections::HashMap;
    let Some(map) = saved.as_object() else {
        return saved;
    };
    let Some(entries) = map.get("entries").and_then(Value::as_array) else {
        return saved;
    };
    let mut records: Vec<Value> = Vec::new();
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut pack_patch = |patch: &Value| -> Value {
        let mut out = Map::new();
        let record_array = patch.as_object().is_some_and(|p| {
            p.get("exists") == Some(&Value::Bool(true))
                && p.get("path")
                    .and_then(Value::as_array)
                    .is_some_and(|path| is_record_array_path(path))
                && p.get("value").and_then(Value::as_array).is_some()
        });
        if !record_array {
            out.insert("patch", patch.clone());
            return Value::Object(out);
        }
        let mut meta = patch.as_object().expect("checked").clone();
        let values = meta.remove("value").expect("checked");
        let refs = values
            .as_array()
            .expect("checked")
            .iter()
            .map(|item| {
                let key = pm_model::json::to_canonical_string(item);
                let n = *ids.entry(key).or_insert_with(|| {
                    records.push(item.clone());
                    records.len() - 1
                });
                Value::Number(n as f64)
            })
            .collect();
        out.insert("patch", Value::Object(meta));
        out.insert("refs", Value::Array(refs));
        Value::Object(out)
    };
    let mut packed_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(e) = entry.as_object() else {
            return saved;
        };
        let mut e = e.clone();
        for side in ["forward", "backward"] {
            let Some(list) = e.get(side).and_then(Value::as_array) else {
                return saved;
            };
            let list: Vec<Value> = list.iter().map(&mut pack_patch).collect();
            e.insert(side, Value::Array(list));
        }
        packed_entries.push(Value::Object(e));
    }
    if records.is_empty() {
        return saved;
    }
    let mut history = map.clone();
    history.insert("entries", Value::Array(packed_entries));
    let mut out = Map::new();
    out.insert("format", Value::from(DICTIONARY_FORMAT));
    out.insert("version", Value::Number(1.0));
    out.insert("records", Value::Array(records));
    out.insert("history", Value::Object(history));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::parse;

    /// A v1 project's history whose last edit deleted a v1 video asset: an
    /// Undo restores the asset as opening reads it, with the v1 video
    /// reading (`videoColor`), in both saved path forms, and Redo/Undo keep
    /// it; an asset a native import made (`timing`) comes back untouched.
    #[test]
    fn an_undo_restores_a_v1_video_asset_with_its_v1_reading() {
        let project = parse(
            r##"{"id":"P","compId":"C","compName":"Main","w":64,"h":36,"fps":30,"dur":2,
            "bg":"#000000","layers":[],"comps":{},"assets":{"still":{"id":"still","kind":"image"}}}"##,
        )
        .unwrap();
        let asset = |id: &str, timing: bool| {
            let timing = if timing { r#","timing":2"# } else { "" };
            format!(r#"{{"id":"{id}","kind":"video","name":"{id}.mov","dur":4{timing}}}"#)
        };
        for paths in [r#""paths":[]"#, r#""paths":"composition""#] {
            let saved = parse(&format!(
                r#"{{"version":1,{paths},"index":0,"entries":[{{"label":"Delete Media",
                "forward":[{{"path":["assets","v1"],"exists":false}},{{"path":["assets","new"],"exists":false}}],
                "backward":[{{"path":["assets","v1"],"exists":true,"value":{}}},
                            {{"path":["assets","new"],"exists":true,"value":{}}}]}}]}}"#,
                asset("v1", false),
                asset("new", true),
            ))
            .unwrap();
            let history = History::import(&saved, Some("C")).unwrap();
            let mut store = crate::DocumentStore::adopt(
                pm_model::Document::new(Some(crate::model::to_canonical(project.clone())), 1),
                history,
            );
            let reading = |store: &crate::DocumentStore, id: &str| {
                let doc = store.snapshot().document.to_value().unwrap();
                doc.get("assets").and_then(|a| a.get(id)).map(|a| {
                    a.get("videoColor")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
            };
            store.undo().unwrap();
            assert_eq!(
                reading(&store, "v1"),
                Some(Some("electron".into())),
                "{paths}"
            );
            assert_eq!(reading(&store, "new"), Some(None), "{paths}");
            store.redo().unwrap();
            assert_eq!(reading(&store, "v1"), None);
            store.undo().unwrap();
            assert_eq!(
                reading(&store, "v1"),
                Some(Some("electron".into())),
                "{paths}"
            );
        }
    }

    fn record(h: &mut History, label: &str) -> u64 {
        let p = Patch::set(vec![PathPart::from("x")], Value::Number(1.0));
        h.push(
            label.into(),
            Origin::Interface,
            vec![Op::Leaf(p.clone())],
            vec![Op::Leaf(p)],
            None,
            1,
        )
    }

    #[test]
    fn compact_records_replay_squash_trim_and_save_identically() {
        let mut h = History::new();
        let mark = h.mark();
        for i in 1..=4 {
            let patch = |n| {
                Op::Leaf(Patch {
                    path: vec![PathPart::Key("x".into())],
                    exists: true,
                    value: Some(Value::Number(n as f64)),
                })
            };
            h.push(
                format!("Step {i}"),
                Origin::Interface,
                vec![patch(i)],
                vec![patch(i - 1)],
                None,
                i as u64,
            );
        }
        let saved = h.snapshot().to_saved();
        let identity = h.identity();
        let mut h = History::import(&saved, None).unwrap();
        assert_eq!(identity, h.identity());
        assert_eq!(
            pm_model::json::to_canonical_string(&saved),
            pm_model::json::to_canonical_string(&h.snapshot().to_saved())
        );
        let mut doc = pm_model::Document::new(Some(parse(r#"{"x":4}"#).unwrap()), 1);
        for expected in (0..4).rev() {
            doc.apply_ops_at(&h.peek_undo().unwrap().inverse.to_vec(), doc.revision() + 1)
                .unwrap();
            h.mark_undone();
            assert_eq!(
                doc.root().unwrap().to_value().get("x"),
                Some(&Value::Number(expected as f64))
            );
        }
        for expected in 1..=4 {
            doc.apply_ops_at(&h.peek_redo().unwrap().forward.to_vec(), doc.revision() + 1)
                .unwrap();
            h.mark_redone();
            assert_eq!(
                doc.root().unwrap().to_value().get("x"),
                Some(&Value::Number(expected as f64))
            );
        }
        h.squash_since(&mark, "Combined").unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h.peek_undo().unwrap().inverse.len(), 4);
        let roundtrip = History::import(&h.snapshot().to_saved_packed(), None).unwrap();
        assert_eq!(h.identity(), roundtrip.identity());
        h.set_max_bytes(0);
        assert_eq!(h.len(), 1, "latest oversized edit stays");
    }

    #[test]
    fn compact_operations_preserve_runtime_number_bits() {
        for number in [-0.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            let ops = StoredOps::from(vec![Op::Leaf(Patch {
                path: vec![],
                exists: true,
                value: Some(Value::Number(number)),
            })]);
            let Op::Leaf(patch) = ops.to_vec().remove(0) else {
                panic!()
            };
            assert_eq!(
                patch.value.unwrap().as_f64().unwrap().to_bits(),
                number.to_bits()
            );
        }
    }

    #[test]
    fn push_discards_redo_and_tokens_follow_the_cursor() {
        let mut h = History::new();
        let base = h.state_token();
        let a = record(&mut h, "a");
        let b = record(&mut h, "b");
        assert_eq!(h.state_token(), b);
        h.mark_undone();
        assert_eq!(h.state_token(), a);
        h.mark_undone();
        assert_eq!(h.state_token(), base);
        h.mark_redone();
        let c = record(&mut h, "c");
        assert_eq!(h.labels(), ["a", "c"]);
        assert_eq!(h.state_token(), c);
    }

    #[test]
    fn trimming_the_bottom_moves_the_base_token() {
        let mut h = History::with_limits(2, usize::MAX);
        let a = record(&mut h, "a");
        record(&mut h, "b");
        record(&mut h, "c");
        assert_eq!(h.labels(), ["b", "c"]);
        h.mark_undone();
        h.mark_undone();
        // Undoing everything that is left reaches the state after `a`.
        assert_eq!(h.state_token(), a);
    }

    #[test]
    fn byte_budget_drops_redo_first() {
        let mut h = History::new();
        record(&mut h, "a");
        record(&mut h, "b");
        record(&mut h, "c");
        h.mark_undone();
        let one = h.records()[0].bytes;
        h.set_max_bytes(one * 2);
        assert_eq!(h.labels(), ["a", "b"]);
        assert_eq!(h.applied(), 2);
        h.set_max_bytes(one);
        assert_eq!(h.labels(), ["b"]);
    }

    #[test]
    fn imports_and_exports_the_old_format() {
        let text = r#"{"entries":[{"backward":[{"exists":true,"path":["comps","C1","layers",0,"name"],"value":"a"}],"forward":[{"exists":true,"path":["comps","C1","layers",0,"name"],"value":"b"}],"label":"Rename Layer"}],"index":0,"paths":"composition","version":1}"#;
        let saved = parse(text).unwrap();
        let h = History::from_saved(&saved, Some("C1")).unwrap();
        assert_eq!(h.labels(), ["Rename Layer"]);
        assert_eq!(h.applied(), 1);
        assert_eq!(
            pm_model::json::to_canonical_string(&h.snapshot().to_saved()),
            text
        );
    }

    /// Entries parse on several threads but become records in their saved
    /// order, and a refusal names the first bad entry, as one by one.
    #[test]
    fn many_entries_import_in_order_and_refuse_at_the_first_bad_one() {
        let entry = |i: usize| {
            format!(
                r#"{{"backward":[{{"exists":true,"path":["comps","C1","layers",0,"name"],"value":"a{i}"}}],"forward":[{{"exists":true,"path":["comps","C1","layers",0,"name"],"value":"b{i}"}}],"label":"Step {i}"}}"#
            )
        };
        let entries: Vec<String> = (0..64).map(entry).collect();
        let text = format!(
            r#"{{"entries":[{}],"index":63,"paths":"composition","version":1}}"#,
            entries.join(",")
        );
        let h = History::from_saved(&parse(&text).unwrap(), Some("C1")).unwrap();
        let labels: Vec<String> = (0..64).map(|i| format!("Step {i}")).collect();
        assert_eq!(h.labels(), labels);
        assert!(h.records().windows(2).all(|w| w[0].id < w[1].id));
        assert_eq!(
            pm_model::json::to_canonical_string(&h.snapshot().to_saved()),
            text
        );
        let mut bad = entries.clone();
        bad[40] = r#"{"label":"x","forward":[{"path":[1.5],"exists":true}],"backward":[]}"#.into();
        bad[50] = r#"{"label":3,"forward":[],"backward":[]}"#.into();
        let text = format!(
            r#"{{"entries":[{}],"index":63,"paths":"composition","version":1}}"#,
            bad.join(",")
        );
        assert_eq!(
            History::import(&parse(&text).unwrap(), Some("C1")).unwrap_err(),
            "a forward patch has an invalid path"
        );
    }

    #[test]
    fn canonicalizes_root_relative_histories() {
        let text = r#"{"entries":[{"backward":[{"exists":true,"path":["layers",0,"name"],"value":"a"}],"forward":[{"exists":true,"path":["layers",0,"name"],"value":"b"}],"label":"x"}],"index":0,"version":1}"#;
        let h = History::from_saved(&parse(text).unwrap(), Some("C1")).unwrap();
        let saved = pm_model::json::to_canonical_string(&h.snapshot().to_saved());
        assert!(
            saved.contains(r#""path":["comps","C1","layers",0,"name"]"#),
            "{saved}"
        );
    }

    #[test]
    fn refuses_what_the_old_app_refused() {
        for text in [
            r#"{"entries":[],"index":0}"#,
            r#"{"entries":[],"index":0,"version":1}"#,
            r#"{"entries":[{"label":"x","forward":[{"path":["__proto__"],"exists":true}],"backward":[]}],"index":0,"version":1}"#,
            r#"{"entries":[{"label":"x","forward":[{"path":[1.5],"exists":true}],"backward":[]}],"index":0,"version":1}"#,
            r#"{"entries":[{"label":3,"forward":[],"backward":[]}],"index":0,"version":1}"#,
        ] {
            assert!(
                History::from_saved(&parse(text).unwrap(), None).is_none(),
                "{text}"
            );
        }
        assert!(
            History::from_saved(
                &parse(r#"{"entries":[],"index":-1,"version":1}"#).unwrap(),
                None
            )
            .is_some()
        );
    }

    #[test]
    fn unpacks_dictionary_histories() {
        let text = r#"{"format":"powermove-history-dictionary","version":1,"records":[{"id":"a"}],"history":{"version":1,"paths":"composition","index":0,"entries":[{"label":"x","forward":[{"patch":{"path":["comps","C1","layers"],"exists":true},"refs":[0]}],"backward":[{"patch":{"path":["comps","C1","layers"],"exists":true,"value":[]}}]}]}}"#;
        let h = History::from_saved(&parse(text).unwrap(), Some("C1")).unwrap();
        let saved = pm_model::json::to_canonical_string(&h.snapshot().to_saved());
        assert!(saved.contains(r#""value":[{"id":"a"}]"#), "{saved}");
    }
}
