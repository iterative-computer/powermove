//! The actor: async calls, publication before events, the UI query mirror,
//! and continuations to another realm's mailbox.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use pm_kernel::{
    Call, Caller, Command, HistoryCall, KernelActor, KernelConfig, KernelEvent, OpenSource, Origin,
    QueryMirror, RealmMessage, Reply, Request, WriteBody, Writer,
};
use pm_model::json::parse;

const PROJECT: &str = r##"{
  "id": "P", "name": "Fixture", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 2, "bg": "#000000", "assets": {},
  "layers": [
    {"id": "layer0", "type": "shape", "name": "Layer", "from": 0, "dur": 2, "on": true,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}}
  ],
  "comps": {}
}"##;

fn opacity(v: f64) -> Call {
    Call::Transact {
        label: None,
        body: WriteBody::Commands(vec![Command::SetStaticOpacity {
            layer_id: "layer0".into(),
            value: v,
        }]),
        expected_revision: None,
    }
}

/// A minimal executor: the future must complete without blocking the
/// polling thread (we park between polls, as a run loop would).
fn block_on<F: Future>(f: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
}

#[test]
fn calls_are_futures_and_publication_precedes_events() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let events = kernel.subscribe();
    block_on(kernel.open("P", OpenSource::Untitled(parse(PROJECT).unwrap())))
        .unwrap()
        .unwrap();
    let rev = kernel.published("P").unwrap().revision();
    let ack = block_on(kernel.call("P", Request::new(Caller::native(), opacity(40.0))))
        .unwrap()
        .unwrap();
    assert_eq!(ack.revision, rev + 1);
    // The published state already reflects the ack.
    let published = kernel.published("P").unwrap();
    assert_eq!(published.revision(), ack.revision);
    assert!(published.history.can_undo);
    assert!(published.dirty);
    let mut saw = Vec::new();
    while let Ok(e) = events.recv_timeout(Duration::from_millis(200)) {
        if let KernelEvent::Published { revision, .. } = e {
            // Any event's revision is already readable.
            assert!(kernel.published("P").unwrap().revision() >= revision);
            saw.push(revision);
        }
        if saw.contains(&ack.revision) {
            break;
        }
    }
    assert!(saw.contains(&ack.revision));
    // Unknown project: an error, not a hang.
    assert!(
        block_on(kernel.call("nope", Request::new(Caller::native(), opacity(1.0))))
            .unwrap()
            .is_err()
    );
}

#[test]
fn the_ui_query_mirror_follows_published_changes() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    block_on(kernel.open("P", OpenSource::Untitled(parse(PROJECT).unwrap())))
        .unwrap()
        .unwrap();
    let mut mirror = QueryMirror::new(&kernel.published("P").unwrap());
    let opacity_now = |m: &mut QueryMirror| {
        m.queries()
            .evaluate(None, "layer0", "opacity", 0.0)
            .unwrap()
            .as_f64()
            .unwrap()
    };
    assert_eq!(opacity_now(&mut mirror), 100.0);
    for v in [10.0, 20.0, 30.0] {
        kernel
            .call_blocking("P", Request::new(Caller::native(), opacity(v)))
            .unwrap();
    }
    let published = kernel.published("P").unwrap();
    assert_eq!(mirror.sync(&published), published.revision());
    assert_eq!(
        opacity_now(&mut mirror),
        30.0,
        "synchronous, on the caller's thread"
    );
}

#[test]
fn external_undo_runs_in_the_owning_realm_and_resolves_the_caller() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    block_on(kernel.open("P", OpenSource::Untitled(parse(PROJECT).unwrap())))
        .unwrap()
        .unwrap();
    let page = Caller::realm(1, 1, Origin::Extension);
    let mailbox = kernel.register_realm(Writer {
        realm: 1,
        generation: 1,
    });
    kernel
        .call_blocking(
            "P",
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
    // The native menu's Undo: deferred until the page runs handle 42.
    let undo = kernel.call("P", Request::new(Caller::native(), HistoryCall::Undo));
    let envelope = mailbox.recv_timeout(Duration::from_secs(5)).unwrap();
    let RealmMessage::RunExternal { continuation, .. } = envelope.message else {
        panic!()
    };
    assert_eq!(continuation.handle, 42);
    assert!(undo.try_take().is_none(), "still waiting on the realm");
    // Meanwhile the kernel keeps serving everyone (no lease is held).
    kernel
        .call_blocking("P", Request::new(Caller::native(), opacity(5.0)))
        .unwrap();
    kernel
        .call_blocking(
            "P",
            Request::new(
                page,
                Call::ExternalDone {
                    ticket: continuation.ticket,
                    ok: true,
                    message: None,
                },
            ),
        )
        .unwrap();
    let ack = undo.wait().unwrap().unwrap();
    assert!(matches!(ack.reply, Reply::Bool(true)));
}

#[test]
fn shutdown_closes_pending_handles() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    actor.shutdown();
    let after = kernel.call("P", Request::new(Caller::native(), opacity(1.0)));
    assert!(
        after.wait().is_none(),
        "a stopped kernel answers None, never hangs"
    );
}
