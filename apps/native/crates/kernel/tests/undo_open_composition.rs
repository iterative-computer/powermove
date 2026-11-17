//! PM-475: undoing New Composition while that composition is open works: the
//! editor goes back to the composition open before it, then the new one is
//! removed, in the one undo step; and no refusal names a composition by id.
use pm_kernel::commands::comps::NewComp;
use pm_kernel::tx::{Call, Caller, HistoryCall, Reply, Request};
use pm_kernel::{Command, CompCommand, Kernel, OpenSource, WriteBody};
use pm_model::json::parse;

const PROJECT: &str = r##"{
  "id": "P", "name": "Fixture", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 2, "bg": "#000000", "assets": {},
  "layers": [
    {"id": "layer0", "type": "shape", "name": "Layer", "from": 0, "dur": 2, "on": true,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}}
  ],
  "comps": {"C2": {"id": "C2", "name": "Second", "w": 10, "h": 10, "fps": 30, "dur": 2, "layers": []}}
}"##;

fn call(k: &mut Kernel, c: impl Into<Call>) -> Reply {
    k.handle("P", Request::new(Caller::native(), c))
        .unwrap()
        .reply
        .unwrap()
        .reply
}
fn transact(k: &mut Kernel, label: &str, commands: Vec<Command>) {
    let reply = call(
        k,
        Call::Transact {
            label: Some(label.into()),
            body: WriteBody::Commands(commands),
            expected_revision: None,
        },
    );
    assert!(!matches!(reply, Reply::Throw(_)), "{label}: {reply:?}");
}
fn open_comp(k: &mut Kernel, comp: &str) {
    let document = k.project("P").unwrap().store().doc_id();
    let reply = call(
        k,
        Call::OpenComposition {
            comp_id: comp.into(),
            document,
        },
    );
    assert!(!matches!(reply, Reply::Throw(_)), "open {comp}: {reply:?}");
}
fn open_now(k: &Kernel) -> Option<String> {
    pm_kernel::model::open_comp_id(k.project("P").unwrap().store().root().unwrap())
}
fn has_comp(k: &Kernel, comp: &str) -> bool {
    k.project("P").unwrap().store().root().unwrap().get("comps").and_then(|c| c.get(comp)).is_some()
}
fn undo(k: &mut Kernel) -> Reply {
    call(k, HistoryCall::Undo)
}
fn new_comp_open_with_a_layer(k: &mut Kernel) {
    transact(
        k,
        "New Composition",
        vec![CompCommand::Create {
            comp_id: "C9".into(),
            settings: NewComp::default(),
        }
        .into()],
    );
    open_comp(k, "C9");
    // Something done after it, then undone.
    transact(
        k,
        "Rename",
        vec![Command::SetLayerName {
            layer_id: "layer0".into(),
            name: "Renamed".into(),
        }],
    );
}

#[test]
fn undoing_new_composition_while_it_is_open_goes_back_and_removes_it() {
    let mut k = Kernel::new(Default::default());
    k.open("P", OpenSource::Untitled(parse(PROJECT).unwrap())).unwrap();
    let before = open_now(&k);
    new_comp_open_with_a_layer(&mut k);
    assert_eq!(open_now(&k).as_deref(), Some("C9"));
    assert!(matches!(undo(&mut k), Reply::Bool(true)), "undo the rename");
    let reply = undo(&mut k);
    assert!(matches!(reply, Reply::Bool(true)), "undo New Composition: {reply:?}");
    assert!(!has_comp(&k, "C9"), "the new composition is removed");
    assert_eq!(open_now(&k), before, "the composition open before it is open again");
    // Redo brings it back; the editor stays where it is.
    assert!(matches!(call(&mut k, HistoryCall::Redo), Reply::Bool(true)));
    assert!(has_comp(&k, "C9"));
}

#[test]
fn the_previous_composition_is_the_one_open_before_not_the_main_one() {
    let mut k = Kernel::new(Default::default());
    k.open("P", OpenSource::Untitled(parse(PROJECT).unwrap())).unwrap();
    open_comp(&mut k, "C2");
    new_comp_open_with_a_layer(&mut k);
    undo(&mut k);
    assert!(matches!(undo(&mut k), Reply::Bool(true)));
    assert_eq!(open_now(&k).as_deref(), Some("C2"));
}

/// A refusal names no composition by its id.
#[test]
fn a_missing_open_composition_is_not_named_by_id() {
    let message = pm_kernel::document::OPEN_COMP_MISSING;
    assert!(!message.contains("C9") && !message.contains('{'), "{message}");
}
