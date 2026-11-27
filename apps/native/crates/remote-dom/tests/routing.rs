use pm_remote_dom::{protocol::*, *};
use std::collections::BTreeMap;

#[derive(Default)]
struct Queue<T> {
    values: Vec<T>,
    full: bool,
}
impl<T> TrySink<T> for Queue<T> {
    fn try_send(&mut self, value: T) -> Result<(), Error> {
        if self.full {
            return Err(Error::Backpressure);
        }
        self.values.push(value);
        Ok(())
    }
}
fn session() -> Session {
    Session {
        panel: "inspector".into(),
        generation: 1,
    }
}
fn router() -> Router {
    Router::new(session(), Limits::default()).unwrap()
}
fn batch(seq: u64, ops: Vec<Op>) -> Batch {
    Batch {
        version: VERSION,
        session: session(),
        seq,
        caused_by: None,
        ops,
    }
}
fn create(id: u64) -> Op {
    Op::Create {
        id,
        kind: NodeKind::Element,
        name: "input".into(),
        namespace: None,
    }
}
fn insert(parent: u64, id: u64) -> Op {
    Op::Insert {
        parent,
        id,
        before: None,
    }
}
fn mounted(router: &mut Router) {
    router
        .route_batch(
            batch(
                1,
                vec![
                    create(6),
                    insert(BODY, 6),
                    Op::Listen {
                        id: 6,
                        event: "input".into(),
                        enabled: true,
                        policy: EventPolicy::Observe,
                    },
                ],
            ),
            &mut Queue {
                values: vec![],
                full: false,
            },
        )
        .unwrap();
}
fn event(seq: u64) -> PageMessage {
    PageMessage::Event {
        session: session(),
        seq,
        applied: 1,
        target: 6,
        event: "input".into(),
        bubbles: true,
        cancelable: false,
        default_prevented: false,
        data: BTreeMap::new(),
    }
}
fn page_queue() -> Queue<PageMessage> {
    Queue {
        values: vec![],
        full: false,
    }
}
fn batch_queue() -> Queue<Batch> {
    Queue {
        values: vec![],
        full: false,
    }
}

#[test]
fn mounts_routes_and_inspects_without_entering_js() {
    let mut r = router();
    mounted(&mut r);
    let mut q = page_queue();
    r.route_page(event(1), &mut q).unwrap();
    assert_eq!(q.values.len(), 1);
    assert_eq!(r.inspect().nodes, 6);
    assert_eq!(r.inspect().last_event, 1);
    let mut b = batch(
        2,
        vec![Op::Property {
            id: 6,
            name: "value".into(),
            value: "typed".into(),
        }],
    );
    b.caused_by = Some(1);
    r.route_batch(b, &mut batch_queue()).unwrap();
    assert_eq!(r.inspect().sent, 2);
}
#[test]
fn serde_roundtrips_every_operation_and_feedback_shape() {
    let ops = vec![
        Op::Range {
            anchor: Some(6),
            anchor_offset: 0,
            focus: Some(6),
            focus_offset: 1,
        },
        create(6),
        insert(BODY, 6),
        Op::Remove {
            parent: BODY,
            id: 6,
        },
        Op::Attribute {
            id: 6,
            name: "aria-label".into(),
            namespace: None,
            value: Some("Name".into()),
        },
        Op::Text {
            id: 7,
            value: "🦀".into(),
        },
        Op::Property {
            id: 6,
            name: "checked".into(),
            value: true.into(),
        },
        Op::Listen {
            id: 6,
            event: "click".into(),
            enabled: true,
            policy: EventPolicy::Block,
        },
        Op::Focus {
            id: 6,
            prevent_scroll: true,
        },
        Op::Blur { id: 6 },
        Op::Selection {
            id: 6,
            start: 0,
            end: 2,
            direction: "none".into(),
        },
        Op::Scroll {
            id: 6,
            x: 10.0,
            y: 20.0,
            behavior: "instant".into(),
        },
        Op::Measure {
            request: 1,
            ids: vec![6],
            styles: vec!["color".into()],
        },
        Op::ObserveResize {
            id: 6,
            enabled: true,
        },
    ];
    let b = batch(1, ops);
    assert_eq!(
        serde_json::from_slice::<Batch>(&serde_json::to_vec(&b).unwrap()).unwrap(),
        b
    );
    for m in [
        event(1),
        PageMessage::Applied {
            session: session(),
            seq: 1,
        },
        PageMessage::Layout {
            session: session(),
            applied: 1,
            request: None,
            entries: vec![],
        },
        PageMessage::Fault {
            session: session(),
            seq: 1,
            message: "fault".into(),
        },
    ] {
        assert_eq!(
            router()
                .decode_page(&serde_json::to_vec(&m).unwrap())
                .unwrap(),
            m
        );
    }
}
#[test]
fn generation_and_panel_are_fenced() {
    let mut r = router();
    let mut b = batch(1, vec![]);
    b.session.generation = 2;
    assert_eq!(
        r.route_batch(b, &mut batch_queue()),
        Err(Error::StaleSession)
    );
    mounted(&mut r);
    let mut e = event(1);
    if let PageMessage::Event { session, .. } = &mut e {
        session.panel = "other".into();
    }
    assert_eq!(r.route_page(e, &mut page_queue()), Err(Error::StaleSession));
}
#[test]
fn sequence_and_causing_event_are_validated() {
    let mut r = router();
    assert_eq!(
        r.route_batch(batch(2, vec![]), &mut batch_queue()),
        Err(Error::Sequence)
    );
    let mut b = batch(1, vec![]);
    b.caused_by = Some(1);
    assert_eq!(r.route_batch(b, &mut batch_queue()), Err(Error::Sequence));
    mounted(&mut r);
    r.route_page(event(1), &mut page_queue()).unwrap();
    assert_eq!(
        r.route_page(event(1), &mut page_queue()),
        Err(Error::Sequence)
    );
}
#[test]
fn invalid_batch_is_atomic_and_retryable() {
    let mut r = router();
    let before = r.inspect();
    let mut q = batch_queue();
    assert!(
        r.route_batch(batch(1, vec![create(6), insert(99, 6)]), &mut q)
            .is_err()
    );
    assert_eq!(r.inspect(), before);
    assert!(q.values.is_empty());
    mounted(&mut r);
}
#[test]
fn transport_backpressure_does_not_advance_state_or_drop_batch() {
    let mut r = router();
    let before = r.inspect();
    let mut q = Queue {
        values: vec![],
        full: true,
    };
    let b = batch(1, vec![create(6), insert(BODY, 6)]);
    assert_eq!(r.route_batch(b.clone(), &mut q), Err(Error::Backpressure));
    assert_eq!(r.inspect(), before);
    q.full = false;
    r.route_batch(b.clone(), &mut q).unwrap();
    assert_eq!(q.values, vec![b]);
}
#[test]
fn event_backpressure_allows_same_event_retry() {
    let mut r = router();
    mounted(&mut r);
    let mut q = Queue {
        values: vec![],
        full: true,
    };
    assert_eq!(r.route_page(event(1), &mut q), Err(Error::Backpressure));
    assert_eq!(r.inspect().last_event, 0);
    q.full = false;
    r.route_page(event(1), &mut q).unwrap();
    assert_eq!(r.inspect().last_event, 1);
}
#[test]
fn pending_window_is_bounded_by_applied_acknowledgements() {
    let mut r = Router::new(
        session(),
        Limits {
            in_flight: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    mounted(&mut r);
    assert_eq!(
        r.route_batch(batch(2, vec![]), &mut batch_queue()),
        Err(Error::Backpressure)
    );
    r.route_page(
        PageMessage::Applied {
            session: session(),
            seq: 1,
        },
        &mut page_queue(),
    )
    .unwrap();
    r.route_batch(batch(2, vec![]), &mut batch_queue()).unwrap();
}
#[test]
fn future_and_regressing_acks_are_refused_duplicate_is_harmless() {
    let mut r = router();
    mounted(&mut r);
    assert_eq!(
        r.route_page(
            PageMessage::Applied {
                session: session(),
                seq: 2
            },
            &mut page_queue()
        ),
        Err(Error::Sequence)
    );
    for _ in 0..2 {
        r.route_page(
            PageMessage::Applied {
                session: session(),
                seq: 1,
            },
            &mut page_queue(),
        )
        .unwrap();
    }
    assert_eq!(
        r.route_page(
            PageMessage::Applied {
                session: session(),
                seq: 0
            },
            &mut page_queue()
        ),
        Err(Error::Sequence)
    );
}
#[test]
fn cycles_invalid_anchors_and_reserved_roots_are_refused() {
    for ops in [
        vec![create(6), insert(6, 6)],
        vec![create(6), create(7), insert(6, 7), insert(7, 6)],
        vec![
            create(6),
            Op::Insert {
                parent: BODY,
                id: 6,
                before: Some(HEAD),
            },
        ],
        vec![Op::Remove {
            parent: HTML,
            id: BODY,
        }],
        vec![insert(BODY, HTML)],
    ] {
        assert!(
            router()
                .route_batch(batch(1, ops), &mut batch_queue())
                .is_err()
        );
    }
}
#[test]
fn duplicate_and_inexact_ids_are_refused() {
    for ops in [
        vec![create(6), create(6)],
        vec![create(MAX_ID + 1)],
        vec![create(1)],
    ] {
        assert!(
            router()
                .route_batch(batch(1, ops), &mut batch_queue())
                .is_err()
        );
    }
}
#[test]
fn detached_identity_survives_but_events_require_attached_subscription() {
    let mut r = router();
    mounted(&mut r);
    r.route_batch(
        batch(
            2,
            vec![Op::Remove {
                parent: BODY,
                id: 6,
            }],
        ),
        &mut batch_queue(),
    )
    .unwrap();
    let mut detached = event(1);
    if let PageMessage::Event { applied, .. } = &mut detached {
        *applied = 2;
    }
    assert!(r.route_page(detached, &mut page_queue()).is_err());
    r.route_batch(batch(3, vec![insert(BODY, 6)]), &mut batch_queue())
        .unwrap();
    assert_eq!(r.inspect().nodes, 6);
    r.route_page(event(1), &mut page_queue()).unwrap();
}
#[test]
fn ancestor_delegated_subscription_works() {
    let mut r = router();
    r.route_batch(
        batch(
            1,
            vec![
                create(6),
                insert(BODY, 6),
                Op::Listen {
                    id: DOCUMENT,
                    event: "input".into(),
                    enabled: true,
                    policy: EventPolicy::Observe,
                },
            ],
        ),
        &mut batch_queue(),
    )
    .unwrap();
    r.route_page(event(1), &mut page_queue()).unwrap();
}
#[test]
fn arbitrary_properties_and_wrong_target_types_are_refused() {
    for op in [
        Op::Property {
            id: 6,
            name: "innerHTML".into(),
            value: "unsafe".into(),
        },
        Op::Property {
            id: 6,
            name: "checked".into(),
            value: "true".into(),
        },
        Op::Text {
            id: 6,
            value: "lost children".into(),
        },
        Op::Focus {
            id: WINDOW,
            prevent_scroll: false,
        },
    ] {
        assert!(
            router()
                .route_batch(batch(1, vec![create(6), op]), &mut batch_queue())
                .is_err()
        );
    }
}
#[test]
fn malformed_json_unknown_fields_version_and_size_limits_are_refused() {
    let r = router();
    assert!(r.decode_batch(b"{").is_err());
    let mut json = serde_json::to_value(batch(1, vec![])).unwrap();
    json["eval"] = true.into();
    assert!(r.decode_batch(&serde_json::to_vec(&json).unwrap()).is_err());
    let mut b = batch(1, vec![]);
    b.version = 2;
    assert!(router().route_batch(b, &mut batch_queue()).is_err());
    let r = Router::new(
        session(),
        Limits {
            bytes: 10,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(r.decode_batch(&[b' '; 11]), Err(Error::TooLarge));
    let mut r = Router::new(
        session(),
        Limits {
            operations: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.route_batch(batch(1, vec![create(6), create(7)]), &mut batch_queue()),
        Err(Error::TooLarge)
    );
}
#[test]
fn layout_requests_are_single_use_and_invalid_layout_is_retryable() {
    let mut r = router();
    mounted(&mut r);
    r.route_batch(
        batch(
            2,
            vec![Op::Measure {
                request: 1,
                ids: vec![6],
                styles: vec![],
            }],
        ),
        &mut batch_queue(),
    )
    .unwrap();
    let good = PageMessage::Layout {
        session: session(),
        applied: 2,
        request: Some(1),
        entries: vec![LayoutEntry {
            id: 6,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 20.0,
            },
            scroll_left: 0.0,
            scroll_top: 0.0,
            scroll_width: 100.0,
            scroll_height: 20.0,
            client_width: 100.0,
            client_height: 20.0,
            styles: BTreeMap::new(),
        }],
    };
    let mut bad = good.clone();
    if let PageMessage::Layout { entries, .. } = &mut bad {
        entries[0].rect.width = -1.0;
    }
    assert!(r.route_page(bad, &mut page_queue()).is_err());
    r.route_page(good.clone(), &mut page_queue()).unwrap();
    assert!(r.route_page(good, &mut page_queue()).is_err());
}
#[test]
fn page_fault_fences_generation_but_preserves_inspection() {
    let mut r = router();
    mounted(&mut r);
    assert_eq!(
        r.route_page(
            PageMessage::Fault {
                session: session(),
                seq: 1,
                message: "page failed".into()
            },
            &mut page_queue()
        ),
        Err(Error::PageFault("page failed".into()))
    );
    assert!(r.inspect().faulted);
    assert_eq!(r.inspect().nodes, 6);
    assert!(r.route_batch(batch(2, vec![]), &mut batch_queue()).is_err());
}

#[test]
fn queued_tree_changes_do_not_reject_input_from_the_still_visible_revision() {
    let mut r = router();
    mounted(&mut r);
    r.route_batch(
        batch(
            2,
            vec![Op::Remove {
                parent: BODY,
                id: 6,
            }],
        ),
        &mut batch_queue(),
    )
    .unwrap();
    r.route_page(event(1), &mut page_queue()).unwrap();
    r.route_page(
        PageMessage::Applied {
            session: session(),
            seq: 2,
        },
        &mut page_queue(),
    )
    .unwrap();
    assert_eq!(
        r.route_page(event(2), &mut page_queue()),
        Err(Error::Sequence)
    );
}
#[test]
fn executable_page_content_is_refused() {
    for op in [
        Op::Create {
            id: 6,
            kind: NodeKind::Element,
            name: "svg:script".into(),
            namespace: Some("http://www.w3.org/2000/svg".into()),
        },
        Op::Create {
            id: 6,
            kind: NodeKind::Element,
            name: "script".into(),
            namespace: None,
        },
        Op::Attribute {
            id: BODY,
            name: "onclick".into(),
            namespace: None,
            value: Some("code".into()),
        },
        Op::Attribute {
            id: BODY,
            name: "href".into(),
            namespace: None,
            value: Some("java\nscript:code".into()),
        },
    ] {
        assert!(
            router()
                .route_batch(batch(1, vec![op]), &mut batch_queue())
                .is_err()
        );
    }
}

#[test]
fn range_validation_preserves_exact_endpoints_and_refuses_incomplete_pairs() {
    let mut r = router();
    mounted(&mut r);
    assert!(
        r.route_batch(
            batch(
                2,
                vec![Op::Range {
                    anchor: Some(6),
                    anchor_offset: 0,
                    focus: None,
                    focus_offset: 0
                }]
            ),
            &mut batch_queue()
        )
        .is_err()
    );
    r.route_batch(
        batch(
            2,
            vec![Op::Range {
                anchor: Some(6),
                anchor_offset: 0,
                focus: Some(6),
                focus_offset: 1,
            }],
        ),
        &mut batch_queue(),
    )
    .unwrap();
    r.route_batch(
        batch(
            3,
            vec![Op::Range {
                anchor: None,
                anchor_offset: 0,
                focus: None,
                focus_offset: 0,
            }],
        ),
        &mut batch_queue(),
    )
    .unwrap();
}

#[test]
fn bounded_host_queue_is_nonblocking_and_preserves_order_on_retry() {
    let (mut sink, receiver) = bounded_channel(1).unwrap();
    let mut r = router();
    r.route_batch(batch(1, vec![create(6)]), &mut sink).unwrap();
    assert_eq!(
        r.route_batch(batch(2, vec![insert(BODY, 6)]), &mut sink),
        Err(Error::Backpressure)
    );
    assert_eq!(receiver.try_recv().unwrap().seq, 1);
    r.route_batch(batch(2, vec![insert(BODY, 6)]), &mut sink)
        .unwrap();
    assert_eq!(receiver.try_recv().unwrap().seq, 2);
    assert!(bounded_channel::<Batch>(0).is_err());
}
#[test]
fn closed_host_queue_does_not_publish_or_advance_router_state() {
    let (mut sink, receiver) = bounded_channel(1).unwrap();
    drop(receiver);
    let mut r = router();
    assert_eq!(
        r.route_batch(batch(1, vec![create(6)]), &mut sink),
        Err(Error::TransportClosed)
    );
    assert_eq!(r.inspect().sent, 0);
}
