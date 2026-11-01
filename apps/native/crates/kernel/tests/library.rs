//! The v1.2.0 library store read path and persisted-history round trips,
//! over synthetic fixtures (tests/fixtures/library-store).

mod common;

use pm_kernel::library::{LibraryStore, StoreKey, legacy_apply_patches, parse_key};
use pm_kernel::{
    Caller, History, HistoryCall, Kernel, KernelConfig, OpenSource, Reply, Request,
};
use pm_model::json::{Value, parse, to_canonical_string};

fn store() -> LibraryStore {
    LibraryStore::open(&common::crate_dir().join("tests/fixtures/library-store"))
}

fn layers(v: &Value) -> Vec<String> {
    v.get("comps")
        .and_then(|c| c.get("C1"))
        .and_then(|c| c.get("layers"))
        .or_else(|| v.get("layers"))
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .map(|l| l.get("name").and_then(Value::as_str).unwrap().to_owned())
        .collect()
}

#[test]
fn keys_follow_the_registry() {
    assert_eq!(parse_key("projects"), Some(StoreKey::Static("projects".into())));
    assert_eq!(
        parse_key("projectHistory.ab_C-9"),
        Some(StoreKey::Dynamic {
            prefix: "projectHistory".into(),
            id: "ab_C-9".into()
        })
    );
    assert_eq!(parse_key("project.bad id"), None);
    assert_eq!(parse_key("pm.projects"), None, "no pm.* file namespace");
    assert_eq!(parse_key(&format!("project.{}", "x".repeat(121))), None);
}

#[test]
fn summary_counts_families_orphans_and_quarantine() {
    let s = store().summary().unwrap();
    assert_eq!(s.files, 20, "the .tmp- file is skipped");
    assert_eq!(s.static_keys, 3); // projects, projectTrash, autosave
    assert_eq!(s.dynamic.get("project"), Some(&6));
    assert_eq!(s.dynamic.get("projectHistory"), Some(&3));
    assert_eq!(s.dynamic.get("ext"), Some(&1));
    assert_eq!(s.unregistered, 1);
    assert_eq!(s.quarantined, 1);
    assert_eq!((s.index_rows, s.trash_rows), (2, 1));
    assert_eq!(s.orphan_slots, 3, "orphan, broken and delta are kept, never dropped");
    assert_eq!(s.missing_slots, 0);
    let index = store().projects().unwrap();
    assert!(index[1].has_thumb && !index[0].has_thumb);
}

#[test]
fn load_applies_the_journal_like_get_async_and_overlays_session() {
    let p = store().load("alpha").unwrap().unwrap();
    assert_eq!((p.journal_applied, p.journal_skipped), (2, 1));
    assert_eq!(p.project.get("name").and_then(Value::as_str), Some("Alpha journalled"));
    assert_eq!(layers(&p.project), ["Renamed", "Two"]);
    assert!(p.project.get("missing").is_none(), "a patch with no parent is skipped");
    assert!(p.project.get("futureField").is_some(), "unknown fields are kept");
    // projectMeta and projectHistory both exist: the legacy state is not read.
    let session = p.session.unwrap();
    assert_eq!(session.get("time").and_then(Value::as_f64), Some(1.5));
    assert!(session.get("view").is_none() && session.get("history").is_none());
    assert_eq!(
        p.history.unwrap().get("format").and_then(Value::as_str),
        Some("powermove-history-dictionary")
    );
    assert!(p.workspace.is_some());
}

#[test]
fn load_reads_stringified_slots_legacy_state_history_and_autosave() {
    let beta = store().load("beta").unwrap().unwrap();
    assert_eq!(layers(&beta.project), ["Before"]);
    let session = beta.session.unwrap();
    assert_eq!(session.get("time").and_then(Value::as_f64), Some(2.0));
    assert!(session.get("history").is_none());
    assert_eq!(
        beta.history.unwrap().get("entries").and_then(Value::as_array).map(Vec::len),
        Some(1)
    );
    let legacy = store().load("legacy").unwrap().unwrap();
    assert!(legacy.from_autosave);
    assert_eq!(layers(&legacy.project), ["Autosaved"]);
    assert!(store().load("absent").unwrap().is_none());
}

#[test]
fn library_projects_open_in_the_kernel_with_undo_and_export_round_trips() {
    let lib = store().load("alpha").unwrap().unwrap();
    let packed = lib.history.clone().unwrap();
    let mut k = Kernel::new(KernelConfig::default());
    let info = k.open("alpha", OpenSource::Library(lib)).unwrap();
    assert_eq!(info.history_entries, 2);
    assert!(!info.history_rejected);
    let store = k.project("alpha").unwrap().store();
    assert!(!store.is_dirty(), "matches its slot");
    // Export: packed form equals what the old packer stored; plain form
    // equals its unpacked contents.
    let snapshot = store.history().snapshot();
    assert_eq!(
        to_canonical_string(&snapshot.to_saved_packed()),
        to_canonical_string(&packed)
    );
    let reimported = History::from_saved(&snapshot.to_saved(), Some("C1")).unwrap();
    assert_eq!(
        to_canonical_string(&reimported.snapshot().to_saved()),
        to_canonical_string(&snapshot.to_saved())
    );
    // Undo after migration.
    let undo = |k: &mut Kernel| {
        k.handle("alpha", Request::new(Caller::native(), HistoryCall::Undo))
            .unwrap()
            .reply
            .unwrap()
            .reply
    };
    let doc = |k: &Kernel| k.project("alpha").unwrap().store().snapshot().document.to_value().unwrap();
    assert!(matches!(undo(&mut k), Reply::Bool(true)));
    assert_eq!(layers(&doc(&k)), ["One", "Two"]);
    assert!(matches!(undo(&mut k), Reply::Bool(true)));
    assert_eq!(layers(&doc(&k)), ["One"]);
    assert!(matches!(undo(&mut k), Reply::Bool(false)));
    // The exported cursor after undo (index -1) re-imports as all-redo.
    let saved = k.project("alpha").unwrap().store().history().snapshot().to_saved();
    assert_eq!(saved.get("index").and_then(Value::as_f64), Some(-1.0));
}

#[test]
fn v1_3_gzip_disk_envelopes_are_decoded() {
    let delta = store().load("delta").unwrap().unwrap();
    let history = delta.history.clone().unwrap();
    assert_eq!(history.get("version").and_then(Value::as_f64), Some(1.0));
    let mut k = Kernel::new(KernelConfig::default());
    let info = k.open("delta", OpenSource::Library(delta)).unwrap();
    assert_eq!((info.history_entries, info.history_rejected), (1, false));
    let ack = k
        .handle("delta", Request::new(Caller::native(), HistoryCall::Undo))
        .unwrap()
        .reply
        .unwrap();
    assert!(matches!(ack.reply, Reply::Bool(true)));
    let doc = k.project("delta").unwrap().store().snapshot().document.to_value().unwrap();
    assert_eq!(layers(&doc), ["Before"]);
    // A truncated or oversized envelope is refused, never half-read.
    let bad = parse(r#"{"__powermoveDiskValue":1,"encoding":"gzip","bytes":999,"data":"H4sIAAAAAAAA"}"#).unwrap();
    assert!(pm_kernel::library::decode_disk_value(bad.clone(), 1 << 20).is_err());
    assert!(pm_kernel::library::decode_disk_value(bad, 10).is_err());
}

#[test]
fn an_invalid_dictionary_never_opens_silently() {
    let lib = store().load("broken").unwrap().unwrap();
    let mut k = Kernel::new(KernelConfig::default());
    let info = k.open("broken", OpenSource::Library(lib)).unwrap();
    assert!(info.history_rejected);
    assert_eq!(info.history_entries, 0);
}

#[test]
fn legacy_apply_patches_matches_projects_ts() {
    let mut root = Some(parse(r#"{"a":[1,2,3],"o":{"k":1}}"#).unwrap());
    legacy_apply_patches(
        &mut root,
        &parse(
            r#"[{"path":["a",1],"exists":false},
                {"path":["o","k"],"exists":false},
                {"path":["o","n"],"exists":true,"value":{"x":1}},
                {"path":["nope","x"],"exists":true,"value":1},
                {"path":["a",4],"exists":true,"value":9}]"#,
        )
        .unwrap(),
    );
    assert_eq!(
        to_canonical_string(root.as_ref().unwrap()),
        r#"{"a":[1,3,null,null,9],"o":{"n":{"x":1}}}"#
    );
    legacy_apply_patches(&mut root, &parse(r#"[{"path":[],"exists":false}]"#).unwrap());
    assert!(root.is_none());
}

/// Read-only smoke test against a copy of a real profile store. Ignored by
/// default; reports counts only (never names, ids or content):
/// `PM_KERNEL_STORE_COPY=/tmp/copy cargo test -p pm-kernel --test library -- --ignored --nocapture`
#[test]
#[ignore]
fn smoke_real_store_copy_counts_only() {
    let Ok(dir) = std::env::var("PM_KERNEL_STORE_COPY") else {
        return;
    };
    let store = LibraryStore::open(std::path::Path::new(&dir));
    let summary = store.summary().unwrap();
    let mut loaded = 0;
    let mut opened = 0;
    let mut histories = 0;
    let mut rejected = 0;
    let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
    let mut journal_entries = 0;
    let mut failures = 0;
    let mut k = Kernel::new(KernelConfig::default());
    let keys = store.keys().unwrap();
    for key in &keys {
        let StoreKey::Dynamic { prefix, id } = key else { continue };
        if prefix != "project" {
            continue;
        }
        match store.load(id) {
            Ok(Some(p)) => {
                loaded += 1;
                journal_entries += p.journal_applied;
                histories += usize::from(p.history.is_some());
                match k.open(id, OpenSource::Library(p)) {
                    Ok(info) => {
                        opened += 1;
                        rejected += usize::from(info.history_rejected);
                        if let Some(reason) = info.history_error {
                            *reasons.entry(reason).or_default() += 1;
                        }
                        k.close(id);
                    }
                    Err(_) => failures += 1,
                }
            }
            _ => failures += 1,
        }
    }
    eprintln!("store summary: {summary:?}");
    eprintln!("history refusal reasons: {reasons:?}");
    eprintln!(
        "project slots loaded {loaded}, opened in kernel {opened}, with history {histories}, \
         history rejected {rejected}, journal entries applied {journal_entries}, failures {failures}"
    );
}
