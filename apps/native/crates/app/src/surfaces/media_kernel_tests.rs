use super::*;
use pm_kernel::{HistoryCall, KernelActor, KernelConfig, OpenSource, Reply};
use pm_model::json::parse;
const PROJECT: &str = r#"{"id":"P","name":"Test","w":640,"h":360,"fps":30,"dur":20,"layers":[{"id":"v","type":"video","from":2,"dur":8,"d":{"asset":"a","trimStart":3}}],"assets":{"a":{"id":"a","name":"old.mov","kind":"video","folder":"bin","extensionData":{"keep":true},"originalVersion":"archived-old"}}}"#;
fn setup() -> (KernelActor, KernelHandle, Target) {
    let actor = KernelActor::spawn(KernelConfig::default());
    let h = actor.handle();
    h.open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let p = h.published("P").unwrap();
    let target = Target {
        owner: owner("window".into(), &p),
        id: "a".into(),
        generation: 1,
    };
    (actor, h, target)
}
fn asset(id: &str) -> ArchivedAsset {
    ArchivedAsset {
        id: id.into(),
        metadata: parse(r#"{"name":"new.mov","kind":"video","folder":"other"}"#).unwrap(),
        original_version: "archived-new".into(),
    }
}
#[test]
fn replacement_keeps_id_trim_timing_unknown_fields_and_prior_bytes_receipt_in_undo() {
    let (actor, h, t) = setup();
    let p = h.published("P").unwrap();
    let before = p.snapshot.document.to_value().unwrap();
    let request = admission(&t, &p, vec![asset("a")], true).unwrap();
    let ack = submit(&h, &t, request).wait().unwrap().unwrap();
    assert!(matches!(ack.reply, Reply::Applied { changed: true, .. }));
    let value = h
        .published("P")
        .unwrap()
        .snapshot
        .document
        .to_value()
        .unwrap();
    assert_eq!(value.get("layers"), before.get("layers"));
    let a = value.get("assets").unwrap().get("a").unwrap();
    assert_eq!(a.get("id").and_then(Value::as_str), Some("a"));
    assert_eq!(a.get("folder").and_then(Value::as_str), Some("bin"));
    assert_eq!(
        a.get("extensionData"),
        before
            .get("assets")
            .unwrap()
            .get("a")
            .unwrap()
            .get("extensionData")
    );
    h.call("P", Request::new(Caller::native(), HistoryCall::Undo))
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(
        h.published("P")
            .unwrap()
            .snapshot
            .document
            .to_value()
            .unwrap(),
        before
    );
    actor.shutdown();
}
#[test]
fn completed_batch_is_one_import_media_undo_step() {
    let (actor, h, t) = setup();
    let p = h.published("P").unwrap();
    let before = p.snapshot.document.to_value().unwrap();
    let request = admission(&t, &p, vec![asset("b"), asset("c")], false).unwrap();
    submit(&h, &t, request).wait().unwrap().unwrap();
    assert_eq!(
        h.published("P").unwrap().history.undo_label.as_deref(),
        Some("Import Media")
    );
    h.call("P", Request::new(Caller::native(), HistoryCall::Undo))
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(
        h.published("P")
            .unwrap()
            .snapshot
            .document
            .to_value()
            .unwrap(),
        before
    );
    actor.shutdown();
}
#[test]
fn admission_refuses_missing_archive_duplicate_ids_and_replacement_of_another_asset() {
    let (actor, h, t) = setup();
    let p = h.published("P").unwrap();
    let mut a = asset("b");
    a.original_version.clear();
    assert!(matches!(
        admission(&t, &p, vec![a], false),
        Err(AdmissionError::ArchiveRequired)
    ));
    assert!(matches!(
        admission(&t, &p, vec![asset("b"), asset("b")], false),
        Err(AdmissionError::InvalidAsset)
    ));
    assert!(matches!(
        admission(&t, &p, vec![asset("b")], true),
        Err(AdmissionError::InvalidAsset)
    ));
    assert!(matches!(
        admission(&t, &p, vec![asset("a")], false),
        Err(AdmissionError::InvalidAsset)
    ));
    actor.shutdown();
}
#[test]
fn reopening_project_rejects_stale_worker_admission_atomically() {
    let (actor, h, t) = setup();
    let p = h.published("P").unwrap();
    let request = admission(&t, &p, vec![asset("b")], false).unwrap();
    h.close("P").wait().unwrap();
    h.open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let ack = submit(&h, &t, request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, Reply::Applied { changed: true, .. }));
    assert!(matches!(
        admission(&t, &h.published("P").unwrap(), vec![asset("b")], false),
        Err(AdmissionError::Stale)
    ));
    assert!(
        h.published("P")
            .unwrap()
            .snapshot
            .document
            .to_value()
            .unwrap()
            .get("assets")
            .unwrap()
            .get("b")
            .is_none()
    );
    actor.shutdown();
}
#[test]
fn changed_revision_refuses_delayed_worker_request() {
    let (actor, h, t) = setup();
    let p = h.published("P").unwrap();
    let first = admission(&t, &p, vec![asset("b")], false).unwrap();
    let delayed = admission(&t, &p, vec![asset("c")], false).unwrap();
    submit(&h, &t, first).wait().unwrap().unwrap();
    let ack = submit(&h, &t, delayed).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, Reply::Applied { changed: true, .. }));
    assert!(
        h.published("P")
            .unwrap()
            .snapshot
            .document
            .to_value()
            .unwrap()
            .get("assets")
            .unwrap()
            .get("c")
            .is_none()
    );
    actor.shutdown();
}
