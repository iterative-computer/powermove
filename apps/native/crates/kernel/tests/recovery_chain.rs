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

/// A copy of `session` kept aside, to put back where it was: the on-disk
/// state of a kill before the session replacing it wrote checkpoint 0 (the
/// replaced one is retired only once that checkpoint is durable).
struct Kept {
    at: std::path::PathBuf,
    copy: std::path::PathBuf,
}
fn keep(session: &Path, root: &TempDir) -> Kept {
    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                copy_dir(&path, &to.join(e.file_name()));
            } else {
                std::fs::copy(&path, to.join(e.file_name())).unwrap();
            }
        }
    }
    let copy = root.path("kept").join(session.file_name().unwrap());
    copy_dir(session, &copy);
    Kept {
        at: session.to_path_buf(),
        copy,
    }
}
impl Kept {
    fn put_back(&self) {
        if !self.at.exists() {
            std::fs::rename(&self.copy, &self.at).unwrap();
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

/// The project file with `document`'s content saved by another writer whose
/// own history holds one undone step (its redo), `name` set and taken back.
fn other_writers_file(path: &Path, from: &Path) {
    let mut store =
        DocumentStore::open(pm_kernel::project_io::open_project(from, Default::default()).unwrap());
    store
        .execute(
            Command::SetLayerName {
                layer_id: "none".into(),
                name: "x".into(),
            },
            Origin::Interface,
        )
        .ok();
    let comp = pm_kernel::CompCommand::Set {
        patch: Box::new(pm_kernel::commands::comps::CompPatch {
            name: Some("Their edit".into()),
            ..Default::default()
        }),
    };
    store.execute(comp.into(), Origin::Interface).unwrap();
    store.undo().unwrap();
    let other = path.with_extension("other.pmv");
    common::save_as(&mut store, &other).unwrap();
    drop(store);
    std::fs::rename(&other, path).unwrap();
}

/// Review w32, finding 1. Saved A; an edit, killed; Recover, another edit,
/// killed after its checkpoint. Meanwhile another writer saves the file
/// with the same document A and a history of its own (a redo step). The
/// recovered session inherited A only as a document: adopting the file in
/// place would replace that other history on the next save. It is offered
/// as a copy, as before the ancestors were carried.
#[test]
fn an_inherited_base_needs_the_history_it_had() {
    let root = TempDir::new("recovery-inherited-history");
    let path = saved_file(&root, value());
    for round in 1..=2 {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("round {round}: {offered}"));
        rename(&mut k, &id, &format!("Edit {round}"));
        k.durability_barrier().unwrap();
    }
    let original = root.path("original-copy.pmv");
    std::fs::copy(&path, &original).unwrap();
    other_writers_file(&path, &original);
    let mut k = kernel(&root.0);
    let (_, offered) = launch(&mut k, &path, RecoveryDecision::OpenAsCopy);
    assert!(offered.starts_with("OfferCopy"), "{offered}");
}

/// Review w32, finding 2. A recoverable session S1; then a kill while the
/// replacement session was being created, before its header: an empty
/// newest session. The open still finds S1's work and offers it.
#[test]
fn a_half_created_session_hides_nothing() {
    let root = TempDir::new("recovery-half-created");
    let path = saved_file(&root, value());
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        rename(&mut k, &id, "Edited");
        k.durability_barrier().unwrap();
    }
    let folder = root.path("journals/file");
    // What an older build left: the folder made, no header yet; and what this
    // build would leave: its staging folder.
    std::fs::create_dir_all(folder.join("s999999999999999999999999-0000000001")).unwrap();
    std::fs::create_dir_all(folder.join(".creating-s999999999999999999999999-0000000002")).unwrap();
    let mut k = kernel(&root.0);
    let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
    let id = id.unwrap_or_else(|| panic!("{offered}"));
    assert_eq!(offered, "Recovered { truncated: false }");
    assert_eq!(name(&k, &id), "Edited");
}

/// A session's last segment ends in a torn line (killed mid-append).
fn tear_tail(session: &Path) {
    use std::io::Write;
    let mut segments: Vec<_> = std::fs::read_dir(session)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("journal-")
        })
        .collect();
    segments.sort();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(segments.last().unwrap())
        .unwrap();
    f.write_all(b"0123456789abcdef {\"t\":\"rev\",\"b\":")
        .unwrap();
}

/// Review w32, finding 4, the trigger. S1 recovered with a torn tail
/// (accepted as truncated); Recover, an edit in S2, killed before S2's first
/// checkpoint. S2 continues from S1's accepted prefix, which replays to
/// exactly what S2 adopted: both come back.
#[test]
fn a_torn_tail_recovered_then_killed_again_recovers() {
    let root = TempDir::new("recovery-torn");
    let path = saved_file(&root, value());
    let folder = root.path("journals/file");
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        rename(&mut k, &id, "Before the tear");
        k.durability_barrier().unwrap();
    }
    let s1 = journal::sessions(&folder).pop().unwrap();
    tear_tail(&s1);
    let kept = keep(&s1, &root);
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        assert_eq!(offered, "Recovered { truncated: true }");
        widen(&mut k, &id, 333.0);
        k.durability_barrier().unwrap();
    }
    kill_before_first_checkpoint(&folder);
    kept.put_back();
    let mut k = kernel(&root.0);
    let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
    let id = id.unwrap_or_else(|| panic!("{offered}"));
    assert_eq!(name(&k, &id), "Before the tear");
    assert_eq!(width(&k, &id), 333.0);
}

/// Review w32, finding 4. Work that cannot be replayed, whose session
/// continues from an earlier one (killed before its first checkpoint):
/// Open Saved Version sets both aside, and a clean close of the saved
/// version deletes neither.
#[test]
fn open_saved_version_keeps_what_the_set_aside_session_needs() {
    let root = TempDir::new("recovery-archive-chain");
    let path = saved_file(&root, value());
    let folder = root.path("journals/file");
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        rename(&mut k, &id, "First");
        k.durability_barrier().unwrap();
    }
    let s1 = journal::sessions(&folder).pop().unwrap();
    let kept = keep(&s1, &root);
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        rename(&mut k, &id, "Second");
        k.durability_barrier().unwrap();
    }
    kill_before_first_checkpoint(&folder);
    kept.put_back();
    let s2 = journal::sessions(&folder).pop().unwrap();
    assert_ne!(s1, s2);
    // S2's records no longer apply (a revision gap): unrecoverable.
    let segment = s2.join("journal-00000000.log");
    let text = std::fs::read_to_string(&segment).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let rev = lines
        .iter()
        .position(|l| l.contains("\"t\":\"rev\""))
        .expect("a rev record");
    let json = lines[rev]
        .split_once(' ')
        .unwrap()
        .1
        .replace("\"b\":", "\"b\":9");
    let sum = &pm_model::hash::sha256_hex(json.as_bytes())[..16];
    lines[rev] = format!("{sum} {json}");
    std::fs::write(&segment, lines.join("\n") + "\n").unwrap();
    let mut k = kernel(&root.0);
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "file".into(),
            source: SessionSource::File(path.clone()),
            electron_profile: None,
        })
        .unwrap();
    let ShellReply::RecoveryRequired { ticket, recovery } = reply else {
        panic!("{reply:?}");
    };
    assert!(
        matches!(recovery, RecoveryResult::Unrecoverable { .. }),
        "{recovery:?}"
    );
    let ShellReply::Opened { session, .. } = k
        .shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::OpenSavedVersion,
        })
        .unwrap()
    else {
        panic!("the saved version did not open");
    };
    k.shell(ShellCall::Close(session)).unwrap();
    drop(k);
    let discarded = folder.join("discarded");
    for s in [&s1, &s2] {
        assert!(
            discarded
                .join(s.file_name().unwrap())
                .join("journal-00000000.log")
                .exists(),
            "{s:?} kept beside the other"
        );
    }
}

/// Review w32, finding 5. Work recovered into a copy is durable in the
/// copy: the journal it came from is retired, not kept in `discarded/`.
#[test]
fn a_materialised_copy_retires_its_source_journal() {
    let root = TempDir::new("recovery-copy-retired");
    let path = saved_file(&root, value());
    let folder = root.path("journals/file");
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        rename(&mut k, &id, "Edited");
        k.durability_barrier().unwrap();
    }
    let crashed = journal::sessions(&folder).pop().unwrap();
    std::fs::remove_file(&path).unwrap();
    let mut other = value();
    other
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("Elsewhere"));
    let mut store = DocumentStore::new_untitled(other);
    common::save_as(&mut store, &path).unwrap();
    drop(store);
    let mut k = kernel(&root.0);
    let (id, offered) = launch(&mut k, &path, RecoveryDecision::OpenAsCopy);
    let id = id.unwrap_or_else(|| panic!("{offered}"));
    assert!(offered.starts_with("OfferCopy"), "{offered}");
    assert_eq!(name(&k, &id), "Edited — Recovered");
    assert!(!crashed.exists(), "the source session is retired");
    assert!(
        !folder
            .join("discarded")
            .join(crashed.file_name().unwrap())
            .exists(),
        "not archived"
    );
}

/// Review w32 re-check, finding 1. An edit fully journalled, then a layer
/// name with an emoji whose append a machine crash cut inside the emoji's
/// bytes, before the session's first checkpoint. The header is intact: the
/// session is offered, with the complete edit, not skipped (and later
/// deleted by a clean close).
#[test]
fn a_tail_torn_inside_a_character_hides_nothing() {
    let root = TempDir::new("recovery-torn-utf8");
    let path = saved_file(&root, value());
    let folder = root.path("journals/file");
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        rename(&mut k, &id, "Complete");
        rename(&mut k, &id, "Torn \u{1F3AC} scene");
        k.durability_barrier().unwrap();
    }
    kill_before_first_checkpoint(&folder);
    let session = journal::sessions(&folder).pop().unwrap();
    let segment = session.join("journal-00000000.log");
    let bytes = std::fs::read(&segment).unwrap();
    let emoji = "\u{1F3AC}".as_bytes();
    let at = bytes
        .windows(emoji.len())
        .position(|w| w == emoji)
        .expect("the emoji in the journal");
    std::fs::write(&segment, &bytes[..at + 2]).unwrap();
    let mut k = kernel(&root.0);
    let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
    let id = id.unwrap_or_else(|| panic!("{offered}"));
    assert_eq!(offered, "Recovered { truncated: true }");
    assert_eq!(name(&k, &id), "Complete");
}

/// Review w32 re-check, finding 3. Saved A; an edit, killed; Recover,
/// another edit, killed before that session's first checkpoint (it
/// continues the first). Another writer saves the file with the same
/// document A and its own history. Replaying from the predecessor must not
/// bring back the predecessor's hash-only base: offered as a copy.
#[test]
fn a_continued_session_matches_only_its_own_bases() {
    let root = TempDir::new("recovery-continued-bases");
    let path = saved_file(&root, value());
    let folder = root.path("journals/file");
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        rename(&mut k, &id, "R");
        k.durability_barrier().unwrap();
    }
    let kept = keep(&journal::sessions(&folder).pop().unwrap(), &root);
    {
        let mut k = kernel(&root.0);
        let (id, offered) = launch(&mut k, &path, RecoveryDecision::Recover);
        let id = id.unwrap_or_else(|| panic!("{offered}"));
        rename(&mut k, &id, "T");
        k.durability_barrier().unwrap();
    }
    kill_before_first_checkpoint(&folder);
    kept.put_back();
    let original = root.path("original-copy.pmv");
    std::fs::copy(&path, &original).unwrap();
    other_writers_file(&path, &original);
    let mut k = kernel(&root.0);
    let (_, offered) = launch(&mut k, &path, RecoveryDecision::OpenAsCopy);
    assert!(offered.starts_with("OfferCopy"), "{offered}");
}

/// PM-360. A library project that opens as a copy each time (here: its
/// file is missing on this Mac, as for a "— Native" project made from an
/// original-app project): opened, edited, saved, renamed, killed before
/// the save's journal checkpoint landed. Its journal was kept with the copy
/// that open made, which no later open looks at: the next open asked
/// nothing and the rename was lost. And the save's journal took the copy's
/// saved state, which the next open does not read, as its base: replayed
/// onto the project that opens, it could not be recovered. Now the next
/// open of the same project asks, and Recover brings the rename back.
#[test]
fn a_project_opened_as_a_copy_recovers_on_the_next_open() {
    let root = TempDir::new("recovery-as-copy");
    {
        let w =
            pm_kernel::persistence::LibraryWriter::open(&root.0, DiskFormat::V1_2, None).unwrap();
        let mut project = value();
        let o = project.as_object_mut().unwrap();
        o.insert("id", Value::from("sync"));
        o.insert("name", Value::from("Sync — Native"));
        let meta = pm_model::json::parse(&format!(
            r#"{{"file":{{"path":"{}"}}}}"#,
            root.path("gone/Sync.pmv").display()
        ))
        .unwrap();
        w.save_project("sync", project, Value::Null, Value::Null, meta)
            .unwrap();
    }
    let kept;
    let open = |k: &mut Kernel| {
        k.shell(ShellCall::PrepareOpen {
            id: "sync".into(),
            source: SessionSource::Library {
                profile: root.0.clone(),
                project: "sync".into(),
            },
            electron_profile: None,
        })
        .unwrap()
    };
    {
        let mut k = kernel(&root.0);
        let ShellReply::Opened {
            session, decision, ..
        } = open(&mut k)
        else {
            panic!("not opened")
        };
        assert!(
            matches!(decision, pm_kernel::locks::OpenDecision::AsCopy(_)),
            "{decision:?}"
        );
        rename(&mut k, &session.project, "Before saving");
        k.durability_barrier().unwrap();
        let before_save = journal::sessions(&root.path("journals/sync"))
            .pop()
            .unwrap();
        kept = keep(&before_save, &root);
        k.shell(ShellCall::Save(session.clone())).unwrap();
        rename(&mut k, &session.project, "Renamed after saving");
        k.durability_barrier().unwrap();
    }
    // Killed a moment after the rename: the session the save started has
    // no checkpoint yet, and the one before it is still there.
    kill_before_first_checkpoint(&root.path("journals/sync"));
    kept.put_back();
    let mut k = kernel(&root.0);
    let reply = open(&mut k);
    let ShellReply::RecoveryRequired { ticket, recovery } = reply else {
        panic!("no recovery question: {reply:?}");
    };
    let ShellReply::Opened { session, .. } = k
        .shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::Recover,
        })
        .unwrap()
    else {
        panic!("{recovery:?} did not open");
    };
    // Recovered in the copy the save went to, which the next open of the
    // project takes up again (PM-379).
    assert!(
        name(&k, &session.project).starts_with("Renamed after saving"),
        "{}",
        name(&k, &session.project)
    );
}

/// PM-379. A library project that opens as a copy (its file is missing on
/// this Mac: a "— Native" project made from an original-app project) is
/// edited, saved and closed, and the app quits. Opened again the way the
/// app reopens it (the entry the person opened), it made another copy of the
/// entry and showed the state from before the save; the save was only in the
/// copy the first open made. Now the open takes up the copy made last time:
/// the saved edit comes back, and however often it is opened Home lists the
/// entry and that one copy.
#[test]
fn a_project_opened_as_a_copy_reopens_with_its_saved_edits() {
    let root = TempDir::new("reopen-as-copy");
    {
        let w =
            pm_kernel::persistence::LibraryWriter::open(&root.0, DiskFormat::V1_2, None).unwrap();
        let mut project = value();
        let o = project.as_object_mut().unwrap();
        o.insert("id", Value::from("sync"));
        o.insert("name", Value::from("Sync — Native"));
        let meta = pm_model::json::parse(&format!(
            r#"{{"file":{{"path":"{}"}}}}"#,
            root.path("gone/Sync.pmv").display()
        ))
        .unwrap();
        w.save_project("sync", project, Value::Null, Value::Null, meta)
            .unwrap();
    }
    let open = |k: &mut Kernel| {
        let ShellReply::Opened {
            session, decision, ..
        } = k
            .shell(ShellCall::PrepareOpen {
                id: "sync".into(),
                source: SessionSource::Library {
                    profile: root.0.clone(),
                    project: "sync".into(),
                },
                electron_profile: None,
            })
            .unwrap()
        else {
            panic!("not opened")
        };
        assert!(
            matches!(decision, pm_kernel::locks::OpenDecision::AsCopy(_)),
            "{decision:?}"
        );
        session
    };
    let entries = || {
        let mut rows: Vec<String> = pm_kernel::library::LibraryStore::open(&root.0)
            .projects()
            .unwrap()
            .into_iter()
            .map(|row| format!("{} {}", row.id, row.name))
            .collect();
        rows.sort();
        rows
    };
    {
        let mut k = kernel(&root.0);
        let session = open(&mut k);
        assert_eq!(width(&k, &session.project), 320.);
        widen(&mut k, &session.project, 640.);
        k.shell(ShellCall::Save(session.clone())).unwrap();
        k.shell(ShellCall::Close(session)).unwrap();
    }
    let after_save = entries();
    eprintln!("after the save: {after_save:?}");
    for reopen in 1..=2 {
        let mut k = kernel(&root.0);
        let session = open(&mut k);
        assert_eq!(
            width(&k, &session.project),
            640.,
            "open {reopen} after the save shows the state before it"
        );
        k.shell(ShellCall::Close(session)).unwrap();
        assert_eq!(entries(), after_save, "open {reopen} made another entry");
    }
    assert_eq!(after_save.len(), 2, "{after_save:?}");
    assert!(
        after_save.iter().any(|e| e == "sync Sync — Native"),
        "{after_save:?}"
    );
    // Work not saved in that copy, then a forced quit: the next open asks,
    // and Recover brings it back in the same copy.
    {
        let mut k = kernel(&root.0);
        let session = open(&mut k);
        widen(&mut k, &session.project, 800.);
        k.durability_barrier().unwrap();
    }
    let mut k = kernel(&root.0);
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "sync".into(),
            source: SessionSource::Library {
                profile: root.0.clone(),
                project: "sync".into(),
            },
            electron_profile: None,
        })
        .unwrap();
    let ShellReply::RecoveryRequired { ticket, .. } = reply else {
        panic!("no recovery question: {reply:?}");
    };
    let ShellReply::Opened { session, .. } = k
        .shell(ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::Recover,
        })
        .unwrap()
    else {
        panic!("not opened");
    };
    assert_eq!(width(&k, &session.project), 800.);
    k.shell(ShellCall::Save(session.clone())).unwrap();
    k.shell(ShellCall::Close(session)).unwrap();
    assert_eq!(entries(), after_save, "the recovery made another entry");
}
