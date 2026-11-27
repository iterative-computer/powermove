use super::*;
use crate::surface_backing::{projects::*, workspaces::*};

#[test]
fn notes_debounce_and_switch_keep_original_session() {
    let mut notes = Notes::default();
    let old = session(1);
    let new = session(2);
    notes.attach(old.clone(), "before".into());
    notes.input(&old, "after".into(), 10).unwrap();
    assert!(notes.flush(&old, 259, false).unwrap().is_none());
    notes.attach(new.clone(), "other".into());
    let write = notes.flush(&old, 260, false).unwrap().unwrap();
    assert_eq!(write.session, old);
    assert_eq!(write.expected, "before");
    assert!(!notes.can_release(&old));
    notes.settle(&write, Ok(())).unwrap();
    assert!(notes.can_release(&old));
    assert_eq!(notes.draft(&new).unwrap().text, "other");
}
#[test]
fn notes_failure_retains_text_and_force_flush_retries() {
    let mut notes = Notes::default();
    let s = session(1);
    notes.attach(s.clone(), "old".into());
    notes.input(&s, "new".into(), 0).unwrap();
    let first = notes.flush(&s, 0, true).unwrap().unwrap();
    assert!(notes.settle(&first, Err(failure())).is_err());
    assert_eq!(
        notes.draft(&s).unwrap().failure,
        Some(ServiceErrorCode::DiskFull)
    );
    notes.observe(&s, "external".into()).unwrap();
    assert_eq!(notes.draft(&s).unwrap().text, "new");
    let retry = notes.flush(&s, 0, true).unwrap().unwrap();
    assert_ne!(retry.ticket, first.ticket);
    assert!(notes.settle(&first, Ok(())).is_err());
    notes.settle(&retry, Ok(())).unwrap();
    assert!(notes.can_release(&s));
}
#[test]
fn notes_edit_while_inflight_cannot_be_acknowledged_as_saved() {
    let mut notes = Notes::default();
    let s = session(1);
    notes.attach(s.clone(), "a".into());
    notes.input(&s, "b".into(), 0).unwrap();
    let first = notes.flush(&s, 250, false).unwrap().unwrap();
    notes.input(&s, "c".into(), 300).unwrap();
    notes.settle(&first, Ok(())).unwrap();
    assert!(!notes.can_release(&s));
    let second = notes.flush(&s, 550, false).unwrap().unwrap();
    assert_eq!(second.expected, "b");
    assert_eq!(second.text, "c");
}
#[test]
fn focused_notes_reject_external_replacement_and_clean_unfocused_notes_refresh() {
    let mut notes = Notes::default();
    let s = session(1);
    notes.attach(s.clone(), "a".into());
    notes.focus(&s, true).unwrap();
    notes.observe(&s, "b".into()).unwrap();
    assert_eq!(notes.draft(&s).unwrap().text, "a");
    notes.focus(&s, false).unwrap();
    notes.observe(&s, "b".into()).unwrap();
    assert_eq!(notes.draft(&s).unwrap().text, "b");
}
#[test]
fn reopened_project_is_distinct_from_pending_old_notes() {
    let mut notes = Notes::default();
    let old = session(1);
    let reopened = session(2);
    notes.attach(old.clone(), "a".into());
    notes.input(&old, "b".into(), 0).unwrap();
    notes.attach(reopened.clone(), "a".into());
    assert!(!notes.can_release(&old));
    assert!(notes.can_release(&reopened));
}
fn take(n: u64) -> Value {
    let mut t = take_record(
        format!("t{n}"),
        format!("authored {n}"),
        n,
        "{\"future\":true}".into(),
    );
    t["unknown"] = json!([1, 2]);
    t
}
#[test]
fn takes_keep_24_visible_and_archive_older_authored_snapshots() {
    let before = records((0..26).map(take).collect());
    let mut host = MemoryRecords {
        current: Some(before.clone()),
        ..Default::default()
    };
    let saved = save_take(&mut host, &before, take(27)).unwrap();
    assert_eq!(saved.oldest_archived, 3);
    assert_eq!(host.current.as_ref().unwrap().entries.len(), 24);
    assert_eq!(host.current.as_ref().unwrap().entries[23], take(22));
    assert_eq!(host.history, vec![before]);
}
#[test]
fn take_archive_failure_never_removes_original() {
    let before = records(vec![take(1)]);
    let mut host = MemoryRecords {
        current: Some(before.clone()),
        fail: true,
        ..Default::default()
    };
    assert!(archive_take(&mut host, &before, "t1").is_err());
    assert_eq!(host.current, Some(before.clone()));
    host.fail = false;
    archive_take(&mut host, &before, "t1").unwrap();
    assert_eq!(host.history, vec![before]);
}
#[test]
fn take_restore_passes_exact_snapshot_and_session_to_one_undo_adapter() {
    struct Restore;
    impl TakeRestoreHost for Restore {
        fn restore(&mut self, s: &Session, rev: u64, snapshot: &str) -> Result<u64> {
            assert_eq!(s, &session(3));
            assert_eq!(rev, 5);
            assert_eq!(snapshot, "{\"future\":true}");
            Ok(6)
        }
    }
    assert_eq!(
        restore_take(&mut Restore, &session(3), 5, &records(vec![take(1)]), "t1").unwrap(),
        6
    );
}
#[test]
fn takes_refuse_duplicate_ids_and_stale_writes() {
    let before = records(vec![take(1)]);
    let mut host = MemoryRecords::default();
    assert!(save_take(&mut host, &before, take(1)).is_err());
    assert!(archive_take(&mut host, &before, "t1").is_err());
    assert!(host.history.is_empty());
}
#[test]
fn mods_preserve_unknown_health_and_provenance_without_optimistic_enablement() {
    let before = records(vec![
        json!({"id":"user","scope":"user","enabled":true,"health":{"state":"needs-setup"},"manifest":{"vars":[{}],"forkedFrom":"x@1"},"future":{"n":3}}),
        json!({"id":"builtin","scope":"builtin","enabled":false,"health":{"state":"replaced","by":"user"}}),
    ]);
    let rows = mods(&before).unwrap();
    assert!(rows[0].needs_setup);
    assert!(rows[0].needs_attention);
    assert!(rows[0].actions.contains(&ModAction::SetUp));
    assert!(!rows[1].actions.contains(&ModAction::Archive));
    assert!(!rows[1].needs_attention);
    assert_eq!(rows[0].record, before.entries[0]);
}
#[test]
fn builtin_mod_archive_is_refused_and_host_failure_propagates() {
    struct Host;
    impl ModsHost for Host {
        fn act(&mut self, _: u64, _: &str, _: ModAction) -> Result<()> {
            Err(failure())
        }
    }
    let r = records(vec![
        json!({"id":"b","scope":"builtin","enabled":true,"health":{"state":"ok"}}),
    ]);
    assert_eq!(
        act_on_mod(&mut Host, &r, "b", ModAction::Archive)
            .unwrap_err()
            .code,
        ServiceErrorCode::InvalidInput
    );
    assert_eq!(
        act_on_mod(&mut Host, &r, "b", ModAction::Disable)
            .unwrap_err()
            .code,
        ServiceErrorCode::DiskFull
    );
    assert_eq!(r.entries[0]["enabled"], true);
}
#[derive(Default)]
struct Ws {
    records: MemoryRecords,
    activated: Vec<String>,
    invalid: bool,
}
impl RecordHost for Ws {
    fn replace_retaining(&mut self, s: &str, b: &Records, a: &[Value]) -> Result<u64> {
        self.records.replace_retaining(s, b, a)
    }
}
impl WorkspaceHost for Ws {
    fn validate_definition(&mut self, _: &Value) -> Result<()> {
        if self.invalid {
            Err(invalid("bad layout"))
        } else {
            Ok(())
        }
    }
    fn activate(&mut self, id: &str, _: u64) -> Result<()> {
        self.activated.push(id.into());
        Ok(())
    }
}
fn workspace() -> Value {
    json!({"id":"w","name":"Authored","custom":[{"id":"custom","controls":[{"type":"future"}]}],"theme":{"future":5},"unknown":{"exact":[1,2]}})
}
#[test]
fn workspace_json_editor_pins_identity_and_preserves_extension_payload() {
    let before = records(vec![workspace()]);
    let mut host = Ws {
        records: MemoryRecords {
            current: Some(before.clone()),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut draft = DefinitionDraft::open(&before, "w").unwrap();
    let mut v = workspace();
    v["id"] = "hijacked".into();
    draft.json = v.to_string();
    draft.apply(&mut host, &before).unwrap();
    assert_eq!(host.records.current.unwrap().entries[0], workspace());
    assert_eq!(host.activated, vec!["w"]);
}
#[test]
fn invalid_and_stale_workspace_json_never_reaches_persistence() {
    let before = records(vec![workspace()]);
    let mut host = Ws::default();
    let mut draft = DefinitionDraft::open(&before, "w").unwrap();
    draft.json = "{bad".into();
    assert!(draft.apply(&mut host, &before).is_err());
    draft.json = workspace().to_string();
    host.invalid = true;
    assert!(draft.apply(&mut host, &before).is_err());
    let mut stale_records = before.clone();
    stale_records.revision += 1;
    assert!(draft.apply(&mut host, &stale_records).is_err());
    assert!(host.records.history.is_empty());
}
#[test]
fn save_workspace_as_new_keeps_custom_sections_and_authored_metadata() {
    let before = records(vec![workspace()]);
    let mut host = Ws {
        records: MemoryRecords {
            current: Some(before.clone()),
            ..Default::default()
        },
        ..Default::default()
    };
    save_as(&mut host, &before, &workspace(), "new", " New ").unwrap();
    let new = &host.records.current.as_ref().unwrap().entries[1];
    assert_eq!(new["name"], "New");
    assert_eq!(new["unknown"], workspace()["unknown"]);
    assert_eq!(new["custom"], workspace()["custom"]);
}
#[test]
fn current_workspace_archive_requires_valid_fallback_and_retains_old_definition() {
    let mut builtin = workspace();
    builtin["id"] = "design".into();
    builtin["builtin"] = true.into();
    let before = records(vec![workspace(), builtin]);
    let mut host = Ws {
        records: MemoryRecords {
            current: Some(before.clone()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(archive_workspace(&mut host, &before, "w", "w", "missing").is_err());
    assert!(archive_workspace(&mut host, &before, "design", "w", "w").is_err());
    archive_workspace(&mut host, &before, "w", "w", "design").unwrap();
    assert_eq!(host.activated, vec!["design"]);
    assert_eq!(host.records.history, vec![before]);
}
#[test]
fn shelves_and_panel_previews_keep_source_identity() {
    let before = records(vec![
        workspace(),
        json!({"id":"ae","sourceApp":"after-effects","unknown":1}),
        json!({"id":"p","kind":"panel","code":"not evaluated"}),
    ]);
    assert_eq!(
        shelf_entries(&before, Shelf::AfterEffects).unwrap().len(),
        1
    );
    assert_eq!(shelf_entries(&before, Shelf::Panels).unwrap().len(), 1);
    let first = PanelPreview::new(before.entries[2].clone()).unwrap();
    let second = PanelPreview::new(first.definition.clone()).unwrap();
    assert_ne!(first.generation, second.generation);
    assert_eq!(first.panel_id, "p");
}
