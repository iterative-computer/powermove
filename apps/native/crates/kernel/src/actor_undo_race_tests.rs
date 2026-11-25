//! PM-718 through the real actor and real threads: a person's Undo sent
//! while an agent's edit is caught mid-call (held at a stage of the
//! transaction path, with barriers, never sleeps) or while its mutation
//! lease is open. The Undo always takes back exactly the newest finished
//! step, and the agent's edit is never cut or folded into.

use std::path::PathBuf;
use std::time::Instant;

use crate::Origin;
use crate::actor::{KernelActor, KernelHandle};
use crate::fault_injection::Stage;
use crate::persistence::DiskFormat;
use crate::session::{SessionSource, ShellCall, ShellReply};
use crate::tx::{Call, Caller, EditCall, HistoryCall, Reply, Request};
use crate::{Command, KernelConfig, WriteBody, journal};

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn agent() -> Caller {
    Caller::realm(41, 1, Origin::Mcp)
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

/// A project with the person's finished step (opacity 100 → 40).
fn fixture(id: &str) -> (Temp, KernelActor, KernelHandle) {
    let root = Temp(std::env::temp_dir().join(format!(
        "pm-kernel-undo-race-{id}-{}-{:?}",
        std::process::id(),
        Instant::now()
    )));
    std::fs::create_dir_all(&root.0).unwrap();
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(root.0.join("journals")),
        journal: journal::JournalConfig::default(),
        ..Default::default()
    });
    let kernel = actor.handle();
    kernel
        .shell(ShellCall::AttachLibrary {
            profile: root.0.clone(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap();
    let project = pm_model::json::parse(&format!(
        r##"{{"id":"{id}","name":"Fixture","compId":"C1","compName":"Main","w":200,"h":100,
        "fps":30,"dur":2,"bg":"#000000","assets":{{}},"comps":{{}},
        "layers":[{{"id":"layer0","type":"shape","name":"Layer","from":0,"dur":2,"on":true,
        "p":{{"opacity":{{"v":100,"kf":[],"expr":null}}}}}}]}}"##
    ))
    .unwrap();
    let opened = kernel
        .shell(ShellCall::Open {
            id: id.into(),
            source: SessionSource::New(project),
            electron_profile: None,
        })
        .wait()
        .unwrap();
    assert!(
        matches!(opened, Ok(ShellReply::Opened { .. })),
        "{opened:?}"
    );
    kernel
        .call_blocking(id, Request::new(Caller::native(), opacity(40.0)))
        .unwrap();
    (root, actor, kernel)
}

fn layer_opacity(kernel: &KernelHandle, id: &str) -> f64 {
    let doc = kernel
        .published(id)
        .unwrap()
        .snapshot
        .document
        .to_value()
        .unwrap();
    doc.get("comps")
        .and_then(|c| c.get("C1")?.get("layers")?.as_array()?.first().cloned())
        .and_then(|l| l.get("p")?.get("opacity")?.get("v")?.as_f64())
        .unwrap()
}

fn undo_label(kernel: &KernelHandle, id: &str) -> Option<String> {
    kernel.published(id).unwrap().history.undo_label.clone()
}

fn person(kernel: &KernelHandle, id: &str, call: HistoryCall) -> Reply {
    kernel
        .call_blocking(id, Request::new(Caller::native(), call))
        .unwrap()
        .reply
}

/// The agent's one-call edit (opacity 70) is held at `stage`; the person's
/// Undo is sent from another thread meanwhile. It runs only after the whole
/// call (applied, recorded, published), and takes back exactly that edit.
fn undo_racing_a_held_call(stage: Stage) {
    let id = format!("held{stage:?}").to_lowercase();
    let (_root, actor, kernel) = fixture(&id);
    let (reached, go) = kernel.hold_at(stage, &id);
    let edit = {
        let (kernel, id) = (kernel.clone(), id.clone());
        std::thread::spawn(move || kernel.call_blocking(&id, Request::new(agent(), opacity(70.0))))
    };
    reached.recv().expect("the agent's edit reached the stage");
    let undo = kernel.call(&id, Request::new(Caller::native(), HistoryCall::Undo));
    assert!(
        undo.try_take().is_none(),
        "{stage:?}: Undo ran inside the call"
    );
    go.send(()).unwrap();
    let edit = edit.join().unwrap().unwrap();
    assert!(
        matches!(edit.reply, Reply::Applied { .. }),
        "{stage:?}: {edit:?}"
    );
    let undo = undo.wait().unwrap().unwrap();
    assert!(
        matches!(undo.reply, Reply::Bool(true)),
        "{stage:?}: {undo:?}"
    );
    assert_eq!(
        layer_opacity(&kernel, &id),
        40.0,
        "{stage:?}: the step below kept"
    );
    assert_eq!(undo_label(&kernel, &id).as_deref(), Some("Opacity 40"));
    assert!(matches!(
        person(&kernel, &id, HistoryCall::Redo),
        Reply::Bool(true)
    ));
    assert_eq!(layer_opacity(&kernel, &id), 70.0, "{stage:?}");
    actor.shutdown();
}

#[test]
fn undo_sent_while_an_agent_edit_is_applied_but_not_yet_recorded_waits_for_it() {
    undo_racing_a_held_call(Stage::AfterApply);
}

#[test]
fn undo_sent_between_an_agent_edits_record_and_its_publication_waits_for_it() {
    undo_racing_a_held_call(Stage::AfterRecord);
}

#[test]
fn undo_sent_while_an_agent_edit_is_being_published_waits_for_it() {
    undo_racing_a_held_call(Stage::BeforePublish);
}

/// An agent's lease spans calls: the person's Undo, sent from the editor
/// while it is open, is answered only once the lease ends, and undoes it
/// whole. The publication says an edit is applying meanwhile.
#[test]
fn undo_sent_while_an_agent_lease_is_open_is_answered_after_the_lease_ends() {
    let id = "lease";
    let (_root, actor, kernel) = fixture(id);
    let Reply::Mutation(lease) = kernel
        .call_blocking(
            id,
            Request::new(
                agent(),
                EditCall::MutateBegin {
                    label: "Agent edit".into(),
                },
            ),
        )
        .unwrap()
        .reply
    else {
        panic!("no lease")
    };
    kernel
        .call_blocking(id, Request::new(agent(), opacity(70.0)))
        .unwrap();
    assert!(
        kernel
            .published(id)
            .unwrap()
            .entry
            .as_ref()
            .unwrap()
            .applying
    );

    let undo = kernel.call(id, Request::new(Caller::native(), HistoryCall::Undo));
    // The actor answers in order: once this write returns, the Undo was
    // handled, and it is waiting.
    kernel
        .call_blocking(id, Request::new(agent(), opacity(80.0)))
        .unwrap();
    assert!(
        undo.try_take().is_none(),
        "Undo ran inside the agent's edit"
    );
    assert_eq!(layer_opacity(&kernel, id), 80.0);

    let finished = kernel
        .call_blocking(
            id,
            Request::new(
                agent(),
                EditCall::MutateFinish {
                    lease,
                    ops: Vec::new(),
                    label: None,
                },
            ),
        )
        .unwrap();
    assert!(
        matches!(&finished.reply, Reply::Edit(r) if r.ok),
        "{finished:?}"
    );
    let undo = undo.wait().unwrap().unwrap();
    assert!(matches!(undo.reply, Reply::Bool(true)), "{undo:?}");
    let published = kernel.published(id).unwrap();
    assert!(published.entry.is_none());
    assert_eq!(
        layer_opacity(&kernel, id),
        40.0,
        "the whole agent edit undone"
    );
    assert_eq!(published.history.undo_label.as_deref(), Some("Opacity 40"));
    assert_eq!(published.history.redo_label.as_deref(), Some("Agent edit"));
    assert!(matches!(
        person(&kernel, id, HistoryCall::Redo),
        Reply::Bool(true)
    ));
    assert_eq!(layer_opacity(&kernel, id), 80.0);
    actor.shutdown();
}
