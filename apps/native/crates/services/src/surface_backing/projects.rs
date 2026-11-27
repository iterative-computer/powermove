//! S103/S104/S105. Tag: TakesPanel, NotesPanel, core/history, extensions/mods.
use super::{RecordHost, Records, Session, Ticket, id, invalid, replace, stale, ticket};
use crate::{Result, ServiceErrorCode};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const NOTES_DEBOUNCE_MS: u64 = 250;

#[derive(Clone, Debug, PartialEq)]
pub struct NoteWrite {
    pub ticket: Ticket,
    pub session: Session,
    pub expected: String,
    pub text: String,
}
/// Implement with a kernel session guard and expected-text comparison, updating
/// notes and autosave atomically. No Undo entry, matching the shipped notes editor.
pub trait NotesHost {
    fn persist(&mut self, write: &NoteWrite) -> Result<()>;
}
#[derive(Clone, Debug, PartialEq)]
pub struct NoteDraft {
    pub text: String,
    pub focused: bool,
    pub read_only: bool,
    pub failure: Option<ServiceErrorCode>,
    published: String,
    due: Option<u64>,
    inflight: Option<NoteWrite>,
}
#[derive(Default)]
pub struct Notes {
    pub current: Option<Session>,
    drafts: BTreeMap<Session, NoteDraft>,
}
impl Notes {
    /// Old session drafts remain available to flush/recover; switching never
    /// retargets them at the newly opened document (N4/N7).
    pub fn attach(&mut self, session: Session, published: String) {
        self.drafts
            .entry(session.clone())
            .or_insert_with(|| NoteDraft {
                text: published.clone(),
                published,
                focused: false,
                read_only: false,
                due: None,
                inflight: None,
                failure: None,
            });
        self.current = Some(session);
    }
    pub fn draft(&self, session: &Session) -> Option<&NoteDraft> {
        self.drafts.get(session)
    }
    pub fn set_read_only(&mut self, session: &Session, read_only: bool) -> Result<()> {
        self.drafts.get_mut(session).ok_or_else(stale)?.read_only = read_only;
        Ok(())
    }
    pub fn detach(&mut self) {
        self.current = None;
    }
    pub fn focus(&mut self, session: &Session, focused: bool) -> Result<()> {
        let d = self.drafts.get_mut(session).ok_or_else(stale)?;
        d.focused = focused;
        if !focused && d.due.is_none() && d.inflight.is_none() && d.failure.is_none() {
            d.text = d.published.clone();
        }
        Ok(())
    }
    pub fn observe(&mut self, session: &Session, text: String) -> Result<()> {
        let d = self.drafts.get_mut(session).ok_or_else(stale)?;
        // Focused local text wins, rebased against the latest published value.
        // Failed/inflight text stays recoverable even if the editor loses focus.
        if !d.focused
            && d.due.is_none()
            && d.inflight.is_none()
            && d.failure.is_none()
            && d.text == d.published
        {
            d.text = text.clone();
        }
        if d.inflight.is_none() {
            d.published = text;
        }
        Ok(())
    }
    pub fn input(&mut self, session: &Session, text: String, now_ms: u64) -> Result<()> {
        let d = self.drafts.get_mut(session).ok_or_else(stale)?;
        if d.read_only {
            return Err(invalid("notes are read-only"));
        }
        d.text = text;
        d.due = Some(now_ms.saturating_add(NOTES_DEBOUNCE_MS));
        d.failure = None;
        Ok(())
    }
    /// force on blur, project:flush-edits, switch, close or shutdown. Caller
    /// must keep the old session open until every write is acknowledged.
    pub fn flush(
        &mut self,
        session: &Session,
        now_ms: u64,
        force: bool,
    ) -> Result<Option<NoteWrite>> {
        let d = self.drafts.get_mut(session).ok_or_else(stale)?;
        if d.inflight.is_some() {
            return Ok(None);
        }
        if d.text == d.published {
            d.due = None;
            return Ok(None);
        }
        if !force && d.due.is_none_or(|at| now_ms < at) {
            return Ok(None);
        }
        let write = NoteWrite {
            ticket: ticket(),
            session: session.clone(),
            expected: d.published.clone(),
            text: d.text.clone(),
        };
        d.inflight = Some(write.clone());
        d.due = None;
        Ok(Some(write))
    }
    pub fn settle(&mut self, write: &NoteWrite, result: Result<()>) -> Result<()> {
        let d = self.drafts.get_mut(&write.session).ok_or_else(stale)?;
        if d.inflight.as_ref() != Some(write) {
            return Err(stale());
        }
        d.inflight = None;
        match result {
            Ok(()) => {
                d.published = write.text.clone();
                d.failure = None;
            }
            Err(e) => {
                d.failure = Some(e.code);
                return Err(e);
            }
        }
        Ok(())
    }
    pub fn can_release(&self, session: &Session) -> bool {
        self.drafts
            .get(session)
            .is_none_or(|d| d.text == d.published && d.inflight.is_none() && d.failure.is_none())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TakeSaved {
    pub revision: u64,
    pub oldest_archived: usize,
}
pub fn save_take(host: &mut impl RecordHost, before: &Records, take: Value) -> Result<TakeSaved> {
    before.validate()?;
    let take_id = id(&take)?;
    if take.get("label").and_then(Value::as_str).is_none()
        || take.get("at").and_then(Value::as_f64).is_none()
        || !take.get("json").is_some_and(Value::is_string)
    {
        return Err(invalid("invalid take snapshot"));
    }
    if before
        .entries
        .iter()
        .any(|v| v.get("id").and_then(Value::as_str) == Some(take_id))
    {
        return Err(invalid("take id already exists"));
    }
    let mut after = vec![take];
    after.extend(before.entries.clone());
    // S103 keeps 24 visible, archiving overflow in the atomic retained generation.
    let oldest_archived = after.len().saturating_sub(24);
    after.truncate(24);
    Ok(TakeSaved {
        revision: replace(host, "takes", before, &after)?,
        oldest_archived,
    })
}
/// Immediate Delete's Undo is valid only until the next status/mutation. It
/// doesn't expose a general archive browser or discard the retained generation.
pub fn undo_take_delete(
    host: &mut impl RecordHost,
    current: &Records,
    before_delete: &Records,
    deleted_revision: u64,
) -> Result<u64> {
    if current.revision != deleted_revision {
        return Err(stale());
    }
    replace(host, "takes", current, &before_delete.entries)
}
pub fn archive_take(host: &mut impl RecordHost, before: &Records, take_id: &str) -> Result<u64> {
    before.find(take_id)?;
    let after: Vec<_> = before
        .entries
        .iter()
        .filter(|v| v["id"] != take_id)
        .cloned()
        .collect();
    replace(host, "takes", before, &after)
}
pub trait TakeRestoreHost {
    /// Single undoable kernel transaction, with session/revision guard. Preserve
    /// the pre-restore editable source/history; parse tagged snapshot via kernel.
    fn restore(&mut self, session: &Session, revision: u64, snapshot: &str) -> Result<u64>;
}
pub fn restore_take(
    host: &mut impl TakeRestoreHost,
    session: &Session,
    revision: u64,
    records: &Records,
    take_id: &str,
) -> Result<u64> {
    records.validate()?;
    let snapshot = records
        .find(take_id)?
        .get("json")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing take snapshot"))?;
    host.restore(session, revision, snapshot)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModRow {
    pub id: String,
    pub record: Value,
    pub builtin: bool,
    pub needs_attention: bool,
    pub needs_setup: bool,
    pub actions: BTreeSet<ModAction>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModAction {
    Enable,
    Disable,
    Reload,
    SetUp,
    Reveal,
    Archive,
}
pub fn mods(records: &Records) -> Result<Vec<ModRow>> {
    records.validate()?;
    records
        .entries
        .iter()
        .map(|record| {
            let builtin = record["scope"] == "builtin";
            let state = record["health"]["state"].as_str().unwrap_or("");
            let enabled = record["enabled"]
                .as_bool()
                .ok_or_else(|| invalid("missing mod enablement"))?;
            let mut actions = BTreeSet::from([
                ModAction::Reload,
                if enabled {
                    ModAction::Disable
                } else {
                    ModAction::Enable
                },
            ]);
            if !builtin {
                actions.extend([ModAction::Reveal, ModAction::Archive]);
            }
            if record["scope"] == "user"
                && record["manifest"]["vars"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
            {
                actions.insert(ModAction::SetUp);
            }
            Ok(ModRow {
                id: id(record)?.into(),
                record: record.clone(),
                builtin,
                needs_attention: !["ok", "disabled", "replaced"].contains(&state),
                needs_setup: enabled && state == "needs-setup",
                actions,
            })
        })
        .collect()
}
pub trait ModsHost {
    /// Dispatch to extension runtime/store lifecycle, and refresh published rows.
    /// Archive must preserve source, manifest, fork provenance AND entered values.
    fn act(&mut self, revision: u64, id: &str, action: ModAction) -> Result<()>;
}
pub fn act_on_mod(
    host: &mut impl ModsHost,
    records: &Records,
    mod_id: &str,
    action: ModAction,
) -> Result<()> {
    let rows = mods(records)?;
    let row = rows.iter().find(|r| r.id == mod_id).ok_or_else(stale)?;
    if !row.actions.contains(&action) {
        return Err(invalid("mod action unavailable"));
    }
    host.act(records.revision, mod_id, action)
}

/// Metadata is supplied by the caller; dates/names are never synthesized here.
pub fn take_record(id: String, label: String, at_ms: u64, snapshot: String) -> Value {
    json!({"id": id, "label": label, "at": at_ms, "json": snapshot})
}
