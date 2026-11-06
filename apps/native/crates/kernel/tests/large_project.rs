//! A long keyframed project. The Electron journal fix found one edit could cost
//! twice the project: a 358 MB undo entry and a 179 MB journal line on 180 MB.
mod common;
use common::TempDir;
use pm_kernel::journal::{Recovery, sessions};
use pm_kernel::*;
use pm_model::json::{parse, to_canonical_string};
use pm_model::ops::Op;
use pm_model::patch::{Patch, PathPart};
use pm_model::{Map, Value};

/// Plenty to tell a value from an operation; large enough to count as large.
/// `PM_LARGE_PROJECT_BYTES` runs the real size (180 MB) in a release build.
fn target_bytes() -> usize {
    std::env::var("PM_LARGE_PROJECT_BYTES")
        .ok()
        .and_then(|bytes| bytes.parse().ok())
        .unwrap_or(8 << 20)
}
/// What one edit may cost, in history and in the journal, on any project size.
const EDIT_BYTES: usize = 32 << 10;

fn project() -> Value {
    let keys = (0..120)
        .map(|n| format!(r#"{{"id":"k{n}","t":{n},"v":{n},"e":"linear"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let mut layers = vec![];
    let mut bytes = 0;
    let target = target_bytes();
    while bytes < target {
        let id = layers.len();
        let layer = format!(
            r#"{{"id":"L{id}","type":"shape","name":"Layer {id}","from":0,"dur":2,"on":true,"p":{{"opacity":{{"v":100,"kf":[{keys}],"expr":null}},"position.x":{{"v":0,"kf":[{keys}],"expr":null}}}}}}"#
        );
        bytes += layer.len();
        layers.push(layer);
    }
    parse(&format!(
        r##"{{"id":"big","name":"Big","compId":"main","compName":"Main","w":320,"h":180,"fps":30,"dur":2,"bg":"#000000","assets":{{"zebra":{{"name":"Z","kind":"image"}},"apple":{{"name":"A","kind":"image"}}}},"nativeAssetOrder":{{"zebra":0,"apple":1}},"comps":{{}},"layers":[{}]}}"##,
        layers.join(",")
    ))
    .unwrap()
}
fn add_layer(id: &str) -> Command {
    LayerCommand::Add(Box::new(pm_kernel::commands::layers::AddLayer {
        id: id.into(),
        layer_type: "shape".into(),
        comp: None,
        name: Some(id.into()),
        from: Some(0.),
        duration: Some(1.),
        content: Map::new(),
        properties: Map::new(),
        color: None,
        index: None,
        parent: None,
        blend: None,
        motion_blur: None,
        visible: None,
        shy: None,
        collapsed: None,
        time: 0.,
    }))
    .into()
}
fn layers_path() -> Vec<PathPart> {
    ["comps", "main", "layers"]
        .map(|k| PathPart::Key(k.into()))
        .to_vec()
}
fn layer_ids(store: &DocumentStore) -> Vec<String> {
    let comp = pm_kernel::model::comp_node(store.root().unwrap(), Some("main")).unwrap();
    comp.to_value()
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.get("id").unwrap().as_str().unwrap().to_owned())
        .collect()
}

fn call(kernel: &mut Kernel, call: impl Into<Call>) {
    let outcome = kernel
        .handle("big", Request::new(Caller::native(), call.into()))
        .unwrap();
    let reply = outcome.reply.expect("a synchronous reply").reply;
    assert!(!matches!(reply, Reply::Throw(_)), "{reply:?}");
}
fn commands(label: &str, list: Vec<Command>) -> Call {
    Call::Transact {
        label: Some(label.into()),
        body: WriteBody::Commands(list),
        expected_revision: None,
    }
}
fn kernel() -> Kernel {
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel.open("big", OpenSource::Untitled(project())).unwrap();
    kernel
}
fn store(kernel: &Kernel) -> &DocumentStore {
    kernel.project("big").unwrap().store()
}

#[test]
fn an_edit_costs_what_it_changed_on_any_project_size_and_saves_and_reopens() {
    let dir = TempDir::new("large-project");
    let mut kernel = kernel();
    call(&mut kernel, commands("Add", vec![add_layer("new-a")]));
    let cost = |k: &Kernel| store(k).history().records().last().unwrap().bytes;
    assert!(
        cost(&kernel) < EDIT_BYTES,
        "typed edit: {} bytes",
        cost(&kernel)
    );

    // An extension assigning the whole array to add one layer.
    let comp = pm_kernel::model::comp_node(store(&kernel).root().unwrap(), Some("main")).unwrap();
    let mut layers = comp.to_value().get("layers").unwrap().clone();
    let extra =
        parse(r#"{"id":"new-b","type":"shape","name":"B","from":0,"dur":1,"on":true,"p":{}}"#);
    layers.as_array_mut().unwrap().push(extra.unwrap());
    call(
        &mut kernel,
        Call::Transact {
            label: Some("Assign".into()),
            body: WriteBody::Ops(vec![Op::Leaf(Patch::set(layers_path(), layers))]),
            expected_revision: None,
        },
    );
    assert!(
        cost(&kernel) < EDIT_BYTES,
        "whole-array write: {} bytes",
        cost(&kernel)
    );
    assert!(store(&kernel).history().stats().bytes < 2 * EDIT_BYTES);
    assert_eq!(
        store(&kernel).history().len(),
        2,
        "no edit trimmed the others"
    );

    // Undo works from operations, and they survive a save and reopen.
    let path = dir.path("big.pmv");
    let saved = store(&kernel).snapshot();
    let mut file = DocumentStore::new_untitled(saved.document.to_value().unwrap());
    let history = store(&kernel).history().snapshot().to_saved();
    common::save_as(&mut file, &path).unwrap();
    let mut with_history = pm_kernel::project_io::open_project(&path, Default::default()).unwrap();
    with_history.history = Some(History::import(&history, Some("main")).unwrap());
    let mut reopened = DocumentStore::open(with_history);
    let ids = layer_ids(&reopened);
    assert!(ids.contains(&"new-b".to_owned()));
    reopened.undo().unwrap();
    assert!(!layer_ids(&reopened).contains(&"new-b".to_owned()));
    reopened.undo().unwrap();
    assert!(!layer_ids(&reopened).contains(&"new-a".to_owned()));
    reopened.redo().unwrap();
    assert!(layer_ids(&reopened).contains(&"new-a".to_owned()));
}

#[test]
fn undo_skips_what_a_write_outside_history_removed_and_stays_usable() {
    let mut kernel = kernel();
    call(&mut kernel, commands("Add", vec![add_layer("gone")]));
    call(&mut kernel, commands("Add", vec![add_layer("kept")]));
    // Remove `gone` where history cannot see, then undo over that entry.
    let stale = store(&kernel).history().records()[0].inverse.to_vec();
    call(&mut kernel, Call::DirectWrite { ops: stale });
    assert!(!layer_ids(store(&kernel)).contains(&"gone".to_owned()));
    let undo = |k: &mut Kernel| call(k, HistoryCall::Undo);
    undo(&mut kernel);
    assert!(!layer_ids(store(&kernel)).contains(&"kept".to_owned()));
    undo(&mut kernel);
    assert!(store(&kernel).history().can_redo());
    assert!(
        !store(&kernel).history().can_undo(),
        "a stale operation must not wedge Undo"
    );
    call(&mut kernel, HistoryCall::Redo);
    assert!(layer_ids(store(&kernel)).contains(&"gone".to_owned()));
}

#[test]
fn a_large_document_checkpoints_by_journal_size_and_recovers_at_any_point() {
    let dir = TempDir::new("large-journal");
    let root = dir.path("journals");
    let mut kernel = Kernel::new(KernelConfig {
        journal_root: Some(root.clone()),
        ..Default::default()
    });
    kernel.open("big", OpenSource::Untitled(project())).unwrap();
    // The first checkpoint measures the document.
    kernel.durability_barrier().unwrap();
    let session = sessions(&root.join("big")).pop().unwrap();
    let checkpoints = || {
        std::fs::read_dir(&session)
            .unwrap()
            .filter(|e| {
                let name = e.as_ref().unwrap().file_name();
                name.to_string_lossy().starts_with("checkpoint-")
            })
            .count()
    };
    let journal_bytes = || -> u64 {
        std::fs::read_dir(&session)
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| e.file_name().to_string_lossy().starts_with("journal-"))
            .map(|e| e.metadata().unwrap().len())
            .sum()
    };
    assert_eq!(checkpoints(), 1);
    let start = journal_bytes();
    for n in 0..120 {
        let id = format!("added-{n}");
        call(&mut kernel, commands("Add", vec![add_layer(&id)]));
    }
    kernel.durability_barrier().unwrap();
    assert_eq!(checkpoints(), 1, "fifty edits rewrite nothing");
    let per_edit = (journal_bytes() - start) / 120;
    assert!(
        (per_edit as usize) < EDIT_BYTES,
        "journal: {per_edit} bytes per edit"
    );

    // A crash: recovery rebuilds the document from checkpoint and lines.
    let expected = store(&kernel).root().unwrap().to_value();
    drop(kernel);
    let Recovery::Recovered(recovered) = pm_kernel::journal::recover(&root.join("big"), None, None)
    else {
        panic!("the journal recovers");
    };
    let document = recovered.document.root().unwrap().to_value();
    assert!(to_canonical_string(&document) == to_canonical_string(&expected));
    assert_eq!(
        pm_kernel::asset_order::keys(&document),
        ["zebra", "apple"],
        "authored media order is journaled"
    );
}
