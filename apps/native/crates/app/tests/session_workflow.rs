//! Exercise the app's chosen session/typed-command/gesture contract together.
use pm_kernel::{commands::properties::SetProperty, persistence::DiskFormat, session::*, *};
fn answer(k: &KernelHandle, call: ShellCall) -> ShellReply {
    k.shell(call).wait().unwrap().unwrap()
}
#[test]
fn library_gesture_is_one_undo_step_and_round_trips_through_pmv() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("profile");
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.path().join("journals")),
        ..Default::default()
    });
    let k = actor.handle();
    answer(
        &k,
        ShellCall::AttachLibrary {
            profile: profile.clone(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        },
    );
    let doc=pm_model::json::parse(r##"{"id":"test","name":"Integration","compId":"main","w":320,"h":180,"fps":30,"dur":2,"bg":"#000000","assets":{},"layers":[{"id":"layer","type":"shape","name":"Shape","from":0,"dur":2,"on":true,"p":{"opacity":{"v":100,"kf":[],"expr":null}}}],"comps":{}}"##).unwrap();
    let ShellReply::Opened { session, .. } = answer(
        &k,
        ShellCall::Open {
            id: "test".into(),
            source: SessionSource::New(doc),
            electron_profile: None,
        },
    ) else {
        panic!()
    };
    let begin = k
        .call_blocking(
            "test",
            Request::new(
                Caller::native(),
                GestureCall::Begin {
                    label: "Change Opacity".into(),
                },
            ),
        )
        .unwrap();
    let Reply::Gesture(gesture) = begin.reply else {
        panic!()
    };
    for value in [80., 65., 37.] {
        let a = k
            .call_blocking(
                "test",
                Request::new(
                    Caller::native(),
                    GestureCall::Step {
                        gesture,
                        body: WriteBody::Commands(vec![Command::Property(PropertyCommand::Set(
                            SetProperty::new("layer", "opacity", value, 0.),
                        ))]),
                        replace: true,
                    },
                ),
            )
            .unwrap();
        assert!(!matches!(a.reply, Reply::Throw(_)), "{:?}", a.reply);
    }
    k.call_blocking(
        "test",
        Request::new(Caller::native(), GestureCall::Commit { gesture }),
    )
    .unwrap();
    let p = k.published("test").unwrap();
    assert_eq!(p.history.entries, 1);
    assert_eq!(p.history.undo_label.as_deref(), Some("Change Opacity"));
    assert!(p.dirty);
    answer(&k, ShellCall::Save(session.clone()));
    assert!(!k.published("test").unwrap().dirty);
    let path = dir.path().join("project.pmv");
    answer(
        &k,
        ShellCall::SaveAs {
            session: session.clone(),
            path: path.clone(),
        },
    );
    answer(&k, ShellCall::Close(session));
    let ShellReply::Opened { session, .. } = answer(
        &k,
        ShellCall::Open {
            id: "reopened".into(),
            source: SessionSource::File(path),
            electron_profile: None,
        },
    ) else {
        panic!()
    };
    let p = k.published("reopened").unwrap();
    let mut q = QueryMirror::new(&p);
    assert_eq!(
        q.queries()
            .evaluate(None, "layer", "opacity", 0.)
            .unwrap()
            .as_f64(),
        Some(37.)
    );
    k.call_blocking(
        "reopened",
        Request::new(Caller::native(), HistoryCall::Undo),
    )
    .unwrap();
    let mut q = QueryMirror::new(&k.published("reopened").unwrap());
    assert_eq!(
        q.queries()
            .evaluate(None, "layer", "opacity", 0.)
            .unwrap()
            .as_f64(),
        Some(100.)
    );
    answer(&k, ShellCall::Close(session));
    let ShellReply::Recent(rows) = answer(&k, ShellCall::Recent) else {
        panic!()
    };
    assert!(!rows.is_empty());
    actor.shutdown();
}
