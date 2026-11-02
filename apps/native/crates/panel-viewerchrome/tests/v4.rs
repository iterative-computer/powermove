use pm_kernel::{Caller, KernelActor, KernelConfig, OpenSource, Reply};
use pm_model::{Value, json::parse};
use pm_panel_viewerchrome::{v4::Session, *};

const FIXTURE: &str = r##"{"id":"p","name":"Launch Film","compId":"c","compName":"Launch Film","w":1920,"h":1080,"fps":24,"dur":10,"work":[1,9],"bg":"#000000","layers":[],"assets":{},"comps":{}}"##;
fn fixture() -> (KernelActor, Controller, Viewport) {
    let actor = KernelActor::spawn(KernelConfig::default());
    actor
        .handle()
        .open("p", OpenSource::Untitled(parse(FIXTURE).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let controller = Controller::new(actor.handle(), "p");
    let viewport = controller
        .read()
        .unwrap()
        .viewport(
            Size {
                width: 1052.8,
                height: 599.2,
            },
            8.0,
        )
        .unwrap();
    (actor, controller, viewport)
}
fn request(ticket: &str, element: &str, argument: Value) -> Value {
    let mut r = pm_model::Map::new();
    r.insert("ticket", Value::from(ticket));
    r.insert("element", Value::from(element));
    r.insert("argument", argument);
    Value::Object(r)
}
fn finish(controller: &Controller, plan: Plan) {
    match controller
        .submit(plan, &mut InspectHost::default())
        .unwrap()
    {
        Submission::State(p) => {
            p.wait().unwrap().unwrap();
        }
        Submission::Document(p) => {
            assert!(!matches!(p.wait().unwrap().unwrap().reply, Reply::Throw(_)))
        }
        Submission::Enqueued => panic!("test expects real actor ack"),
    }
}
#[test]
fn projection_has_display_zoom_without_materializing_rows() {
    let (_actor, controller, viewport) = fixture();
    let mut session = Session::default();
    let value = session
        .publish(&controller.read().unwrap(), viewport)
        .unwrap();
    assert_eq!(value.get("ticket").and_then(Value::as_str), Some("1"));
    assert_eq!(
        value.get("name").and_then(Value::as_str),
        Some("Launch Film")
    );
    assert!((value.get("shownZoom").and_then(Value::as_f64).unwrap() - 0.54).abs() < 1e-12);
    assert_eq!(value.get("empty"), Some(&Value::Bool(true)));
    assert_eq!(value.get("selectionCount"), Some(&Value::Number(0.0)));
    assert!(value.get("layers").is_none());
    assert!(value.get("selection").is_none());
}
#[test]
fn every_popup_choice_uses_model_and_actual_kernel_ack() {
    let (_actor, controller, viewport) = fixture();
    let before = controller.published().unwrap();
    let mut session = Session::default();
    for (id, value) in ZOOM_PRESETS
        .iter()
        .map(|z| ("viewer.zoom", Value::Number(*z)))
        .chain([
            ("viewer.fit", Value::Null),
            ("viewer.zoom.in", Value::Null),
            ("viewer.zoom.out", Value::Null),
        ])
        .chain(
            ["auto", "1", "0.5", "0.25"]
                .into_iter()
                .map(|q| ("viewer.quality", Value::from(q))),
        )
    {
        let model = controller.read().unwrap();
        let data = session.publish(&model, viewport).unwrap();
        let ticket = data.get("ticket").and_then(Value::as_str).unwrap();
        let plan = session
            .drive(
                &model,
                &request(ticket, id, value),
                viewport,
                Caller::native(),
            )
            .unwrap();
        finish(&controller, plan);
    }
    let after = controller.published().unwrap();
    assert_eq!(before.revision(), after.revision());
    assert_eq!(before.dirty, after.dirty);
    assert_eq!(before.history.entries, after.history.entries);
    assert_eq!(controller.read().unwrap().chrome.quality, Quality::Quarter);
}
#[test]
fn old_ticket_and_changed_state_cannot_mutate() {
    let (_actor, controller, viewport) = fixture();
    let mut session = Session::default();
    let model = controller.read().unwrap();
    session.publish(&model, viewport).unwrap();
    session.publish(&model, viewport).unwrap();
    let req = request("1", "viewer.zoom", Value::Number(2.0));
    assert_eq!(
        session
            .drive(&model, &req, viewport, Caller::native())
            .unwrap_err(),
        Error::Stale
    );
    let req = request("2", "viewer.zoom", Value::Number(2.0));
    finish(
        &controller,
        session
            .drive(&model, &req, viewport, Caller::native())
            .unwrap(),
    );
    assert_eq!(
        session
            .drive(
                &controller.read().unwrap(),
                &req,
                viewport,
                Caller::native()
            )
            .unwrap_err(),
        Error::Stale
    );
}
#[test]
fn invalid_projection_does_not_retire_good_ticket_and_invalid_requests_fail() {
    let (_actor, controller, viewport) = fixture();
    let model = controller.read().unwrap();
    let mut session = Session::default();
    session.publish(&model, viewport).unwrap();
    let mut invalid = viewport;
    invalid.composition.width += 1.0;
    assert!(session.publish(&model, invalid).is_err());
    assert!(
        session
            .drive(
                &model,
                &request("1", "viewer.fit", Value::Null),
                viewport,
                Caller::native()
            )
            .is_ok()
    );
    for req in [
        Value::Null,
        request("01", "viewer.fit", Value::Null),
        request("1", "unknown", Value::Null),
        request("1", "viewer.quality", Value::from("invented")),
        request("1", "viewer.zoom", Value::Number(f64::NAN)),
    ] {
        assert!(
            session
                .drive(&model, &req, viewport, Caller::native())
                .is_err()
        );
    }
}
#[test]
fn selection_change_and_reopened_document_invalidate_ticket() {
    let (actor, controller, viewport) = fixture();
    let mut session = Session::default();
    let before = controller.read().unwrap();
    session.publish(&before, viewport).unwrap();
    actor
        .handle()
        .call(
            "p",
            pm_kernel::Request::new(
                Caller::native(),
                pm_kernel::SelectionCall::Channel(Some("position.x".into())),
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let changed = controller.read().unwrap();
    assert_eq!(changed.stamp.revision, before.stamp.revision);
    assert_eq!(changed.stamp.state_revision, before.stamp.state_revision);
    assert_eq!(
        session
            .drive(
                &changed,
                &request("1", "viewer.fit", Value::Null),
                viewport,
                Caller::native()
            )
            .unwrap_err(),
        Error::Stale
    );
    assert!(actor.handle().close("p").wait().unwrap());
    actor
        .handle()
        .open("p", OpenSource::Untitled(parse(FIXTURE).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(
        session
            .drive(
                &controller.read().unwrap(),
                &request("1", "viewer.fit", Value::Null),
                viewport,
                Caller::native()
            )
            .unwrap_err(),
        Error::Stale
    );
}

#[test]
fn viewport_changes_need_a_new_publication() {
    let (_actor, controller, viewport) = fixture();
    let model = controller.read().unwrap();
    let mut session = Session::default();
    session.publish(&model, viewport).unwrap();
    let mut resized = viewport;
    resized.stage.width += 1.0;
    assert_eq!(
        session
            .drive(
                &model,
                &request("1", "viewer.zoom.in", Value::Null),
                resized,
                Caller::native()
            )
            .unwrap_err(),
        Error::Stale
    );
}
#[test]
fn unnamed_compositions_and_utf16_names_are_preserved() {
    let (actor, controller, viewport) = fixture();
    assert!(actor.handle().close("p").wait().unwrap());
    let unnamed = FIXTURE.replace(",\"compName\":\"Launch Film\"", "");
    actor
        .handle()
        .open("p", OpenSource::Untitled(parse(&unnamed).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let mut session = Session::default();
    assert_eq!(
        session
            .publish(&controller.read().unwrap(), viewport)
            .unwrap()
            .get("name"),
        Some(&Value::from(""))
    );
    assert!(actor.handle().close("p").wait().unwrap());
    let authored = FIXTURE.replace("Launch Film", r"A\ud800日本🙂");
    actor
        .handle()
        .open("p", OpenSource::Untitled(parse(&authored).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let value = session
        .publish(&controller.read().unwrap(), viewport)
        .unwrap();
    assert_eq!(value.get("name"), parse(r#""A\ud800日本🙂""#).ok().as_ref());
}

#[test]
fn native_popup_choices_share_worker_model_actions() {
    let (_actor, controller, viewport) = fixture();
    let mut session = Session::default();
    for argument in [
        Value::from("fit"),
        Value::Number(0.125),
        Value::from("viewer.zoom.in"),
        Value::from("viewer.zoom.out"),
        Value::from("actual"),
        Value::from("viewer.fit"),
    ] {
        let model = controller.read().unwrap();
        let publication = session.publish(&model, viewport).unwrap();
        let ticket = publication.get("ticket").and_then(Value::as_str).unwrap();
        finish(
            &controller,
            session
                .popup(
                    &model,
                    &request(ticket, "viewer.zoom", argument.clone()),
                    viewport,
                    Caller::native(),
                )
                .unwrap(),
        );
        let next = controller.read().unwrap();
        match argument.as_str() {
            Some("fit" | "viewer.fit") => assert!(next.chrome.navigation.fit),
            Some("actual") => assert_eq!(next.chrome.navigation.zoom, 1.0),
            _ => assert!(!next.chrome.navigation.fit),
        }
    }
    let model = controller.read().unwrap();
    let publication = session.publish(&model, viewport).unwrap();
    let ticket = publication.get("ticket").and_then(Value::as_str).unwrap();
    for argument in [Value::Null, Value::Number(7.0), Value::from("forged")] {
        assert!(
            session
                .popup(
                    &model,
                    &request(ticket, "viewer.zoom", argument),
                    viewport,
                    Caller::native()
                )
                .is_err()
        );
    }
    finish(
        &controller,
        session
            .popup(
                &model,
                &request(ticket, "viewer.quality", Value::from("0.5")),
                viewport,
                Caller::native(),
            )
            .unwrap(),
    );
    assert_eq!(controller.read().unwrap().chrome.quality, Quality::Half);
}
