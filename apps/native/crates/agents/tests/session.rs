mod support;
use pm_agents::session::{Gate, Owner};
use serde_json::json;
fn owner(n: u64) -> Owner {
    Owner {
        document: "document".into(),
        generation: n,
    }
}
#[test]
fn disconnect_or_document_loss_while_authorizing_never_activates_a_late_session() {
    for disconnect in [true, false] {
        let mut gate = Gate::new("connection".into(), Some(owner(1))).unwrap();
        let opening = gate.begin_open().unwrap();
        assert!(gate.begin_open().is_err());
        if disconnect {
            gate.disconnect().unwrap();
        } else {
            gate.owner_lost().unwrap();
        }
        assert!(gate.authorized(&opening, "late".into()).is_err());
        if disconnect {
            assert!(gate.recovered(owner(2)).is_err());
        } else {
            gate.recovered(owner(2)).unwrap();
            let next = gate.begin_open().unwrap();
            assert!(gate.authorized(&opening, "late".into()).is_err());
            assert!(gate.authorized(&next, "new".into()).is_ok());
        }
    }
}
#[test]
fn owner_loss_ends_the_transaction_but_connection_recovers_with_a_fresh_lease() {
    let mut gate = Gate::new("connection".into(), Some(owner(1))).unwrap();
    let opening = gate.begin_open().unwrap();
    let old = gate.authorized(&opening, "run".into()).unwrap();
    let finish = gate.owner_lost().unwrap().unwrap();
    assert!(!finish.commit);
    assert!(gate.check(&old).is_err());
    assert!(gate.recovered(owner(2)).is_err());
    gate.finished(&finish, Ok(json!({"rolledBack":true})), None)
        .unwrap();
    gate.recovered(owner(2)).unwrap();
    let opening = gate.begin_open().unwrap();
    let new = gate.authorized(&opening, "new-run".into()).unwrap();
    assert!(gate.check(&new).is_ok());
    assert!(gate.check(&old).is_err());
}
#[test]
fn finishing_is_single_flight_and_rejects_calls_without_replacing_original_errors() {
    let mut gate = Gate::new("connection".into(), Some(owner(1))).unwrap();
    let opening = gate.begin_open().unwrap();
    let lease = gate.authorized(&opening, "run".into()).unwrap();
    let first = gate.begin_finish(true).unwrap();
    assert_eq!(gate.begin_finish(false).unwrap(), first);
    assert!(gate.check(&lease).is_err());
    let outcome = gate
        .finished(
            &first,
            Err("original compile failure".into()),
            Some("cleanup failure".into()),
        )
        .unwrap();
    assert_eq!(outcome.result, Err("original compile failure".into()));
    assert_eq!(outcome.cleanup_error.as_deref(), Some("cleanup failure"));
    assert!(gate.finished(&first, Ok(json!(null)), None).is_err());
}
#[test]
fn late_stop_preserves_publication_receipt_and_forbids_replacement_stage() {
    let mut gate = Gate::new("connection".into(), Some(owner(1))).unwrap();
    let opening = gate.begin_open().unwrap();
    let lease = gate.authorized(&opening, "run".into()).unwrap();
    let published = gate
        .commit_publication(&lease, || {
            Ok(json!({"changeSetId":"committed","extensions":["extension"]}))
        })
        .unwrap();
    let finish = gate.disconnect().unwrap().unwrap();
    assert!(!gate.may_prepare_next_stage(&published));
    assert_eq!(published.receipt["changeSetId"], "committed");
    let mut invoked = false;
    assert!(
        gate.commit_publication(&lease, || {
            invoked = true;
            Ok(json!(null))
        })
        .is_err()
    );
    assert!(!invoked);
    gate.finished(&finish, Ok(published.receipt.clone()), None)
        .unwrap();
}
#[test]
fn panel_capture_and_input_require_the_same_document_and_registration_generation() {
    let mut gate = Gate::new("connection".into(), Some(owner(1))).unwrap();
    let opening = gate.begin_open().unwrap();
    let lease = gate.authorized(&opening, "run".into()).unwrap();
    let capture = gate.capture_panel(&lease, "panel", 4).unwrap();
    assert!(gate.check_panel(&capture, "panel", 4).is_ok());
    assert!(gate.check_panel(&capture, "panel", 5).is_err());
    assert!(gate.check_panel(&capture, "replacement", 4).is_err());
    gate.owner_lost().unwrap();
    assert!(gate.check_panel(&capture, "panel", 4).is_err());
}
#[test]
fn one_connection_cannot_reuse_another_connections_lease_or_finish() {
    let mut a = Gate::new("a".into(), Some(owner(1))).unwrap();
    let opening = a.begin_open().unwrap();
    let lease = a.authorized(&opening, "run".into()).unwrap();
    let finish = a.begin_finish(false).unwrap();
    let mut b = Gate::new("b".into(), Some(owner(1))).unwrap();
    assert!(b.check(&lease).is_err());
    assert!(b.authorized(&opening, "run".into()).is_err());
    assert!(b.finished(&finish, Ok(json!({})), None).is_err());
}

#[test]
fn review_keeps_original_provider_settings_applied_edits_and_capture_failure() {
    use pm_agents::review::Review;
    let mut picker = json!({"provider":"claude","model":"authored-alias","reasoningEffort":"high","approval":"supervised","future":"keep"});
    let mut review =
        Review::new(owner(1), &picker, vec![json!({"type":"actual-executed"})]).unwrap();
    picker["provider"] = json!("chatgpt");
    assert_eq!(review.settings()["provider"], "claude");
    assert_eq!(review.settings()["model"], "authored-alias");
    assert_eq!(review.settings()["access"], "editor");
    review
        .observe(&owner(1), Err("capture failed".into()))
        .unwrap();
    review
        .response(json!({"status":"pass","commands":[]}))
        .unwrap();
    assert!(review.outcome().get("visualVerified").is_none());
    assert_eq!(review.applied(), &[json!({"type":"actual-executed"})]);
    review.failed("cleanup failed".into());
    assert_eq!(review.outcome()["reviewError"], "capture failed");
    assert!(
        review
            .observe(&owner(2), Ok(json!({"images":["stale"]})))
            .is_err()
    );
}
#[test]
fn review_repairs_are_bounded_and_require_fresh_pixels_before_visual_acceptance() {
    use pm_agents::review::Review;
    let mut review = Review::new(
        owner(1),
        &json!({"provider":"compatible","model":"saved"}),
        vec![],
    )
    .unwrap();
    for _ in 0..2 {
        assert!(
            review
                .response(json!({"status":"repair","commands":[{}]}))
                .unwrap()
        );
        review
            .repaired(&owner(1), vec![json!({"type":"executed-repair"})])
            .unwrap();
        assert!(review.outcome().get("visualVerified").is_none());
    }
    assert!(
        !review
            .response(json!({"status":"repair","commands":[{}]}))
            .unwrap()
    );
    assert!(review.repaired(&owner(1), vec![]).is_err());
    review
        .observe(&owner(1), Ok(json!({"images":["real-host-frame"]})))
        .unwrap();
    review
        .response(json!({"status":"pass","commands":[]}))
        .unwrap();
    assert_eq!(review.outcome()["review"]["status"], "pass");
    assert!(review.outcome().get("visualVerified").is_none());
}
#[test]
fn already_open_media_grant_stops_serving_when_its_session_is_lost() {
    let temp = support::Temp::new();
    std::fs::write(temp.0.join("frame.png"), b"synthetic inert bytes").unwrap();
    let mut gate = Gate::new("media".into(), Some(owner(1))).unwrap();
    let opening = gate.begin_open().unwrap();
    let lease = gate.authorized(&opening, "run".into()).unwrap();
    let mut grant = pm_agents::media::BoundGrant::open(
        &gate,
        &lease,
        &temp.0,
        std::path::Path::new("frame.png"),
    )
    .unwrap();
    assert!(grant.read(&gate, 0, 4).is_ok());
    gate.disconnect().unwrap();
    assert!(grant.read(&gate, 0, 4).is_err());
    assert!(grant.response(&gate, "GET", None).is_err());
}

#[test]
fn review_rejects_invalid_settings_and_consumes_each_repair_once() {
    use pm_agents::review::Review;
    assert!(Review::new(owner(1), &json!(null), vec![]).is_err());
    let mut review = Review::new(owner(1), &json!({}), vec![]).unwrap();
    assert!(
        review
            .response(json!({"status":"repair","commands":[{}]}))
            .unwrap()
    );
    review
        .repaired(&owner(1), vec![json!({"executed":true})])
        .unwrap();
    assert!(
        review
            .repaired(&owner(1), vec![json!({"executed":true})])
            .is_err()
    );
    assert_eq!(review.applied().len(), 1);
}
