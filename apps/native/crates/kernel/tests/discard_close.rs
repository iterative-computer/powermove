//! Synthetic close/recovery fixtures; no real profile data.
mod common;
use common::TempDir;
use pm_kernel::{journal, persistence::DiskFormat, session::*, *};
use pm_model::Value;
use std::{fs, path::Path};

fn value() -> Value {
    pm_model::json::parse(r##"{"id":"p","name":"Base","compId":"c","w":320,"h":180,"fps":30,"dur":1,"bg":"#000000","layers":[],"assets":{},"comps":{}}"##).unwrap()
}
fn config(root: &Path) -> KernelConfig {
    KernelConfig {
        journal_root: Some(root.join("journals")),
        journal: journal::JournalConfig {
            checkpoint_revisions: 1,
            ..Default::default()
        },
    }
}
fn attach(k: &mut Kernel, root: &Path) {
    k.shell(ShellCall::AttachLibrary {
        profile: root.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
}
fn opened(reply: ShellReply) -> SessionHandle {
    match reply {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    }
}
fn fixture(root: &Path) -> (Kernel, SessionHandle) {
    let mut k = Kernel::new(config(root));
    attach(&mut k, root);
    let s = opened(
        k.shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::New(value()),
            electron_profile: None,
        })
        .unwrap(),
    );
    k.shell(ShellCall::Save(s.clone())).unwrap();
    (k, s)
}
fn edit(k: &mut Kernel) {
    let ack = k
        .handle(
            "p",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Rename".into()),
                    body: vec![
                        CompCommand::Set {
                            patch: Box::new(pm_kernel::commands::comps::CompPatch {
                                name: Some("Rejected edit".into()),
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
fn source(root: &Path, file: bool) -> SessionSource {
    if file {
        SessionSource::File(root.join("project.pmv"))
    } else {
        SessionSource::Library {
            profile: root.into(),
            project: "p".into(),
        }
    }
}

#[test]
fn lifecycle_defer_prepare_discard_defer_200_times() {
    for iteration in 0..200 {
        let root = TempDir::new("lifecycle-discard-race");
        let (mut k, s) = fixture(&root.0);
        let file = iteration % 2 == 1;
        if file {
            k.shell(ShellCall::SaveAs {
                session: s.clone(),
                path: root.path("project.pmv"),
                replace: false,
            })
            .unwrap();
        }
        edit(&mut k);
        k.shell(ShellCall::DeferClose(s)).unwrap();
        let reply = k
            .shell(ShellCall::PrepareOpen {
                id: "p".into(),
                source: source(&root.0, file),
                electron_profile: None,
            })
            .unwrap();
        let s = match reply {
            ShellReply::RecoveryRequired { ticket, .. } => opened(
                k.shell(ShellCall::DecideRecovery {
                    ticket,
                    decision: RecoveryDecision::Discard,
                })
                .unwrap(),
            ),
            other => panic!("iteration {iteration}: {other:?}"),
        };
        k.shell(ShellCall::DeferClose(s)).unwrap();
        k.shell(ShellCall::PrepareQuit).unwrap();
        k.durability_barrier().unwrap();
    }
}

fn assert_rejected(r: journal::Recovery, expected: &Value) {
    let r = match r {
        journal::Recovery::Recovered(r) | journal::Recovery::Copy { recovered: r, .. } => r,
        other => panic!("{other:?}"),
    };
    assert_eq!(&r.document.to_value().unwrap(), expected);
    assert!(
        pm_model::json::to_canonical_string(&r.history.snapshot().to_saved())
            .contains("Rejected edit")
    );
}

fn dont_save(file: bool) {
    let root = TempDir::new("discard-close");
    let (mut k, s) = fixture(&root.0);
    if file {
        k.shell(ShellCall::SaveAs {
            session: s.clone(),
            path: root.path("project.pmv"),
            replace: false,
        })
        .unwrap();
    }
    let base_path = if file {
        root.path("project.pmv")
    } else {
        root.path("store/project.p.json")
    };
    let base = fs::read(&base_path).unwrap();
    edit(&mut k);
    let expected = k
        .project("p")
        .unwrap()
        .store()
        .snapshot()
        .document
        .to_value()
        .unwrap();
    assert!(matches!(
        k.shell(ShellCall::DiscardClose(s.clone())).unwrap(),
        ShellReply::Closed {
            recovery_copy: None
        }
    ));
    assert!(k.project("p").is_none());
    assert!(
        k.shell(ShellCall::DiscardClose(s)).is_err(),
        "stale session must be refused"
    );
    assert_eq!(fs::read(&base_path).unwrap(), base);
    let project_dir = root.path("journals/p");
    assert!(journal::sessions(&project_dir).is_empty());
    let archive = project_dir.join("discarded");
    assert_rejected(journal::recover(&archive, None, None), &expected);
    assert!(matches!(
        k.shell(ShellCall::PrepareQuit).unwrap(),
        ShellReply::Compacted { projects: 0, .. }
    ));
    assert_eq!(fs::read(&base_path).unwrap(), base);
    drop(k);
    // Restart, quit and explicit preparation must never offer/resave this work.
    let mut k = Kernel::new(config(&root.0));
    attach(&mut k, &root.0);
    k.shell(ShellCall::PrepareQuit).unwrap();
    assert_eq!(fs::read(&base_path).unwrap(), base);
    assert!(matches!(
        k.recover("p", None, None),
        journal::Recovery::Nothing
    ));
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "p".into(),
            source: source(&root.0, file),
            electron_profile: None,
        })
        .unwrap();
    let reopened = opened(reply);
    k.shell(ShellCall::DiscardClose(reopened)).unwrap();
    // Restoring an archive is explicit: moving it out makes it recoverable.
    let rejected = journal::sessions(&archive)
        .into_iter()
        .find(|p| {
            fs::read_dir(p).unwrap().any(|e| {
                fs::read(e.unwrap().path())
                    .is_ok_and(|b| String::from_utf8_lossy(&b).contains("Rejected edit"))
            })
        })
        .unwrap();
    fs::rename(&rejected, project_dir.join(rejected.file_name().unwrap())).unwrap();
    assert_rejected(k.recover("p", None, None), &expected);
}

#[test]
fn dont_save_library_preserves_bytes_archive_and_quit() {
    dont_save(false);
}
#[test]
fn dont_save_file_preserves_bytes_archive_and_quit() {
    dont_save(true);
}

#[test]
fn discard_refuses_active_entry_pending_save_and_failed_archive() {
    let root = TempDir::new("discard-refusal");
    let (mut k, s) = fixture(&root.0);
    edit(&mut k);
    k.handle(
        "p",
        Request::new(
            Caller::native(),
            HistoryCall::Begin {
                label: "Drag".into(),
            },
        ),
    )
    .unwrap();
    assert!(k.shell(ShellCall::DiscardClose(s.clone())).is_err());
    k.handle(
        "p",
        Request::new(Caller::native(), HistoryCall::Commit { label: None }),
    )
    .unwrap();
    let save = k
        .prepare_session_save(s.clone(), Some(root.path("pending.pmv")))
        .unwrap();
    assert!(k.shell(ShellCall::DiscardClose(s.clone())).is_err());
    k.finish_session_save(save.run()).unwrap();
    let path = k.journal_dir("p").unwrap();
    let archive = path.parent().unwrap().join("discarded");
    fs::write(&archive, b"blocked").unwrap();
    assert!(k.shell(ShellCall::DiscardClose(s.clone())).is_err());
    assert!(k.project("p").is_some());
    assert!(path.is_dir());
    fs::remove_file(archive).unwrap();
    k.shell(ShellCall::DiscardClose(s)).unwrap();
}

#[test]
fn actor_discard_close_and_durability_barrier() {
    let root = TempDir::new("actor-discard");
    let actor = KernelActor::spawn(config(&root.0));
    let k = actor.handle();
    let answer = |call| k.shell(call).wait().unwrap().unwrap();
    answer(ShellCall::AttachLibrary {
        profile: root.0.clone(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    });
    let s = opened(answer(ShellCall::Open {
        id: "p".into(),
        source: SessionSource::New(value()),
        electron_profile: None,
    }));
    let base_path = root.path("store/project.p.json");
    let base = fs::read(&base_path).unwrap();
    k.call_blocking(
        "p",
        Request::new(
            Caller::native(),
            Call::Transact {
                label: Some("Rejected".into()),
                body: vec![
                    CompCommand::Set {
                        patch: Box::new(pm_kernel::commands::comps::CompPatch {
                            name: Some("Rejected edit".into()),
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
    .unwrap();
    assert!(matches!(
        answer(ShellCall::DurabilityBarrier),
        ShellReply::Durable
    ));
    assert!(matches!(
        answer(ShellCall::DiscardClose(s)),
        ShellReply::Closed {
            recovery_copy: None
        }
    ));
    assert!(k.published("p").is_none());
    answer(ShellCall::PrepareQuit);
    assert_eq!(fs::read(base_path).unwrap(), base);
    actor.shutdown();
}

const KILL_CHILD: &str = "PM_KERNEL_DISCARD_KILL_CHILD";
#[test]
fn sigkill_during_discard_keeps_journal_or_archive() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
        time::Duration,
    };
    if let Some(path) = std::env::var_os(KILL_CHILD) {
        let root = std::path::PathBuf::from(path);
        let (mut k, s) = fixture(&root);
        edit(&mut k);
        // Record the acknowledged state for independent parent-side replay.
        fs::write(
            root.join("expected.json"),
            pm_model::json::to_canonical_string(
                &k.project("p")
                    .unwrap()
                    .store()
                    .snapshot()
                    .document
                    .to_value()
                    .unwrap(),
            ),
        )
        .unwrap();
        println!("DISCARD");
        std::io::stdout().flush().unwrap();
        // Parent ACK puts the kill and close on opposite sides of a handshake.
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        k.shell(ShellCall::DiscardClose(s)).unwrap();
        println!("CLOSED");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
    // Exercise both sides of the atomic rename and the intervening fsyncs.
    let mut before = 0;
    let mut after = 0;
    for delay in [0, 0, 100, 250, 500, 1000, 2000, 4000, 8000, u64::MAX] {
        let root = TempDir::new("discard-kill");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sigkill_during_discard_keeps_journal_or_archive",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(KILL_CHILD, &root.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        assert!(
            lines
                .by_ref()
                .map_while(Result::ok)
                .any(|line| line.ends_with("DISCARD"))
        );
        let base = fs::read(root.path("store/project.p.json")).unwrap();
        if delay > 0 {
            writeln!(child.stdin.as_mut().unwrap(), "go").unwrap();
            if delay == u64::MAX {
                assert!(
                    lines
                        .by_ref()
                        .map_while(Result::ok)
                        .any(|line| line == "CLOSED")
                );
            } else {
                std::thread::sleep(Duration::from_micros(delay));
            }
        }
        child.kill().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        assert_eq!(fs::read(root.path("store/project.p.json")).unwrap(), base);
        let expected =
            pm_model::json::parse(&fs::read_to_string(root.path("expected.json")).unwrap())
                .unwrap();
        let project = root.path("journals/p");
        let live = journal::sessions(&project);
        let archived = journal::sessions(&project.join("discarded"));
        assert!(
            !live.is_empty() || !archived.is_empty(),
            "lost both journal and archive"
        );
        let location = if archived.is_empty() {
            before += 1;
            project
        } else {
            after += 1;
            project.join("discarded")
        };
        assert_rejected(
            journal::recover(
                &location,
                None,
                Some(&pm_kernel::model::to_canonical(value())),
            ),
            &expected,
        );
    }
    assert!(
        before > 0 && after > 0,
        "must kill before and after archive: {before}/{after}"
    );
}
