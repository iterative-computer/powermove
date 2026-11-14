//! Forced quits around recovery: the work of every session comes back, and
//! a session never leaves its journal where another project's open would try
//! to replay it. Synthetic projects only.
mod common;
use common::TempDir;
use pm_kernel::{journal, persistence::DiskFormat, session::*, *};
use pm_model::Value;
use std::path::Path;

fn value() -> Value {
    pm_model::json::parse(r##"{"id":"p","name":"Base","compId":"c","w":320,"h":180,"fps":30,"dur":1,"bg":"#000000","layers":[],"assets":{},"comps":{}}"##).unwrap()
}
fn config(root: &Path) -> KernelConfig {
    KernelConfig {
        journal_root: Some(root.join("journals")),
        backup_root: None,
        journal: journal::JournalConfig::default(),
        ..Default::default()
    }
}
fn kernel(root: &Path) -> Kernel {
    let mut k = Kernel::new(config(root));
    k.shell(ShellCall::AttachLibrary {
        profile: root.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
    k
}
fn rename(k: &mut Kernel, id: &str, name: &str) {
    let ack = k
        .handle(
            id,
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Rename".into()),
                    body: vec![
                        CompCommand::Set {
                            patch: Box::new(pm_kernel::commands::comps::CompPatch {
                                name: Some(name.into()),
                                ..Default::default()
                            }),
                        }
                        .into(),
                    ]
                    .into(),
                    expected_revision: None,
                },
            ),
        )
        .unwrap()
        .reply
        .unwrap();
    assert!(
        matches!(ack.reply, Reply::Applied { changed: true, .. }),
        "{ack:?}"
    );
}
fn widen(k: &mut Kernel, id: &str, width: f64) {
    let ack = k
        .handle(
            id,
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Width".into()),
                    body: vec![
                        CompCommand::Set {
                            patch: Box::new(pm_kernel::commands::comps::CompPatch {
                                width: Some(width),
                                ..Default::default()
                            }),
                        }
                        .into(),
                    ]
                    .into(),
                    expected_revision: None,
                },
            ),
        )
        .unwrap()
        .reply
        .unwrap();
    assert!(
        matches!(ack.reply, Reply::Applied { changed: true, .. }),
        "{ack:?}"
    );
}
fn width(k: &Kernel, id: &str) -> f64 {
    let root = k.project(id).unwrap().store().root().unwrap().to_value();
    pm_model::canonical::realize_project(root)
        .get("w")
        .and_then(Value::as_f64)
        .unwrap()
}
fn name(k: &Kernel, id: &str) -> String {
    k.project(id)
        .unwrap()
        .store()
        .root()
        .unwrap()
        .get("name")
        .unwrap()
        .to_value()
        .as_str()
        .unwrap()
        .to_owned()
}

/// What a launch opens: the file, or a library project (a recovered copy).
#[derive(Clone)]
enum Opens {
    File(std::path::PathBuf),
    Library(std::path::PathBuf, String),
}

/// Launch: open as the app does (PrepareOpen) and, when asked, press
/// `choice`. Returns the open project's id and what was offered.
fn launch_of(k: &mut Kernel, opens: &Opens, choice: RecoveryDecision) -> (Option<String>, String) {
    let (id, source) = match opens {
        Opens::File(path) => ("file".to_owned(), SessionSource::File(path.clone())),
        Opens::Library(profile, slot) => (
            slot.clone(),
            SessionSource::Library {
                profile: profile.clone(),
                project: slot.clone(),
            },
        ),
    };
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id,
            source,
            electron_profile: None,
        })
        .unwrap();
    match reply {
        ShellReply::Opened { session, .. } => (Some(session.project), "opened".into()),
        ShellReply::RecoveryRequired { ticket, recovery } => {
            let offered = format!("{recovery:?}");
            let decided = k.shell(ShellCall::DecideRecovery {
                ticket,
                decision: choice,
            });
            match decided {
                Ok(ShellReply::Opened { session, .. }) => (Some(session.project), offered),
                other => (None, format!("{offered} -> {other:?}")),
            }
        }
        other => (None, format!("{other:?}")),
    }
}
fn launch(k: &mut Kernel, path: &Path, choice: RecoveryDecision) -> (Option<String>, String) {
    launch_of(k, &Opens::File(path.to_owned()), choice)
}

/// The recovered copies' library slots, by their journal folders.
fn copies(root: &TempDir) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(root.path("journals"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("recovery"))
        .collect();
    found.sort();
    found
}

/// Saves a new project as `project.pmv` under `root`.
fn saved_file(root: &TempDir, project: Value) -> std::path::PathBuf {
    let path = root.path("project.pmv");
    let mut store = DocumentStore::new_untitled(project);
    common::save_as(&mut store, &path).unwrap();
    path
}

/// The newest journal session in `folder` with its checkpoints removed: the
/// app was killed before the session's first checkpoint landed (a large
/// project takes seconds to write one).
fn kill_before_first_checkpoint(folder: &Path) {
    let session = journal::sessions(folder).pop().expect("a session");
    for e in std::fs::read_dir(&session).unwrap().flatten() {
        if e.file_name().to_string_lossy().starts_with("checkpoint-") {
            std::fs::remove_file(e.path()).unwrap();
        }
    }
}

/// Recovering, editing and being killed again, round after round (the
/// playback lane's sequence, and acceptance run 5's three recoveries): each
/// time the work comes back in place, in the same project under its own
/// name, as v1.2.0 brought a project back after a crash; never as a copy
/// while the file is unchanged.
#[test]
fn crash_recover_edit_crash_again_recovers_in_place() {
    let root = TempDir::new("recovery-chain");
    let path = saved_file(&root, value());
    let mut expected = 320.0;
    for round in 1..=5 {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("round {round}: {offered}"));
        let ask = if round == 1 {
            "opened"
        } else {
            "Recovered { truncated: false }"
        };
        assert_eq!(offered, ask, "round {round}");
        assert_eq!(id, "file");
        assert_eq!(name(&k, &id), "Base", "round {round}: the name stays");
        assert_eq!(
            width(&k, &id),
            expected,
            "round {round}: the work came back"
        );
        expected = 320.0 + round as f64;
        widen(&mut k, &id, expected);
        k.durability_barrier().unwrap();
        drop(k); // killed
    }
    assert!(copies(&root).is_empty(), "no copies: {:?}", copies(&root));
}

/// The playback lane's journal of 2026-11-14. Work offered as a copy
/// (here because another program replaced the file while it was pending)
/// opens with Recover in the file's window: under the file's project id,
/// saving to the copy's library slot. It is edited, and the app is killed
/// before that session's first checkpoint lands (a large project takes
/// seconds). The copy's journal belongs in the copy's folder; it was started
/// in the file's, so the next open of the file found it, could not replay it
/// onto the file ("the given document is not this journal's base") and said
/// the project couldn't be recovered, while the copy lost its edit.
#[test]
fn a_recovered_copys_session_is_journalled_under_the_copy() {
    let root = TempDir::new("recovery-copy-chain");
    let path = saved_file(&root, value());
    let journals = root.path("journals");
    // 1. Edit the file, killed.
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        rename(&mut k, &id, "Edit 1");
        k.durability_barrier().unwrap();
    }
    // Another program replaces the file meanwhile.
    std::fs::remove_file(&path).unwrap();
    let mut other = value();
    other
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("Elsewhere"));
    let mut store = DocumentStore::new_untitled(other);
    common::save_as(&mut store, &path).unwrap();
    drop(store);
    // 2. The work is offered as a copy; Recover opens the copy; an edit;
    //    killed before the session's first checkpoint.
    let copy = {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("round 2: {offered}"));
        assert!(offered.starts_with("OfferCopy"), "{offered}");
        assert_eq!(name(&k, &id), "Edit 1 — Recovered");
        widen(&mut k, &id, 400.0);
        k.durability_barrier().unwrap();
        drop(k);
        let found = copies(&root);
        assert_eq!(
            found.len(),
            1,
            "the copy's journal is in its own folder: {found:?}"
        );
        found[0].clone()
    };
    kill_before_first_checkpoint(&journals.join(&copy));
    // 3. The file opens without a refusal: the copy holds that work.
    let mut k = kernel(&root.0);
    let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
    assert!(id.is_some(), "the file: {offered}");
    assert_eq!(offered, "opened", "the file");
    assert_eq!(name(&k, "file"), "Elsewhere");
    drop(k);
    // 4. The copy recovers its edit, in place, under its one name.
    let mut k = kernel(&root.0);
    let opens = Opens::Library(root.0.clone(), copy.clone());
    let (id, offered) = launch_of(&mut k, &opens, RecoveryDecision::Recover);
    let id = id.unwrap_or_else(|| panic!("the copy: {offered}"));
    assert_eq!(offered, "Recovered { truncated: false }");
    assert_eq!(width(&k, &id), 400.0);
    assert_eq!(name(&k, &id), "Edit 1 — Recovered");
    widen(&mut k, &id, 401.0);
    k.durability_barrier().unwrap();
    drop(k);
    // 5. Killed again: still in place, one suffix, one copy.
    let mut k = kernel(&root.0);
    let (id, offered) = launch_of(&mut k, &opens, RecoveryDecision::Recover);
    let id = id.unwrap_or_else(|| panic!("the copy again: {offered}"));
    assert_eq!(offered, "Recovered { truncated: false }");
    assert_eq!(width(&k, &id), 401.0);
    assert_eq!(name(&k, &id), "Edit 1 — Recovered");
    assert_eq!(copies(&root), [copy]);
}

/// A file whose opening upgrades it (old 3D objects), edited and killed
/// before its first checkpoint: the edit comes back.
#[test]
fn an_upgraded_open_killed_before_its_first_checkpoint_recovers() {
    let root = TempDir::new("recovery-upgraded");
    let mut p = value();
    p.as_object_mut().unwrap().insert(
        "layers",
        pm_model::json::parse(r#"[{"id":"M","type":"shape","name":"Model","from":0,"dur":1,"on":true,"p":{},"d":{"definition":"powermove.3d.obj-model"},"fx":[]}]"#).unwrap(),
    );
    let path = saved_file(&root, p);
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        assert!(k.project(&id).unwrap().store().upgraded_on_open());
        rename(&mut k, &id, "Edited after the upgrade");
        k.durability_barrier().unwrap();
    }
    kill_before_first_checkpoint(&root.path("journals/file"));
    let mut k = kernel(&root.0);
    let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
    let id = id.unwrap_or_else(|| panic!("{offered}"));
    assert_eq!(name(&k, &id), "Edited after the upgrade");
}

/// Work that cannot be replayed (its base file was replaced by another
/// program, and no checkpoint landed): the offer says what it was, and its
/// journal stays on disk whatever is answered: unanswered it is asked
/// again, and Open Saved Version sets it aside, still kept.
#[test]
fn unrecoverable_work_is_described_and_kept() {
    let root = TempDir::new("recovery-unrecoverable");
    let path = saved_file(&root, value());
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        rename(&mut k, &id, "First");
        rename(&mut k, &id, "Second");
        k.durability_barrier().unwrap();
    }
    let folder = root.path("journals/file");
    kill_before_first_checkpoint(&folder);
    let crashed = journal::sessions(&folder).pop().unwrap();
    // Another program replaces the file.
    let mut other = value();
    other
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("Replaced"));
    std::fs::remove_file(&path).unwrap();
    {
        let mut store = DocumentStore::new_untitled(other);
        common::save_as(&mut store, &path).unwrap();
    }
    let ask = |k: &mut Kernel| match k
        .shell(ShellCall::PrepareOpen {
            id: "file".into(),
            source: SessionSource::File(path.clone()),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::RecoveryRequired { ticket, recovery } => (ticket, recovery),
        other => panic!("{other:?}"),
    };
    let mut k = kernel(&root.0);
    let (_, recovery) = ask(&mut k);
    let RecoveryResult::Unrecoverable { lost, .. } = &recovery else {
        panic!("{recovery:?}");
    };
    assert_eq!(lost.changes, 2);
    assert_eq!(lost.steps, ["Rename"]);
    assert!(lost.from.is_some() && lost.until >= lost.from, "{lost:?}");
    drop(k); // no answer
    assert!(crashed.exists(), "kept while unanswered");
    let mut k = kernel(&root.0);
    let (ticket, _) = ask(&mut k);
    let ShellReply::Opened { .. } = k
        .shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::OpenSavedVersion,
        })
        .unwrap()
    else {
        panic!("the saved version did not open");
    };
    assert_eq!(name(&k, "file"), "Replaced");
    let kept = folder.join("discarded").join(crashed.file_name().unwrap());
    assert!(
        kept.join("journal-00000000.log").exists(),
        "set aside, kept"
    );
}
