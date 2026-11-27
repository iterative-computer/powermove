use pm_agents::{
    Effects, Event, RunResult,
    delivery::{Delivery, Queue, Surface},
};
use serde_json::json;
#[test]
fn queue_retains_every_observation_and_orders_batches_without_partial_admission() {
    let mut q = Queue::new(6);
    let effects = Effects {
        outgoing: vec![json!({"id":1})],
        events: vec![
            Event::Ready,
            Event::Session {
                run: "r".into(),
                id: "s".into(),
            },
            Event::Completed {
                run: "r".into(),
                result: RunResult::failure("cancelled", true),
            },
        ],
        close_stdin: true,
    };
    q.push(Surface::Codex, effects).unwrap();
    let rejected = q
        .push(
            Surface::Codex,
            Effects {
                outgoing: vec![json!({"id":2})],
                events: vec![Event::Stopped("exit".into())],
                close_stdin: false,
            },
        )
        .unwrap_err();
    let expected = [
        Delivery::Wire(json!({"id":1})),
        Delivery::Inspection(Event::Ready),
        Delivery::Inspection(Event::Session {
            run: "r".into(),
            id: "s".into(),
        }),
        Delivery::Callback(Event::Completed {
            run: "r".into(),
            result: RunResult::failure("cancelled", true),
        }),
        Delivery::CloseStdin,
    ];
    for (i, expected) in expected.into_iter().enumerate() {
        let e = q.pop().unwrap();
        assert_eq!(e.sequence, i as u64);
        assert_eq!(e.delivery, expected);
    }
    assert!(q.pop().is_none());
    q.push(Surface::Codex, rejected).unwrap();
    assert_eq!(q.pop().unwrap().sequence, 5);
    assert_eq!(q.pop().unwrap().sequence, 6);
}
#[test]
fn session_callback_contract_is_provider_specific_and_inspection_is_never_dropped() {
    for surface in [Surface::Codex, Surface::Claude, Surface::CodexExec] {
        let mut q = Queue::new(4);
        let session = Event::Session {
            run: "r".into(),
            id: "s".into(),
        };
        q.push(
            surface,
            Effects {
                events: vec![session.clone(), Event::Notification(json!({"future":true}))],
                ..Effects::default()
            },
        )
        .unwrap();
        let actual = q.pop().unwrap().delivery;
        assert_eq!(
            actual,
            if surface == Surface::Claude {
                Delivery::Callback(session)
            } else {
                Delivery::Inspection(session)
            }
        );
        assert_eq!(
            q.pop().unwrap().delivery,
            Delivery::Inspection(Event::Notification(json!({"future":true})))
        );
    }
}
#[test]
fn zero_capacity_returns_even_terminal_events_intact_for_retry() {
    let mut q = Queue::new(0);
    let e = Effects {
        events: vec![Event::Stopped("exit".into())],
        ..Effects::default()
    };
    let returned = q.push(Surface::Claude, e).unwrap_err();
    assert_eq!(returned.events, vec![Event::Stopped("exit".into())]);
    assert!(q.pop().is_none());
}
