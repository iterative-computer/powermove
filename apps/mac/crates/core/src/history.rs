//! Undo/redo records.
//!
//! One [`HistoryRecord`] per committed transaction or finished gesture. A
//! record holds the *forward* operations and the *inverse* operations
//! captured when it was committed. Undo applies the inverse as a **new,
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
//! | "Selection" entries while at the top of the stack | Not ported: selection is not history (see `docs/gpui/core-api.md`) |
//! | `external` closure entries (media replacement) | Not ported yet |
//! | Nested `begin`/`commit` depth | Not ported: [`transact`](crate::DocumentStore::transact) takes the whole command list |

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
    pub forward: Arc<[Op]>,
    /// Operations that undo the step from the state right after it.
    pub inverse: Arc<[Op]>,
    /// Leaf-patch form (`forward`, `backward`) for the saved file, present
    /// when `forward`/`inverse` contain structural operations.
    pub(crate) saved: Option<Arc<(Vec<Patch>, Vec<Patch>)>>,
    /// The document revision the step was committed at (0 when imported).
    pub revision: u64,
    /// Serialized size, for the byte budget.
    pub bytes: usize,
}

impl HistoryRecord {
    /// The patches the file stores for this record (`forward`, `backward`).
    pub(crate) fn saved_patches(&self) -> (Vec<Patch>, Vec<Patch>) {
        if let Some(saved) = &self.saved {
            return (saved.0.clone(), saved.1.clone());
        }
        let leaves = |ops: &[Op]| {
            ops.iter()
                .filter_map(|op| match op {
                    Op::Leaf(p) => Some(p.clone()),
                    _ => None,
                })
                .collect()
        };
        (leaves(&self.forward), leaves(&self.inverse))
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
        self.peek_undo().map_or(self.base_token, |r| r.id)
    }

    /// Lowers (or raises) the byte budget and trims to it (`Memory.trim`):
    /// distant redo first, then the oldest steps; the newest step stays.
    pub fn set_max_bytes(&mut self, max_bytes: usize) {
        self.max_bytes = max_bytes;
        while self.total_bytes > self.max_bytes && self.entries.len() > 1 {
            if self.cursor < self.entries.len() {
                let removed = self.entries.pop().expect("non-empty");
                self.total_bytes -= removed.bytes;
            } else {
                self.drop_oldest();
            }
        }
    }

    fn drop_oldest(&mut self) {
        let removed = self.entries.remove(0);
        self.total_bytes -= removed.bytes;
        if self.cursor > 0 {
            self.cursor -= 1;
            // "No record applied" now means "after the removed one".
            self.base_token = removed.id;
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
        for removed in self.entries.drain(self.cursor..) {
            self.total_bytes -= removed.bytes;
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
            Some((f, b)) => patches_bytes(f) + patches_bytes(b) + ops_bytes(&forward) + ops_bytes(&inverse),
            None => ops_bytes(&forward) + ops_bytes(&inverse),
        };
        let id = next_token();
        self.entries.push(HistoryRecord {
            id,
            label,
            origin,
            forward: forward.into(),
            inverse: inverse.into(),
            saved: saved.map(Arc::new),
            revision,
            bytes,
        });
        self.total_bytes += bytes;
        self.cursor = self.entries.len();
        self.trim();
        id
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
        let tail: Vec<HistoryRecord> = self.entries.drain(base..).collect();
        for r in &tail {
            self.total_bytes -= r.bytes;
        }
        self.cursor = self.entries.len();
        let forward: Vec<Op> = tail.iter().flat_map(|r| r.forward.iter().cloned()).collect();
        let inverse: Vec<Op> = tail.iter().rev().flat_map(|r| r.inverse.iter().cloned()).collect();
        let saved = tail.iter().any(|r| r.saved.is_some()).then(|| {
            let mut f = Vec::new();
            let mut b = Vec::new();
            for r in &tail {
                f.extend(r.saved_patches().0);
            }
            for r in tail.iter().rev() {
                b.extend(r.saved_patches().1);
            }
            (f, b)
        });
        let revision = tail.last().map_or(0, |r| r.revision);
        Some(self.push(label.to_owned(), tail[0].origin, forward, inverse, saved, revision))
    }

    /// A copy for a save on another thread (records are shared, not copied).
    pub fn snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            records: self.entries.clone(),
            cursor: self.cursor,
        }
    }
}

// ── the saved format ────────────────────────────────────────────────────────

const DICTIONARY_FORMAT: &str = "powermove-history-dictionary";

fn forbidden(key: &JsString) -> bool {
    matches!(key.as_str(), Some("__proto__" | "prototype" | "constructor"))
}

/// `validPatches` for one patch: string parts other than the prototype keys,
/// or safe non-negative integers.
fn valid_patch(patch: &Patch) -> bool {
    patch.path.iter().all(|part| match part {
        PathPart::Key(k) => !forbidden(k),
        PathPart::Index(n) => n.fract() == 0.0 && *n >= 0.0 && *n <= 9_007_199_254_740_991.0,
    })
}

/// `isHistoryRecordArrayPath`.
fn is_record_array_path(path: &[Value]) -> bool {
    matches!(path.last().and_then(Value::as_str), Some("layers" | "edits"))
        && (path.len() == 1
            || path.len() == 3
                && path[0].as_str() == Some("comps")
                && path[1].as_str().is_some_and(|s| !s.is_empty()))
}

/// `unpackHistory`. `None` when the dictionary is malformed.
fn unpack(value: &Value) -> Option<Value> {
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
        let unpacked = unpack(saved)?;
        let map = unpacked.as_object()?;
        if map.get("version") != Some(&Value::Number(1.0)) {
            return None;
        }
        let entries = map.get("entries")?.as_array()?;
        if entries.len() > MAX_ENTRIES {
            return None;
        }
        let index = map.get("index")?.as_f64()?;
        if index.fract() != 0.0 || index < -1.0 || index >= entries.len() as f64 {
            return None;
        }
        let root_relative = map.get("paths").and_then(Value::as_str) != Some("composition");
        let mut history = History::new();
        for entry in entries {
            let e = entry.as_object()?;
            let label = e.get("label")?.as_str()?.to_owned();
            let patches = |side: &str| -> Option<Vec<Patch>> {
                e.get(side)?
                    .as_array()?
                    .iter()
                    .map(|p| {
                        let patch = Patch::from_value(p).ok()?;
                        valid_patch(&patch).then_some(patch)
                    })
                    .collect()
            };
            let (mut forward, mut backward) = (patches("forward")?, patches("backward")?);
            if root_relative {
                (forward, backward) = canonicalize_patches(&forward, &backward, open_comp);
            }
            let bytes = patches_bytes(&forward) + patches_bytes(&backward);
            history.entries.push(HistoryRecord {
                id: next_token(),
                label,
                origin: Origin::Interface,
                forward: forward.into_iter().map(Op::Leaf).collect::<Vec<_>>().into(),
                inverse: backward.into_iter().map(Op::Leaf).collect::<Vec<_>>().into(),
                saved: None,
                revision: 0,
                bytes,
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
        Some(history)
    }
}

impl HistorySnapshot {
    /// `PM.hist.export()`: `{ version: 1, paths: "composition", index,
    /// entries: [{ label, forward, backward }] }`.
    pub fn to_saved(&self) -> Value {
        let mut out = Map::new();
        out.insert("version", Value::Number(1.0));
        out.insert("paths", Value::from("composition"));
        out.insert("index", Value::Number(self.cursor as f64 - 1.0));
        let entries = self
            .records
            .iter()
            .map(|r| {
                let (forward, backward) = r.saved_patches();
                let list = |p: Vec<Patch>| Value::Array(p.iter().map(Patch::to_value).collect());
                let mut e = Map::new();
                e.insert("label", Value::from(r.label.as_str()));
                e.insert("forward", list(forward));
                e.insert("backward", list(backward));
                Value::Object(e)
            })
            .collect();
        out.insert("entries", Value::Array(entries));
        Value::Object(out)
    }

    pub fn records(&self) -> &[HistoryRecord] {
        &self.records
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::parse;

    fn record(h: &mut History, label: &str) -> u64 {
        let p = Patch::set(vec![PathPart::from("x")], Value::Number(1.0));
        h.push(label.into(), Origin::Interface, vec![Op::Leaf(p.clone())], vec![Op::Leaf(p)], None, 1)
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
        assert_eq!(pm_model::json::to_canonical_string(&h.snapshot().to_saved()), text);
    }

    #[test]
    fn canonicalizes_root_relative_histories() {
        let text = r#"{"entries":[{"backward":[{"exists":true,"path":["layers",0,"name"],"value":"a"}],"forward":[{"exists":true,"path":["layers",0,"name"],"value":"b"}],"label":"x"}],"index":0,"version":1}"#;
        let h = History::from_saved(&parse(text).unwrap(), Some("C1")).unwrap();
        let saved = pm_model::json::to_canonical_string(&h.snapshot().to_saved());
        assert!(saved.contains(r#""path":["comps","C1","layers",0,"name"]"#), "{saved}");
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
            assert!(History::from_saved(&parse(text).unwrap(), None).is_none(), "{text}");
        }
        assert!(History::from_saved(&parse(r#"{"entries":[],"index":-1,"version":1}"#).unwrap(), None).is_some());
    }

    #[test]
    fn unpacks_dictionary_histories() {
        let text = r#"{"format":"powermove-history-dictionary","version":1,"records":[{"id":"a"}],"history":{"version":1,"paths":"composition","index":0,"entries":[{"label":"x","forward":[{"patch":{"path":["comps","C1","layers"],"exists":true},"refs":[0]}],"backward":[{"patch":{"path":["comps","C1","layers"],"exists":true,"value":[]}}]}]}}"#;
        let h = History::from_saved(&parse(text).unwrap(), Some("C1")).unwrap();
        let saved = pm_model::json::to_canonical_string(&h.snapshot().to_saved());
        assert!(saved.contains(r#""value":[{"id":"a"}]"#), "{saved}");
    }
}
