use pm_surface::host_api::{
    HostAck, HostOperation, HostRequest, HostRouter, Identity, KernelQueue, Query,
};
use pm_surface::host_data::{HostDataService, KernelData, QuerySnapshot};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc::{self, SyncSender},
};

#[derive(Default)]
struct State {
    revision: u64,
    time: f64,
    queries: Vec<(Query, Option<Vec<String>>, Option<f64>)>,
    pending: Vec<(HostRequest, SyncSender<HostAck>)>,
    fail_query: bool,
    fail_queue: bool,
    fail_retire: bool,
    retired: usize,
}
struct KernelDouble(Rc<RefCell<State>>);
impl KernelData for KernelDouble {
    fn query(
        &mut self,
        _: &Identity,
        query: &Query,
        channels: Option<&[String]>,
        time: Option<f64>,
    ) -> Result<QuerySnapshot, String> {
        let mut s = self.0.borrow_mut();
        if s.fail_query {
            return Err("query unavailable".into());
        }
        s.queries
            .push((query.clone(), channels.map(<[String]>::to_vec), time));
        let value = match query {
            Query::Project => json!({"annotations": [], "unknown": false}),
            Query::Selection => json!(["layer"]),
            Query::Time => json!(s.time),
            Query::Evaluated => json!({"layer.x": -0.125}),
        };
        Ok(QuerySnapshot {
            revision: s.revision,
            value,
        })
    }
    fn enqueue(&mut self, request: HostRequest, reply: SyncSender<HostAck>) -> Result<(), String> {
        let mut s = self.0.borrow_mut();
        if s.fail_queue {
            return Err("actor queue full".into());
        }
        s.pending.push((request, reply));
        Ok(())
    }
    fn retire(&mut self, _: &Identity) -> Result<(), String> {
        let mut s = self.0.borrow_mut();
        if s.fail_retire {
            return Err("seal failed".into());
        }
        s.retired += 1;
        Ok(())
    }
}
fn identity() -> Identity {
    Identity {
        realm: "fixture".into(),
        project: "synthetic".into(),
        generation: 4,
    }
}
fn request(operation: HostOperation) -> HostRequest {
    serde_json::from_value(json!({"version":1,"realm":"fixture","generation":4,"project":"synthetic","requestId":1,"expectedRevision":3,"operation":operation})).unwrap()
}
fn query(query: Query) -> HostOperation {
    HostOperation::Query {
        query,
        channels: None,
        time: None,
    }
}
fn subscribe(id: &str, query: Query) -> HostOperation {
    HostOperation::Subscribe {
        subscription: id.into(),
        query,
        channels: None,
    }
}
fn setup() -> (HostDataService<KernelDouble>, Rc<RefCell<State>>) {
    let state = Rc::new(RefCell::new(State {
        revision: 3,
        ..State::default()
    }));
    (
        HostDataService::new(identity(), KernelDouble(state.clone())).unwrap(),
        state,
    )
}
fn send(service: &mut HostDataService<KernelDouble>, operation: HostOperation) -> HostAck {
    let (tx, rx) = mpsc::sync_channel(1);
    service.enqueue(request(operation), tx).unwrap();
    rx.try_recv().unwrap()
}
#[test]
fn queries_read_actual_publications_without_advancing_document_revision() {
    let (mut service, state) = setup();
    for q in [
        Query::Project,
        Query::Selection,
        Query::Time,
        Query::Evaluated,
    ] {
        let ack = send(&mut service, query(q));
        assert!(ack.accepted);
        assert_eq!(ack.revision, 3);
        assert!(ack.value.is_some());
    }
    let ack = send(
        &mut service,
        HostOperation::Query {
            query: Query::Evaluated,
            channels: Some(vec!["layer.x".into()]),
            time: Some(1.25),
        },
    );
    assert_eq!(ack.value.unwrap()["layer.x"], json!(-0.125));
    assert_eq!(
        state.borrow().queries.last().unwrap(),
        &(Query::Evaluated, Some(vec!["layer.x".into()]), Some(1.25))
    );
    assert!(state.borrow().pending.is_empty());
}
#[test]
fn subscribe_ack_is_initial_snapshot_and_time_publishes_at_same_revision() {
    let (mut service, state) = setup();
    assert_eq!(
        send(&mut service, subscribe("clock", Query::Time)).value,
        Some(json!(0.0))
    );
    assert!(service.publications().unwrap().is_empty());
    state.borrow_mut().time = 1.25;
    let publication = service.publications().unwrap().remove(0);
    assert_eq!(publication.value, json!(1.25));
    assert_eq!(publication.revision, 3);
    assert_eq!(publication.sequence, 1);
    assert_eq!(publication.generation, 4);
    assert_eq!(publication.subscription, "clock");
    assert!(service.publications().unwrap().is_empty());
    state.borrow_mut().revision = 4;
    assert_eq!(service.publications().unwrap()[0].sequence, 2);
    send(
        &mut service,
        HostOperation::Unsubscribe {
            subscription: "clock".into(),
        },
    );
    state.borrow_mut().time = 2.5;
    assert!(service.publications().unwrap().is_empty());
}
#[test]
fn edits_forward_exact_transaction_and_wait_for_actual_actor_ack() {
    let (service, state) = setup();
    let mut router = HostRouter::new(identity(), service).unwrap();
    let operation: HostOperation = serde_json::from_value(json!({"type":"edit","label":"Authored edit","commands":[{"command":"annotation.upsert","args":{"value":"日本🙂", "exact":false}},{"command":"select","args":[]}]})).unwrap();
    router.enqueue(request(operation.clone())).unwrap();
    assert_eq!(router.poll(1).unwrap(), None);
    assert_eq!(state.borrow().pending[0].0.operation, operation);
    assert!(router.enqueue(request(operation)).is_err());
    let (r, tx) = state.borrow_mut().pending.remove(0);
    tx.try_send(HostAck {
        version: 1,
        realm: r.realm,
        project: r.project,
        generation: r.generation,
        request_id: 1,
        revision: 3,
        accepted: false,
        error: Some("protected".into()),
        value: None,
        state_revision: None,
    })
    .unwrap();
    let ack = router.poll(1).unwrap().unwrap();
    assert!(!ack.accepted);
    assert_eq!(ack.error.as_deref(), Some("protected"));
    assert_eq!(router.poll(1).unwrap(), Some(ack));
}
#[test]
fn stale_generations_projects_and_future_queries_never_reach_backend() {
    let (mut service, state) = setup();
    let mut stale = request(query(Query::Project));
    stale.generation = 3;
    let mut foreign = request(query(Query::Project));
    foreign.realm = "other".into();
    let mut wrong_version = request(query(Query::Project));
    wrong_version.version = 2;
    for r in [stale, foreign, wrong_version] {
        let (tx, _) = mpsc::sync_channel(1);
        assert!(service.enqueue(r, tx).is_err());
    }
    let mut r = request(query(Query::Project));
    r.project = "other".into();
    let (tx, _) = mpsc::sync_channel(1);
    assert!(service.enqueue(r, tx).is_err());
    assert!(state.borrow().queries.is_empty());
    let mut r = request(query(Query::Project));
    r.expected_revision = 4;
    let (tx, rx) = mpsc::sync_channel(1);
    assert!(service.enqueue(r, tx).is_err());
    assert!(rx.try_recv().is_err());
}
#[test]
fn queue_and_query_failures_are_visible_and_retryable() {
    let (service, state) = setup();
    let mut router = HostRouter::new(identity(), service).unwrap();
    state.borrow_mut().fail_queue = true;
    let r = request(HostOperation::Command {
        command: "test".into(),
        args: Value::Null,
    });
    assert!(router.enqueue(r.clone()).is_err());
    state.borrow_mut().fail_queue = false;
    router.enqueue(r).unwrap();
    assert_eq!(state.borrow().pending.len(), 1);
    state.borrow_mut().fail_query = true;
    let mut r = request(query(Query::Project));
    r.request_id = 2;
    assert!(router.enqueue(r.clone()).is_err());
    state.borrow_mut().fail_query = false;
    router.enqueue(r).unwrap();
    assert!(router.poll(2).unwrap().unwrap().accepted);
}
#[test]
fn subscription_registration_is_atomic_on_reply_failure_and_duplicates_decline() {
    let (mut service, _) = setup();
    let (tx, rx) = mpsc::sync_channel(1);
    drop(rx);
    assert!(
        service
            .enqueue(request(subscribe("clock", Query::Time)), tx)
            .is_err()
    );
    assert!(send(&mut service, subscribe("clock", Query::Time)).accepted);
    let (tx, _) = mpsc::sync_channel(1);
    assert!(
        service
            .enqueue(request(subscribe("clock", Query::Time)), tx)
            .is_err()
    );
}
#[test]
fn retirement_seals_before_clearing_subscriptions_and_can_retry() {
    let (mut service, state) = setup();
    send(&mut service, subscribe("clock", Query::Time));
    state.borrow_mut().fail_retire = true;
    assert!(service.retire(&identity()).is_err());
    state.borrow_mut().time = 1.;
    assert_eq!(service.publications().unwrap().len(), 1);
    state.borrow_mut().fail_retire = false;
    service.retire(&identity()).unwrap();
    service.retire(&identity()).unwrap();
    assert_eq!(state.borrow().retired, 1);
    assert!(service.publications().is_err());
    let (tx, _) = mpsc::sync_channel(1);
    assert!(service.enqueue(request(query(Query::Project)), tx).is_err());
}
#[test]
fn failed_publication_reads_do_not_consume_cursor_or_fabricate_values() {
    let (mut service, state) = setup();
    send(&mut service, subscribe("clock", Query::Time));
    state.borrow_mut().time = 1.;
    state.borrow_mut().fail_query = true;
    assert!(service.publications().is_err());
    state.borrow_mut().fail_query = false;
    assert_eq!(service.publications().unwrap()[0].sequence, 1);
    state.borrow_mut().revision = 2;
    assert!(service.publications().is_err());
    state.borrow_mut().revision = 3;
    state.borrow_mut().time = 2.;
    assert_eq!(service.publications().unwrap()[0].sequence, 2);
}
#[test]
fn retired_subscription_ids_cannot_receive_old_in_flight_publications() {
    let (mut service, _) = setup();
    send(&mut service, subscribe("clock", Query::Time));
    send(
        &mut service,
        HostOperation::Unsubscribe {
            subscription: "clock".into(),
        },
    );
    let (tx, _) = mpsc::sync_channel(1);
    assert!(
        service
            .enqueue(request(subscribe("clock", Query::Time)), tx)
            .is_err()
    );
}
#[test]
fn malformed_atomic_edit_never_enters_actor_queue() {
    let (mut service, state) = setup();
    for operation in [
        json!({"type":"edit","label":"Edit","commands":[]}),
        json!({"type":"edit","label":"Edit","commands":[{"command":"","args":false}]}),
    ] {
        let (tx, _) = mpsc::sync_channel(1);
        assert!(
            service
                .enqueue(request(serde_json::from_value(operation).unwrap()), tx)
                .is_err()
        );
    }
    assert!(state.borrow().pending.is_empty());
}
