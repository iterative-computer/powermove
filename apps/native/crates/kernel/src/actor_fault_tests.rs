//! A panic injected at each stage of the transaction path, through the
//! actor and the session calls the app uses: the project never keeps a half
//! applied edit. It stops (edits, saves, exports refused; its last
//! publication flagged), closing writes nothing, and reopening recovers
//! exactly the acknowledged work from its journal.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pm_model::json::Value;

use crate::Origin;
use crate::actor::{KernelActor, KernelEvent, KernelHandle};
use crate::fault_injection::Stage;
use crate::persistence::DiskFormat;
use crate::session::{RecoveryDecision, SessionHandle, SessionSource, ShellCall, ShellReply};
use crate::tx::{Call, Caller, HistoryCall, RealmMessage, Reply, Request, Writer};
use crate::{Command, KernelConfig, KernelFailure, WriteBody, journal};

struct Temp(PathBuf);
impl Temp {
    fn new(name: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!(
            "pm-kernel-fault-{name}-{}-{:?}",
            std::process::id(),
            Instant::now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn project(id: &str) -> Value {
    pm_model::json::parse(&format!(
        r##"{{"id":"{id}","name":"Fixture","compId":"C1","compName":"Main","w":200,"h":100,
        "fps":30,"dur":2,"bg":"#000000","assets":{{}},"comps":{{}},
        "layers":[{{"id":"layer0","type":"shape","name":"Layer","from":0,"dur":2,"on":true,
        "p":{{"opacity":{{"v":100,"kf":[],"expr":null}}}}}}]}}"##
    ))
    .unwrap()
}

fn opacity(v: f64) -> Call {
    Call::Transact {
        label: Some(format!("Opacity {v}")),
        body: WriteBody::Commands(vec![Command::SetStaticOpacity {
            layer_id: "layer0".into(),
            value: v,
        }]),
        expected_revision: None,
    }
}

fn shell(kernel: &KernelHandle, call: ShellCall) -> Result<ShellReply, KernelFailure> {
    kernel.shell(call).wait().expect("kernel answers")
}

fn opened(reply: Result<ShellReply, KernelFailure>) -> SessionHandle {
    match reply {
        Ok(ShellReply::Opened { session, .. }) => session,
        other => panic!("{other:?}"),
    }
}

/// A library project, saved, then one acknowledged edit (opacity 40).
fn fixture(root: &Path, id: &str) -> (KernelActor, KernelHandle, SessionHandle) {
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(root.join("journals")),
        journal: journal::JournalConfig {
            checkpoint_revisions: 1,
            ..Default::default()
        },
        ..Default::default()
    });
    let kernel = actor.handle();
    shell(
        &kernel,
        ShellCall::AttachLibrary {
            profile: root.into(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        },
    )
    .unwrap();
    let session = opened(shell(
        &kernel,
        ShellCall::Open {
            id: id.into(),
            source: SessionSource::New(project(id)),
            electron_profile: None,
        },
    ));
    shell(&kernel, ShellCall::Save(session.clone())).unwrap();
    kernel
        .call_blocking(id, Request::new(Caller::native(), opacity(40.0)))
        .unwrap();
    (actor, kernel, session)
}

fn document(kernel: &KernelHandle, id: &str) -> Value {
    kernel
        .published(id)
        .unwrap()
        .snapshot
        .document
        .to_value()
        .unwrap()
}

fn layer_opacity(doc: &Value) -> f64 {
    let text = pm_model::json::to_canonical_string(doc);
    let at = text.find("\"opacity\"").expect("opacity");
    let v = text[at..].find("\"v\":").unwrap() + at + 4;
    text[v..].split([',', '}']).next().unwrap().parse().unwrap()
}

fn refused<T>(r: Result<T, KernelFailure>) -> bool {
    matches!(r, Err(KernelFailure::Faulted(_)))
}

fn fault_event(events: &mpsc::Receiver<KernelEvent>) -> (String, Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left).expect("fault event") {
            KernelEvent::Fault { message, projects } => return (message, projects),
            _ => continue,
        }
    }
}

/// Injects a panic at `stage` while editing opacity 40 → 70 and proves the
/// project can't keep or persist a half-applied edit. `committed`: the
/// stage comes after the journal has the edit, so recovery includes it
/// whole (with its undo step); before that, recovery is the state before.
fn panic_at(stage: Stage, committed: bool) {
    let id = format!("fault{stage:?}").to_lowercase();
    let root = Temp::new(&id);
    let (_actor, kernel, session) = fixture(&root.0, &id);
    let before = kernel.published(&id).unwrap();
    let before_doc = document(&kernel, &id);
    assert_eq!(layer_opacity(&before_doc), 40.0);
    let events = kernel.subscribe();

    kernel.arm_fault(stage, &id);
    let answer = kernel
        .call(&id, Request::new(Caller::native(), opacity(70.0)))
        .wait();
    assert!(
        answer.is_none(),
        "{stage:?}: the interrupted call has no answer"
    );
    let (message, projects) = fault_event(&events);
    assert!(message.contains("injected fault"), "{message}");
    assert_eq!(projects, vec![id.clone()], "{stage:?}: the project stopped");

    // Its publication is the last one before the error, flagged.
    let published = kernel.published(&id).unwrap();
    assert!(published.fault.is_some(), "{stage:?}");
    assert_eq!(published.revision(), before.revision(), "{stage:?}");
    assert_eq!(document(&kernel, &id), before_doc, "{stage:?}");
    assert_eq!(published.history, before.history, "{stage:?}");
    assert_eq!(published.selection, before.selection, "{stage:?}");

    // Edits, selection, saves and exports are refused, never half applied.
    assert!(refused(kernel.call_blocking(
        &id,
        Request::new(Caller::native(), opacity(10.0))
    )));
    assert!(refused(kernel.call_blocking(
        &id,
        Request::new(Caller::native(), HistoryCall::Undo)
    )));
    assert!(refused(shell(&kernel, ShellCall::Save(session.clone()))));
    assert!(refused(shell(
        &kernel,
        ShellCall::SaveAs {
            session: session.clone(),
            path: root.0.join("copy.pmv"),
            replace: false,
        }
    )));
    assert!(refused(
        kernel
            .prepare_export(
                session.clone(),
                published.snapshot.doc_id,
                root.0.join("x.pmv")
            )
            .wait()
            .unwrap()
    ));
    assert!(!root.0.join("copy.pmv").exists());
    // Unchanged afterwards too: a later publication of every project (a
    // state transaction) keeps the stopped one frozen.
    kernel
        .state_transaction(crate::state::StateTransaction::default())
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(document(&kernel, &id), before_doc, "{stage:?}");

    // Quit compacts nothing of it; Close writes nothing and keeps the journal.
    shell(&kernel, ShellCall::PrepareQuit).unwrap();
    assert!(matches!(
        shell(&kernel, ShellCall::Close(session)),
        Ok(ShellReply::Closed {
            recovery_copy: None
        })
    ));
    assert!(kernel.published(&id).is_none());

    // Reopening offers recovery, and Recover brings back the acknowledged
    // work exactly.
    let reply = shell(
        &kernel,
        ShellCall::PrepareOpen {
            id: id.clone(),
            source: SessionSource::Library {
                profile: root.0.clone(),
                project: id.clone(),
            },
            electron_profile: None,
        },
    );
    let ticket = match reply {
        Ok(ShellReply::RecoveryRequired { ticket, .. }) => ticket,
        other => panic!("{stage:?}: {other:?}"),
    };
    opened(shell(
        &kernel,
        ShellCall::DecideRecovery {
            ticket,
            decision: RecoveryDecision::Recover,
        },
    ));
    let recovered = kernel.published(&id).unwrap();
    assert!(recovered.fault.is_none());
    if committed {
        assert_eq!(layer_opacity(&document(&kernel, &id)), 70.0, "{stage:?}");
        assert_eq!(recovered.history.entries, before.history.entries + 1);
        assert_eq!(recovered.history.undo_label.as_deref(), Some("Opacity 70"));
    } else {
        assert_eq!(document(&kernel, &id), before_doc, "{stage:?}");
        assert_eq!(recovered.history.entries, before.history.entries);
        assert_eq!(recovered.history.undo_label, before.history.undo_label);
    }
    // And it edits normally again.
    let ack = kernel
        .call_blocking(&id, Request::new(Caller::native(), opacity(20.0)))
        .unwrap();
    assert_eq!(kernel.published(&id).unwrap().revision(), ack.revision);
}

#[test]
fn panic_before_apply_stops_the_project() {
    panic_at(Stage::BeforeApply, false);
}

#[test]
fn panic_after_apply_before_history_never_keeps_the_edit() {
    panic_at(Stage::AfterApply, false);
}

#[test]
fn panic_after_history_before_journal_never_keeps_the_edit() {
    panic_at(Stage::AfterRecord, false);
}

#[test]
fn panic_after_journal_before_publication_recovers_the_whole_edit() {
    panic_at(Stage::BeforePublish, true);
}

#[test]
fn panic_in_publication_recovers_the_whole_edit() {
    panic_at(Stage::InPublish, true);
}

/// A caller that learns its call got no answer (its reply dropped) already
/// sees the project stopped: the answer goes only after the panic is
/// contained, never while the project still looks editable.
#[test]
fn an_unanswered_caller_already_sees_the_project_stopped() {
    struct Guard(KernelHandle, mpsc::Sender<bool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            let stopped = self
                .0
                .published("unanswered")
                .is_some_and(|p| p.fault.is_some());
            let _ = self.1.send(stopped);
        }
    }
    let root = Temp::new("unanswered");
    let (_actor, kernel, _session) = fixture(&root.0, "unanswered");
    let (tx, rx) = mpsc::channel();
    let guard = Guard(kernel.clone(), tx);
    kernel.arm_fault(Stage::AfterApply, "unanswered");
    kernel.call_with(
        "unanswered",
        Request::new(Caller::native(), opacity(70.0)),
        move |_| drop(guard),
    );
    let stopped = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the reply went");
    assert!(
        stopped,
        "the project was flagged before its caller heard nothing"
    );
}

/// Other open projects keep working when one stops.
#[test]
fn a_stopped_project_leaves_the_others_editable() {
    let root = Temp::new("others");
    let (_actor, kernel, _session) = fixture(&root.0, "stopped");
    let other = opened(shell(
        &kernel,
        ShellCall::Open {
            id: "other".into(),
            source: SessionSource::New(project("other")),
            electron_profile: None,
        },
    ));
    kernel.arm_fault(Stage::AfterApply, "stopped");
    assert!(
        kernel
            .call("stopped", Request::new(Caller::native(), opacity(70.0)))
            .wait()
            .is_none()
    );
    let ack = kernel
        .call_blocking("other", Request::new(Caller::native(), opacity(5.0)))
        .unwrap();
    assert_eq!(kernel.published("other").unwrap().revision(), ack.revision);
    shell(&kernel, ShellCall::Save(other)).unwrap();
}

/// A caller waiting on the stopped project (an Undo deferred to a realm)
/// gets the failure instead of waiting forever.
#[test]
fn deferred_answers_fail_when_their_project_stops() {
    let root = Temp::new("deferred");
    let (_actor, kernel, _session) = fixture(&root.0, "deferred");
    let page = Caller::realm(1, 1, Origin::Extension);
    let mailbox = kernel.register_realm(Writer {
        realm: 1,
        generation: 1,
    });
    kernel
        .call_blocking(
            "deferred",
            Request::new(
                page,
                HistoryCall::External {
                    label: "Replace media".into(),
                    handle: 42,
                    bytes: 0,
                    cleanup: false,
                },
            ),
        )
        .unwrap();
    let undo = kernel.call(
        "deferred",
        Request::new(Caller::native(), HistoryCall::Undo),
    );
    let envelope = mailbox.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(envelope.message, RealmMessage::RunExternal { .. }));
    kernel.arm_fault(Stage::AfterApply, "deferred");
    assert!(
        kernel
            .call("deferred", Request::new(Caller::native(), opacity(70.0)))
            .wait()
            .is_none()
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let answer = loop {
        if let Some(answer) = undo.try_take() {
            break answer;
        }
        assert!(
            Instant::now() < deadline,
            "the deferred Undo never answered"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        matches!(answer, Err(KernelFailure::Faulted(_))),
        "{answer:?}"
    );
}

/// A panic in the caller's own reply handler: the call committed, so the
/// rest of its turn still runs (publication event, journal), the project
/// stays editable and nothing stops.
#[test]
fn a_panicking_reply_handler_leaves_the_committed_edit_whole() {
    let root = Temp::new("reply");
    let (_actor, kernel, _session) = fixture(&root.0, "reply");
    let events = kernel.subscribe();
    kernel.call_with(
        "reply",
        Request::new(Caller::native(), opacity(70.0)),
        |_| panic!("host callback failed"),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut published, mut fault) = (None, None);
    while (published.is_none() || fault.is_none()) && Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(100)) {
            Ok(KernelEvent::Published { revision, .. }) => published = Some(revision),
            Ok(KernelEvent::Fault { message, projects }) => fault = Some((message, projects)),
            _ => {}
        }
    }
    let (message, projects) = fault.expect("the handler's panic is reported");
    assert!(message.contains("host callback failed"), "{message}");
    assert!(projects.is_empty(), "nothing was half applied");
    let published = published.expect("the committed edit's publication event");
    let current = kernel.published("reply").unwrap();
    assert_eq!(current.revision(), published);
    assert!(current.fault.is_none());
    assert_eq!(layer_opacity(&document(&kernel, "reply")), 70.0);
    assert_eq!(current.history.undo_label.as_deref(), Some("Opacity 70"));
    let ack = kernel
        .call_blocking("reply", Request::new(Caller::native(), opacity(10.0)))
        .unwrap();
    assert!(matches!(ack.reply, Reply::Applied { changed: true, .. }));
}
