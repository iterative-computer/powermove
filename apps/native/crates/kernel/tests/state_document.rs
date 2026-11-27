use pm_kernel::state::{StateOp, StateTransaction, UiState};
use pm_kernel::{Kernel, KernelConfig, OpenSource};
use pm_model::{Value, json::parse};
#[test]
fn delayed_ui_publication_cannot_write_a_replacement_document_or_other_transaction_slots() {
    let source =
        || parse(r#"{"id":"P","w":320,"h":180,"fps":30,"dur":2,"layers":[],"assets":{}}"#).unwrap();
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel.open("P", OpenSource::Untitled(source())).unwrap();
    let document = kernel.project("P").unwrap().store().doc_id();
    let mut ui = UiState::default();
    ui.viewer
        .as_object_mut()
        .unwrap()
        .insert("studioPreview", Value::from("quality"));
    let update = StateTransaction {
        expected_revision: None,
        ops: vec![
            StateOp::Setting {
                key: "ext.synthetic-ui".into(),
                value: Some(Value::Bool(true)),
            },
            StateOp::UiDocument {
                project: "P".into(),
                document,
                next: ui,
            },
        ],
    };
    kernel.state_transaction(update.clone()).unwrap();
    let state = kernel.state().clone();
    kernel.close("P");
    kernel.open("P", OpenSource::Untitled(source())).unwrap();
    let replacement = kernel.state().clone();
    let mut stale = update;
    stale.ops[0] = StateOp::Setting {
        key: "ext.synthetic-ui".into(),
        value: Some(Value::Bool(false)),
    };
    assert!(kernel.state_transaction(stale).is_err());
    assert_eq!(kernel.state().revision, state.revision);
    assert_eq!(kernel.state().settings, state.settings);
    assert_eq!(kernel.state().ui, replacement.ui);
    let document = kernel.project("P").unwrap().store().doc_id();
    kernel
        .state_transaction(StateTransaction {
            expected_revision: Some(state.revision),
            ops: vec![StateOp::UiDocument {
                project: "P".into(),
                document,
                next: UiState::default(),
            }],
        })
        .unwrap();
    assert_ne!(kernel.state().revision, state.revision);
}
