use pm_kernel::state::{Registration, StateOp, StateTransaction};
use pm_kernel::{
    Call, Caller, ContentCommand, HistoryCall, Kernel, KernelConfig, OpenSource, Reply, Request,
    Writer,
};
use pm_model::{Value, json::parse};

const SOURCE: &str = r##"{"id":"P","name":"Synthetic","w":640,"h":360,"fps":30,"dur":5,"layers":[{"id":"e","type":"extension","from":0,"dur":5,"d":{"definition":"synthetic.extension","version":"1","w":100,"h":100,"data":{"keep":"\ud800"},"params":{"amount":{"v":2,"kf":[{"id":"k","t":1,"v":5}],"extra":true},"tint":{"v":"#FFFFFF"},"visible":{"v":true},"opaque":{"v":{"future":1}}}}}],"comps":{}}"##;
fn kernel() -> Kernel {
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel
        .open("P", OpenSource::Untitled(parse(SOURCE).unwrap()))
        .unwrap();
    kernel
}
fn definition(kernel: &mut Kernel, version: &str) {
    let definition=parse(&format!(r##"{{"version":"{version}","params":[{{"k":"amount","type":"number","label":"Amount","min":0,"max":10}},{{"k":"tint","type":"color","label":"Tint"}},{{"k":"visible","type":"toggle","label":"Visible"}}]}}"##)).unwrap();
    kernel
        .state_transaction(StateTransaction {
            expected_revision: None,
            ops: vec![StateOp::Register {
                registry: "layerTypes".into(),
                entry: Registration {
                    token: format!("provider-{version}"),
                    owner: Writer {
                        realm: 1,
                        generation: 1,
                    },
                    id: "synthetic.extension".into(),
                    definition,
                },
            }],
        })
        .unwrap();
}
fn set(kernel: &mut Kernel, patch: &str) -> pm_kernel::Ack {
    kernel
        .handle(
            "P",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: None,
                    expected_revision: None,
                    body: pm_kernel::Command::Content(ContentCommand::Set {
                        layer_id: "e".into(),
                        patch: parse(patch).unwrap().as_object().unwrap().clone(),
                        time: 0.,
                    })
                    .into(),
                },
            ),
        )
        .unwrap()
        .reply
        .unwrap()
}
fn document(kernel: &Kernel) -> Value {
    kernel
        .project("P")
        .unwrap()
        .store()
        .snapshot()
        .document
        .to_value()
        .unwrap()
}
#[test]
fn actual_actor_resolves_winning_definition_and_keeps_opaque_data_and_animation() {
    let mut kernel = kernel();
    definition(&mut kernel, "1");
    let before = document(&kernel);
    let ack = set(
        &mut kernel,
        r##"{"version":"1","w":0,"h":20000,"data":{"new":true},"parameters":{"amount":90,"tint":"#aabbccdd","visible":false,"opaque":{"next":2}}}"##,
    );
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{ack:?}");
    let after = document(&kernel);
    let d = after.get("layers").unwrap().as_array().unwrap()[0]
        .get("d")
        .unwrap();
    assert_eq!(d.get("w"), Some(&Value::Number(1.)));
    assert_eq!(d.get("h"), Some(&Value::Number(16384.)));
    assert_eq!(
        d.get("data").unwrap().get("keep"),
        parse(r#""\ud800""#).ok().as_ref()
    );
    let params = d.get("params").unwrap();
    assert_eq!(
        params.get("amount").unwrap().get("v"),
        Some(&Value::Number(10.))
    );
    assert_eq!(
        params.get("amount").unwrap().get("kf"),
        before.get("layers").unwrap().as_array().unwrap()[0]
            .get("d")
            .unwrap()
            .get("params")
            .unwrap()
            .get("amount")
            .unwrap()
            .get("kf")
    );
    assert_eq!(
        params.get("tint").unwrap().get("v").and_then(Value::as_str),
        Some("#AABBCCDD")
    );
    assert_eq!(
        params.get("opaque").unwrap().get("v"),
        parse(r#"{"next":2}"#).ok().as_ref()
    );
    kernel
        .handle("P", Request::new(Caller::native(), HistoryCall::Undo))
        .unwrap();
    assert_eq!(document(&kernel), before);
    kernel
        .handle("P", Request::new(Caller::native(), HistoryCall::Redo))
        .unwrap();
    assert_eq!(document(&kernel), after);
    definition(&mut kernel, "2");
    assert!(matches!(
        set(&mut kernel, r#"{"version":"1"}"#).reply,
        Reply::Throw(_)
    ));
    assert!(!matches!(
        set(&mut kernel, r#"{"version":"2"}"#).reply,
        Reply::Throw(_)
    ));
}
#[test]
fn invalid_extension_content_refuses_atomically_and_missing_definitions_keep_existing_values() {
    let mut kernel = kernel();
    definition(&mut kernel, "1");
    let before = document(&kernel);
    let rev = kernel.project("P").unwrap().store().revision();
    for patch in [
        r#"{"w":300,"parameters":{"amount":3,"tint":"red"}}"#,
        r#"{"parameters":{"visible":1}}"#,
        r#"{"parameters":{"new":1}}"#,
        r#"{"definition":"another"}"#,
        r#"{"data":{"constructor":true}}"#,
        r#"{"code":"invalid"}"#,
    ] {
        assert!(
            matches!(set(&mut kernel, patch).reply, Reply::Throw(_)),
            "{patch}"
        );
        assert_eq!(document(&kernel), before);
        assert_eq!(kernel.project("P").unwrap().store().revision(), rev);
    }
    let mut missing = super_missing();
    assert!(!matches!(
        set(
            &mut missing,
            r#"{"data":{"future":true},"parameters":{"opaque":[1,2]}}"#
        )
        .reply,
        Reply::Throw(_)
    ));
    assert!(matches!(
        set(&mut missing, r#"{"version":"1"}"#).reply,
        Reply::Throw(_)
    ));
}
fn super_missing() -> Kernel {
    kernel()
}
