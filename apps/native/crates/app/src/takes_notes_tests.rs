//! Takes and Notes against a real kernel actor with a library attached: the
//! same worker functions the panels run.
use crate::{notes_panel, takes_panel};
use pm_kernel::{
    Call, Caller, HistoryCall, KernelActor, KernelConfig, OpenSource, Request,
    persistence::DiskFormat,
    session::{ShellCall, ShellReply},
};
use pm_services::surface_backing::{
    Session,
    projects::{NoteWrite, NotesHost},
};

fn actor(profile: &std::path::Path) -> KernelActor {
    let actor = KernelActor::spawn(KernelConfig::default());
    let attached = actor
        .handle()
        .shell(ShellCall::AttachLibrary {
            profile: profile.into(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait();
    assert!(matches!(attached, Some(Ok(ShellReply::Library(_)))));
    let project = pm_model::json::parse(
        r##"{"id":"doc-1","name":"Synthetic","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[]}"##,
    )
    .unwrap();
    actor
        .handle()
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    actor
}
fn notes(actor: &KernelActor) -> String {
    notes_panel::published_notes(&actor.handle().published("synthetic").unwrap())
}
fn write_notes(actor: &KernelActor, expected: &str, text: &str) -> pm_services::Result<()> {
    let p = actor.handle().published("synthetic").unwrap();
    notes_panel::Kernel(&actor.handle()).persist(&NoteWrite {
        ticket: test_ticket(),
        session: Session {
            project: "synthetic".into(),
            incarnation: p.snapshot.doc_id,
        },
        expected: expected.into(),
        text: text.into(),
    })
}
fn test_ticket() -> pm_services::surface_backing::Ticket {
    // A ticket only names a write; any fresh one does.
    let mut notes = pm_services::surface_backing::projects::Notes::default();
    let session = Session {
        project: "t".into(),
        incarnation: 1,
    };
    notes.attach(session.clone(), String::new());
    notes.input(&session, "x".into(), 0).unwrap();
    notes.flush(&session, 0, true).unwrap().unwrap().ticket
}

#[test]
fn notes_write_without_an_undo_step_and_refuse_a_stale_base() {
    let profile = tempfile::tempdir().unwrap();
    let actor = actor(profile.path());
    write_notes(&actor, "", "Warm grade, slow push-in.").unwrap();
    assert_eq!(notes(&actor), "Warm grade, slow push-in.");
    // Someone else changed them since the editor looked: refused, kept as is.
    assert!(write_notes(&actor, "", "Overwrite").is_err());
    assert_eq!(notes(&actor), "Warm grade, slow push-in.");
    // Notes are not an Undo step (v1): Undo has nothing to take back.
    let _ = actor
        .handle()
        .call(
            "synthetic",
            Request::new(Caller::native(), Call::History(HistoryCall::Undo)),
        )
        .wait();
    assert_eq!(notes(&actor), "Warm grade, slow push-in.");
}

#[test]
fn takes_save_list_restore_as_one_undo_step_and_delete() {
    let profile = tempfile::tempdir().unwrap();
    let actor = actor(profile.path());
    let kernel = actor.handle();
    write_notes(&actor, "", "First direction").unwrap();
    let p = kernel.published("synthetic").unwrap();
    assert_eq!(
        takes_panel::save(&kernel, &p, "Take 1".into()).unwrap(),
        "Take 1"
    );
    // A take of another project in the same library stays out of this list.
    let all = takes_panel::read(&kernel).unwrap();
    assert_eq!(all.len(), 1);
    // Takes follow the document's own id, not the session key.
    let rows = takes_panel::rows_for(&all, "doc-1");
    assert_eq!(rows.len(), 1);
    assert!(takes_panel::rows_for(&all, "other").is_empty());

    write_notes(&actor, "First direction", "Second direction").unwrap();
    let p = kernel.published("synthetic").unwrap();
    takes_panel::restore(&kernel, &p, &all, &rows[0]).unwrap();
    assert_eq!(notes(&actor), "First direction");
    // Restoring is one Undo step.
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), Call::History(HistoryCall::Undo)),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(notes(&actor), "Second direction");

    takes_panel::delete(&kernel, &rows[0]).unwrap();
    assert!(takes_panel::read(&kernel).unwrap().is_empty());
    // Takes outlive the kernel that saved them (the library store).
    let p = kernel.published("synthetic").unwrap();
    takes_panel::save(&kernel, &p, "Take 1".into()).unwrap();
    drop(kernel);
    actor.shutdown();
    let reopened = self::actor(profile.path());
    let all = takes_panel::read(&reopened.handle()).unwrap();
    assert_eq!(takes_panel::rows_for(&all, "doc-1").len(), 1);
}

#[test]
fn a_project_sees_its_own_takes_including_v1_ones() {
    let all: Vec<serde_json::Value> = serde_json::from_str(
        r#"[{"id":"t2","label":"Take 2","at":2,"json":"{}","project":"a"},
            {"id":"t1","label":"Take 1","at":1,"json":"{\"id\":\"a\"}"},
            {"id":"t0","label":"Other","at":0,"json":"{\"id\":\"b\"}"}]"#,
    )
    .unwrap();
    let rows = takes_panel::rows_for(&all, "a");
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["t2", "t1"]
    );
}
