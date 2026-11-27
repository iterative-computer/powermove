use pm_kernel::commands::pro_transport::*;
use pm_model::json::parse;
fn active(rate: ShuttleRate) -> ShuttleState {
    ShuttleState {
        available: true,
        time: 4.,
        rate,
        ..Default::default()
    }
}
fn dispatch(s: ShuttleState, k: ShuttleInput) -> ShuttleState {
    match s.dispatch(k).unwrap() {
        ShuttleOutcome::Handled(s) => s,
        _ => panic!("declined"),
    }
}
#[test]
fn jkl_sequence_caps_at_two_and_changes_direction_at_one() {
    let mut s = active(ShuttleRate::Forward1);
    for (key, rate) in [
        (ShuttleInput::L, 2),
        (ShuttleInput::L, 2),
        (ShuttleInput::J, -1),
        (ShuttleInput::J, -2),
        (ShuttleInput::J, -2),
        (ShuttleInput::L, 1),
    ] {
        s = dispatch(s, key);
        assert_eq!(s.rate.factor(), rate);
    }
    s = dispatch(s, ShuttleInput::K);
    assert_eq!(s.rate, ShuttleRate::Stopped);
}
#[test]
fn paused_or_unavailable_declines_once_without_changing_facts() {
    for s in [ShuttleState::default(), active(ShuttleRate::Stopped)] {
        for key in [ShuttleInput::J, ShuttleInput::K, ShuttleInput::L] {
            assert_eq!(s.dispatch(key).unwrap(), ShuttleOutcome::Declined);
        }
    }
    assert_eq!(
        ShuttleState::default()
            .dispatch(ShuttleInput::Reverse)
            .unwrap(),
        ShuttleOutcome::Declined
    );
}
#[test]
fn reverse_starts_paused_and_observed_clock_stops_at_beginning() {
    let s = dispatch(active(ShuttleRate::Stopped), ShuttleInput::Reverse);
    assert_eq!(s.rate, ShuttleRate::Reverse1);
    let s = s.observe_clock(-0.01, 10.).unwrap();
    assert_eq!(s.rate, ShuttleRate::Stopped);
    assert_eq!(s.time, 0.);
    let s = active(ShuttleRate::Forward2)
        .observe_clock(10.1, 10.)
        .unwrap();
    assert_eq!(s.time, 10.);
    assert_eq!(s.rate, ShuttleRate::Stopped);
}
#[test]
fn stop_snaps_to_frame_and_clears_around_return() {
    let mut s = active(ShuttleRate::Forward2);
    s.time = 4.019;
    let s = dispatch(s, ShuttleInput::K);
    assert_eq!(s.time, 121. / 30.);
    assert_eq!(s.return_time, None);
}
#[test]
fn play_around_clamps_and_returns_to_original_frame() {
    let mut s = active(ShuttleRate::Stopped);
    s.time = 1.;
    let s = dispatch(
        s,
        ShuttleInput::PlayAround {
            before: 2.,
            after: 2.,
            duration: 10.,
        },
    );
    assert_eq!(s.time, 0.);
    assert_eq!(s.range, Some(PlayRange { start: 0., end: 3. }));
    let s = s.observe_clock(3.1, 10.).unwrap();
    assert_eq!(s.time, 1.);
    assert_eq!(s.rate, ShuttleRate::Stopped);
    assert_eq!(s.range, None);
}
#[test]
fn transport_facts_roundtrip_and_preserve_opaque_session_members() {
    let s = dispatch(
        active(ShuttleRate::Stopped),
        ShuttleInput::PlayAround {
            before: 2.,
            after: 2.,
            duration: 10.,
        },
    );
    let existing = parse(r#"{"extensionClock":{"keep":true},"time":0,"playing":false}"#).unwrap();
    let v = s.transport_value(&existing).unwrap();
    assert_eq!(v.get("extensionClock"), existing.get("extensionClock"));
    assert_eq!(ShuttleState::from_transport(&v, true, 30.).unwrap(), s);
    let legacy = parse(r#"{"time":2,"playing":true}"#).unwrap();
    assert_eq!(
        ShuttleState::from_transport(&legacy, false, 24.)
            .unwrap()
            .rate,
        ShuttleRate::Forward1
    );
}
#[test]
fn invalid_ranges_rates_and_nonfinite_times_refuse() {
    for (before, after, duration) in [
        (-1., 2., 10.),
        (0., 0., 10.),
        (1., 1., 0.),
        (f64::NAN, 1., 10.),
    ] {
        assert!(
            active(ShuttleRate::Stopped)
                .dispatch(ShuttleInput::PlayAround {
                    before,
                    after,
                    duration
                })
                .is_err()
        );
    }
    assert!(
        active(ShuttleRate::Forward1)
            .observe_clock(f64::NAN, 10.)
            .is_err()
    );
    assert!(
        ShuttleState::from_transport(
            &parse(r#"{"time":0,"playing":true,"rate":3}"#).unwrap(),
            true,
            30.
        )
        .is_err()
    );
}

#[test]
fn malformed_public_state_never_panics_on_a_clock_update() {
    for range in [
        PlayRange { start: 3., end: 2. },
        PlayRange {
            start: f64::NAN,
            end: 2.,
        },
        PlayRange {
            start: 0.,
            end: 20.,
        },
    ] {
        let s = ShuttleState {
            range: Some(range),
            ..active(ShuttleRate::Forward1)
        };
        assert!(s.observe_clock(1., 10.).is_err());
    }
}
#[test]
fn shuttle_facts_publish_through_the_actor_without_document_undo() {
    use pm_kernel::{
        KernelActor, OpenSource,
        state::{StateOp, StateTransaction, UiState},
    };
    let actor = KernelActor::spawn(Default::default());
    let h = actor.handle();
    h.open("P",OpenSource::Untitled(parse(r##"{"id":"P","name":"Transport","compId":"C1","w":100,"h":100,"dur":10,"fps":30,"layers":[],"assets":{}}"##).unwrap())).wait().unwrap().unwrap();
    let before = h.published("P").unwrap();
    let state = dispatch(active(ShuttleRate::Stopped), ShuttleInput::Reverse);
    let ui = UiState {
        transport: state.transport_value(&before.ui_state.transport).unwrap(),
        ..before.ui_state.clone()
    };
    h.state_transaction(StateTransaction {
        ops: vec![StateOp::Ui {
            project: "P".into(),
            next: ui,
        }],
        ..Default::default()
    })
    .wait()
    .unwrap()
    .unwrap();
    let after = h.published("P").unwrap();
    assert_eq!(
        after.snapshot.document.root().map(pm_model::Node::to_value),
        before
            .snapshot
            .document
            .root()
            .map(pm_model::Node::to_value)
    );
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.dirty, before.dirty);
    assert_eq!(after.history, before.history);
    assert_eq!(
        ShuttleState::from_transport(&after.ui_state.transport, true, 30.).unwrap(),
        state
    );
}
