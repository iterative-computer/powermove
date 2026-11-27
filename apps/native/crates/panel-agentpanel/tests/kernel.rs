//! Real pm-kernel actor, not a second DocumentStore. The lease double models a
//! host holding project lifecycle stable; concurrent document writers remain free.
use pm_kernel::{
    Call, Caller, Command, HistoryCall, KernelActor, KernelConfig, OpenSource, Origin, Reply,
    Request, SelectionCall, WriteBody,
};
use pm_model::json::parse;
use pm_panel_agentpanel::{
    Error,
    kernel::{DocumentLease, ProjectModel},
};
use std::{
    future::Future,
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};
const PROJECT: &str = r##"{"id":"P","name":"Synthetic","compId":"C1","compName":"Main","w":200,"h":100,"fps":30,"dur":2,"bg":"#000000","assets":{},"layers":[{"id":"layer","type":"shape","name":"Shape","from":0,"dur":2,"on":true,"p":{"opacity":{"v":100,"kf":[],"expr":null}}}],"comps":{}}"##;
fn block_on<F: Future>(f: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let w = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&w);
    let mut f = pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
}
struct Lease {
    document: u64,
    dropped: Arc<AtomicBool>,
}
impl DocumentLease for Lease {
    fn document_id(&self) -> u64 {
        self.document
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}
fn lease(m: &ProjectModel) -> Lease {
    Lease {
        document: m.document_id(),
        dropped: Arc::new(AtomicBool::new(false)),
    }
}
fn fixture() -> (KernelActor, ProjectModel) {
    let actor = KernelActor::spawn(KernelConfig::default());
    block_on(
        actor
            .handle()
            .open("P", OpenSource::Untitled(parse(PROJECT).unwrap())),
    )
    .unwrap()
    .unwrap();
    let model = ProjectModel::new(actor.handle(), "P".into(), Caller::native()).unwrap();
    (actor, model)
}
fn opacity(id: &str, value: f64) -> Command {
    Command::SetStaticOpacity {
        layer_id: id.into(),
        value,
    }
}
fn value(m: &mut ProjectModel) -> f64 {
    m.queries()
        .unwrap()
        .evaluate(None, "layer", "opacity", 0.0)
        .unwrap()
        .as_f64()
        .unwrap()
}
#[test]
fn atomic_batch_one_history_step_selection_and_query_mirror() {
    let (_actor, mut m) = fixture();
    let rev = m.published().unwrap().revision();
    let l = lease(&m);
    let dropped = l.dropped.clone();
    assert!(!dropped.load(Ordering::SeqCst));
    let ack = block_on(m.transact(
        l,
        vec![opacity("layer", 50.0), opacity("layer", 25.0)],
        "Synthetic edit".into(),
        rev,
        1,
    ))
    .unwrap();
    assert!(dropped.load(Ordering::SeqCst));
    assert!(matches!(ack.reply, Reply::Applied { changed: true, .. }));
    assert_eq!(value(&mut m), 25.0);
    assert_eq!(m.published().unwrap().history.entries, 1);
    block_on(m.select(
        lease(&m),
        SelectionCall::Layers {
            ids: vec!["layer".into()],
            add: false,
        },
    ))
    .unwrap();
    assert_eq!(m.published().unwrap().selection.layers(), &["layer"]);
    block_on(m.history(lease(&m), false)).unwrap();
    assert_eq!(value(&mut m), 100.0);
    block_on(m.history(lease(&m), true)).unwrap();
    assert_eq!(value(&mut m), 25.0);
}
#[test]
fn invalid_second_command_and_stale_revision_publish_nothing() {
    let (_actor, mut m) = fixture();
    let rev = m.published().unwrap().revision();
    assert!(
        block_on(m.transact(
            lease(&m),
            vec![opacity("layer", 20.0), opacity("missing", 20.0)],
            "Invalid".into(),
            rev,
            1
        ))
        .is_err()
    );
    assert_eq!(m.published().unwrap().revision(), rev);
    assert_eq!(value(&mut m), 100.0);
    block_on(m.transact(
        lease(&m),
        vec![opacity("layer", 70.0)],
        "Valid".into(),
        rev,
        2,
    ))
    .unwrap();
    assert!(
        block_on(m.transact(
            lease(&m),
            vec![opacity("layer", 10.0)],
            "Stale".into(),
            rev,
            3
        ))
        .is_err()
    );
    assert_eq!(value(&mut m), 70.0);
}
#[test]
fn retry_txid_deduplicates_even_after_revision_changed() {
    let (_actor, m) = fixture();
    let rev = m.published().unwrap().revision();
    let first = block_on(m.transact(
        lease(&m),
        vec![opacity("layer", 45.0)],
        "Change".into(),
        rev,
        99,
    ))
    .unwrap();
    let retry = block_on(m.transact(
        lease(&m),
        vec![opacity("layer", 45.0)],
        "Change".into(),
        rev,
        99,
    ))
    .unwrap();
    assert_eq!(first.revision, retry.revision);
    assert_eq!(m.published().unwrap().history.entries, 1);
}
#[test]
fn native_batch_folds_into_open_gesture_without_blocking_the_actor() {
    let (actor, mut m) = fixture();
    let k = actor.handle();
    let foreign = Caller::realm(40, 1, Origin::Extension);
    block_on(k.call(
        "P",
        Request::new(
            foreign,
            HistoryCall::Begin {
                label: "Gesture".into(),
            },
        ),
    ))
    .unwrap()
    .unwrap();
    let rev = m.published().unwrap().revision();
    let ack = block_on(m.transact(
        lease(&m),
        vec![opacity("layer", 33.0)],
        "Folded".into(),
        rev,
        1,
    ))
    .unwrap();
    assert!(ack.folded.is_some());
    assert_eq!(value(&mut m), 33.0);
    block_on(k.call(
        "P",
        Request::new(foreign, HistoryCall::Commit { label: None }),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(m.published().unwrap().history.entries, 1);
}
#[test]
fn result_undo_is_atomic_and_refuses_interleaved_foreign_work() {
    let (actor, mut m) = fixture();
    let before = m.published().unwrap().snapshot.clone();
    let ack = block_on(m.transact(
        lease(&m),
        vec![opacity("layer", 50.0)],
        "Result".into(),
        before.revision,
        1,
    ))
    .unwrap();
    block_on(actor.handle().call(
        "P",
        Request::new(
            Caller::realm(42, 1, Origin::Agent),
            Call::Transact {
                label: None,
                body: WriteBody::Commands(vec![opacity("layer", 77.0)]),
                expected_revision: Some(ack.revision),
            },
        ),
    ))
    .unwrap()
    .unwrap();
    assert!(
        block_on(m.undo_result(lease(&m), &before, ack.revision, "Undo result".into(), 2)).is_err()
    );
    assert_eq!(value(&mut m), 77.0);
}
#[test]
fn guarded_result_undo_restores_only_the_expected_document() {
    let (_actor, mut m) = fixture();
    let before = m.published().unwrap().snapshot.clone();
    let ack = block_on(m.transact(
        lease(&m),
        vec![opacity("layer", 50.0)],
        "Result".into(),
        before.revision,
        1,
    ))
    .unwrap();
    block_on(m.undo_result(lease(&m), &before, ack.revision, "Undo result".into(), 2)).unwrap();
    assert_eq!(value(&mut m), 100.0);
}
#[test]
fn reopened_project_and_wrong_lease_are_refused_before_edit() {
    let (actor, m) = fixture();
    let rev = m.published().unwrap().revision();
    let wrong = Lease {
        document: m.document_id() + 1,
        dropped: Arc::new(AtomicBool::new(false)),
    };
    assert_eq!(
        block_on(m.transact(wrong, vec![opacity("layer", 50.0)], "Wrong".into(), rev, 1))
            .unwrap_err(),
        Error::Stale
    );
    block_on(actor.handle().close("P")).unwrap();
    block_on(
        actor
            .handle()
            .open("P", OpenSource::Untitled(parse(PROJECT).unwrap())),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        block_on(m.transact(
            lease(&m),
            vec![opacity("layer", 50.0)],
            "Old".into(),
            rev,
            2
        ))
        .unwrap_err(),
        Error::Stale
    );
}
