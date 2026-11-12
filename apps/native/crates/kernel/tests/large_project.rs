//! A long keyframed project. The Electron journal fix found one edit could cost
//! twice the project: a 358 MB undo entry and a 179 MB journal line on 180 MB.
mod common;
use common::TempDir;
use pm_kernel::journal::{Recovery, sessions};
use pm_kernel::session::*;
use pm_kernel::*;
use pm_model::container::{ReadOptions, read_container};
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
/// What one edit may cost, in history and in the journal, once a member passes
/// the whole-value limit. Under 1 MiB an entry keeps the form shipped readers
/// load, so it can cost up to that limit.
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
fn open_file(kernel: &mut Kernel, path: &std::path::Path) -> SessionHandle {
    match kernel
        .shell(ShellCall::Open {
            id: "big".into(),
            source: SessionSource::File(path.into()),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_edit_on_a_large_project_costs_what_it_changed_and_its_history_survives_the_file() {
    let dir = TempDir::new("large-project");
    let source = dir.path("source.pmv");
    let mut untitled = DocumentStore::new_untitled(project());
    common::save_as(&mut untitled, &source).unwrap();
    drop(untitled);
    let mut kernel = Kernel::new(KernelConfig {
        journal_root: Some(dir.path("journals")),
        ..Default::default()
    });
    let session = open_file(&mut kernel, &source);
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

    // Undo works from operations, and they survive the file itself.
    let path = dir.path("big.pmv");
    let plan = kernel
        .prepare_session_save(session, Some(path.clone()), false)
        .unwrap();
    kernel.finish_session_save(plan.run()).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let container = read_container(
        &file,
        ReadOptions {
            verify_media: false,
        },
    )
    .unwrap();
    assert_eq!(
        container
            .document
            .get("requiresEngine")
            .and_then(Value::as_str),
        Some("native"),
        "a reader that drops operation history cannot represent this file"
    );
    let opened = pm_kernel::project_io::open_project(&path, Default::default()).unwrap();
    assert!(opened.history.is_some(), "the file carries its history");
    let mut reopened = DocumentStore::open(opened);
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
fn undo_over_a_stale_operation_recovers_after_a_crash() {
    let dir = TempDir::new("stale-undo");
    let root = dir.path("journals");
    let mut kernel = Kernel::new(KernelConfig {
        journal_root: Some(root.clone()),
        ..Default::default()
    });
    kernel.open("big", OpenSource::Untitled(project())).unwrap();
    call(&mut kernel, commands("Add", vec![add_layer("gone")]));
    call(&mut kernel, commands("Add", vec![add_layer("kept")]));
    let stale = store(&kernel).history().records()[0].inverse.to_vec();
    call(&mut kernel, Call::DirectWrite { ops: stale });
    call(&mut kernel, HistoryCall::Undo);
    call(&mut kernel, HistoryCall::Undo);
    call(&mut kernel, HistoryCall::Redo);
    call(&mut kernel, HistoryCall::Undo);
    kernel.durability_barrier().unwrap();
    let expected = store(&kernel).root().unwrap().to_value();
    drop(kernel);
    let recovered = match pm_kernel::journal::recover(&root.join("big"), None, None) {
        Recovery::Recovered(recovered) => recovered,
        _ => panic!("a skipped operation must not poison the journal"),
    };
    let document = recovered.document.root().unwrap().to_value();
    assert!(to_canonical_string(&document) == to_canonical_string(&expected));
}

#[test]
fn undo_folded_into_an_open_entry_records_only_what_applied() {
    let mut kernel = kernel();
    call(&mut kernel, commands("Add", vec![add_layer("gone")]));
    let stale = store(&kernel).history().records()[0].inverse.to_vec();
    call(&mut kernel, Call::DirectWrite { ops: stale });
    call(
        &mut kernel,
        HistoryCall::Begin {
            label: "Run".into(),
        },
    );
    call(&mut kernel, HistoryCall::Undo);
    call(&mut kernel, HistoryCall::Commit { label: None });
    // Nothing applied, so the entry has nothing to reverse into.
    call(&mut kernel, commands("Add", vec![add_layer("next")]));
    call(&mut kernel, HistoryCall::Undo);
    assert!(!layer_ids(store(&kernel)).contains(&"gone".to_owned()));
}

#[test]
fn undo_keeps_the_originals_of_a_layer_it_would_reinsert() {
    let mut kernel = kernel();
    let path = layers_path();
    let layer = parse(
        r#"{"id":"model","type":"shape","name":"M","from":0,"dur":1,"on":true,"p":{},"native3dSources":[{"id":"original-a"}]}"#,
    )
    .unwrap();
    let insert = Op::Insert {
        path: path.clone(),
        index: 0,
        value: layer,
    };
    let delete = Op::Delete {
        path,
        id: "model".into(),
    };
    let transact = |ops| Call::Transact {
        label: Some("Edit".into()),
        body: WriteBody::Ops(ops),
        expected_revision: None,
    };
    call(&mut kernel, transact(vec![insert]));
    call(&mut kernel, HistoryCall::Clear);
    call(&mut kernel, transact(vec![delete]));
    assert!(!layer_ids(store(&kernel)).contains(&"model".to_owned()));
    assert!(store(&kernel).reachable_assets().contains("original-a"));
}

/// What the history reaches is asked of every step on every save, and each
/// step answers from what it found the first time: steps added later are
/// found too, and steps that leave the history take theirs with them.
#[test]
fn the_assets_history_reaches_follow_its_steps_across_saves() {
    let mut kernel = kernel();
    let model = |id: &str, original: &str| {
        let layer = parse(&format!(
            r#"{{"id":"{id}","type":"shape","name":"M","from":0,"dur":1,"on":true,"p":{{}},"native3dSources":[{{"id":"{original}"}}]}}"#
        ))
        .unwrap();
        let transact = |ops| Call::Transact {
            label: Some("Edit".into()),
            body: WriteBody::Ops(ops),
            expected_revision: None,
        };
        (
            transact(vec![Op::Insert {
                path: layers_path(),
                index: 0,
                value: layer,
            }]),
            transact(vec![Op::Delete {
                path: layers_path(),
                id: id.into(),
            }]),
        )
    };
    let reaches = |kernel: &Kernel, id: &str| store(kernel).reachable_assets().contains(id);
    let (insert, delete) = model("first", "original-a");
    call(&mut kernel, insert);
    call(&mut kernel, HistoryCall::Clear);
    call(&mut kernel, delete);
    // Asked once (as a save asks): the step that would reinsert it answers.
    assert!(reaches(&kernel, "original-a") && !reaches(&kernel, "original-b"));
    let (insert, delete) = model("second", "original-b");
    call(&mut kernel, insert);
    call(&mut kernel, delete);
    assert!(reaches(&kernel, "original-a") && reaches(&kernel, "original-b"));
    // Undone and redone steps are the same steps.
    call(&mut kernel, HistoryCall::Undo);
    call(&mut kernel, HistoryCall::Undo);
    assert!(reaches(&kernel, "original-a") && reaches(&kernel, "original-b"));
    call(&mut kernel, HistoryCall::Redo);
    call(&mut kernel, HistoryCall::Redo);
    assert!(reaches(&kernel, "original-a") && reaches(&kernel, "original-b"));
    // A new step after an undo drops the undone ones, and what only they reached.
    call(&mut kernel, HistoryCall::Undo);
    call(&mut kernel, HistoryCall::Undo);
    call(&mut kernel, commands("Add", vec![add_layer("other")]));
    assert!(reaches(&kernel, "original-a") && !reaches(&kernel, "original-b"));
    call(&mut kernel, HistoryCall::Clear);
    assert!(!reaches(&kernel, "original-a"));
}

#[test]
fn a_write_outside_history_stays_unsaved_until_a_save_covers_it() {
    let dir = TempDir::new("dirty");
    let source = dir.path("source.pmv");
    let mut untitled = DocumentStore::new_untitled(project());
    common::save_as(&mut untitled, &source).unwrap();
    drop(untitled);
    let mut kernel = Kernel::new(KernelConfig {
        journal_root: Some(dir.path("journals")),
        ..Default::default()
    });
    let session = open_file(&mut kernel, &source);
    let save = |k: &mut Kernel| {
        let plan = k
            .prepare_session_save(session.clone(), None, false)
            .unwrap();
        k.finish_session_save(plan.run()).unwrap();
    };
    let dirty = |k: &Kernel| store(k).is_dirty();
    call(&mut kernel, commands("Add", vec![add_layer("saved")]));
    save(&mut kernel);
    assert!(!dirty(&kernel));

    // Edit and Undo alone come back clean, by position.
    call(&mut kernel, commands("Add", vec![add_layer("edit")]));
    assert!(dirty(&kernel));
    call(&mut kernel, HistoryCall::Undo);
    assert!(!dirty(&kernel));
    call(&mut kernel, HistoryCall::Redo);
    call(&mut kernel, HistoryCall::Undo);
    assert!(!dirty(&kernel));

    // A direct write at the saved position does not.
    let direct = |at: &str| Call::DirectWrite {
        ops: vec![Op::Leaf(Patch::set(
            vec![PathPart::Key("name".into())],
            Value::from(at),
        ))],
    };
    call(&mut kernel, direct("Direct"));
    call(&mut kernel, commands("Add", vec![add_layer("after")]));
    call(&mut kernel, HistoryCall::Undo);
    assert!(dirty(&kernel), "the direct write is not in the file");
    save(&mut kernel);
    assert!(!dirty(&kernel));
    call(&mut kernel, HistoryCall::Redo);
    assert!(dirty(&kernel));
    call(&mut kernel, HistoryCall::Undo);
    assert!(!dirty(&kernel), "the save covers it now");

    // A direct write while a save runs is not covered by that save.
    let plan = kernel
        .prepare_session_save(session.clone(), None, false)
        .unwrap();
    call(&mut kernel, direct("During"));
    kernel.finish_session_save(plan.run()).unwrap();
    assert!(dirty(&kernel));
    call(&mut kernel, commands("Add", vec![add_layer("later")]));
    call(&mut kernel, HistoryCall::Undo);
    assert!(dirty(&kernel));

    // Undo that skipped an operation is not the position it lands on.
    save(&mut kernel);
    call(&mut kernel, commands("Add", vec![add_layer("stale")]));
    let stale = store(&kernel)
        .history()
        .records()
        .last()
        .unwrap()
        .inverse
        .to_vec();
    save(&mut kernel);
    call(&mut kernel, Call::DirectWrite { ops: stale });
    save(&mut kernel);
    assert!(!dirty(&kernel));
    call(&mut kernel, HistoryCall::Undo);
    assert!(dirty(&kernel));
    call(&mut kernel, HistoryCall::Redo);
    assert!(dirty(&kernel), "Redo re-added a layer the save had not");
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

/// A history stored as a dictionary (main's packed form; the library keeps
/// every project's history so) is one this kernel reads and writes in full:
/// editing it must not archive it. Archived, every save of a library project
/// after an edit kept another full copy of its history (a 285 MB project
/// grew 100 MB a save).
#[test]
fn an_edited_dictionary_history_is_not_archived() {
    let dir = TempDir::new("dictionary-history");
    let layer = |id: &str| {
        format!(
            r#"{{"id":"{id}","type":"shape","name":"{id}","from":0,"dur":2,"on":true,"p":{{"opacity":{{"v":100,"kf":[],"expr":null}}}}}}"#
        )
    };
    let (one, two) = (layer("a"), format!("{},{}", layer("a"), layer("b")));
    let history = parse(&format!(
        r#"{{"version":1,"paths":"composition","index":0,"entries":[{{"label":"Add","forward":[{{"path":["comps","main","layers"],"exists":true,"value":[{two}]}}],"backward":[{{"path":["comps","main","layers"],"exists":true,"value":[{one}]}}]}}]}}"#
    ))
    .unwrap();
    let packed = pm_kernel::history::pack(history);
    assert!(packed.get("format").is_some(), "the input is a dictionary");
    let source = dir.path("source.json");
    std::fs::write(
        &source,
        format!(
            r##"{{"v":"1.0.0","proj":{{"id":"dict","name":"Dict","compId":"main","compName":"Main","w":320,"h":180,"fps":30,"dur":2,"bg":"#000000","assets":{{}},"comps":{{}},"layers":[{two}]}},"history":{}}}"##,
            to_canonical_string(&packed)
        ),
    )
    .unwrap();
    let mut kernel = Kernel::new(KernelConfig::default());
    let session = open_file(&mut kernel, &source);
    assert!(
        store(&kernel).history().can_undo(),
        "the dictionary is read"
    );
    call(&mut kernel, commands("Add", vec![add_layer("c")]));
    let path = dir.path("saved.pmv");
    let plan = kernel
        .prepare_session_save(session, Some(path.clone()), false)
        .unwrap();
    kernel.finish_session_save(plan.run()).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let saved = read_container(
        &file,
        ReadOptions {
            verify_media: false,
        },
    )
    .unwrap()
    .document;
    assert!(saved.get("nativeHistoryArchive").is_none());
    let reopened = pm_kernel::project_io::open_project(&path, Default::default()).unwrap();
    assert_eq!(reopened.history.unwrap().len(), 2, "both edits are kept");
}

/// A valid dictionary whose shared records expand past the history byte
/// budget opens trimmed: the trimmed steps exist only in the file, so the
/// first save after an edit archives the history it replaces.
#[test]
fn a_dictionary_trimmed_on_open_is_archived() {
    let dir = TempDir::new("trimmed-history");
    let big = format!(
        r#"{{"id":"a","type":"shape","name":"{}"}}"#,
        "x".repeat((pm_kernel::history::DEFAULT_MAX_BYTES / 2) + (1 << 20))
    );
    let small = r#"{"id":"b","type":"shape","name":"b"}"#;
    let side = |r: usize| {
        format!(r#"[{{"patch":{{"path":["comps","main","layers"],"exists":true}},"refs":[{r}]}}]"#)
    };
    let packed = format!(
        r#"{{"format":"powermove-history-dictionary","version":1,"records":[{big},{small}],"history":{{"version":1,"paths":"composition","index":1,"entries":[{{"label":"Big","forward":{f0},"backward":{f0}}},{{"label":"Small","forward":{f1},"backward":{f1}}}]}}}}"#,
        f0 = side(0),
        f1 = side(1),
    );
    drop(big);
    let source = dir.path("source.json");
    std::fs::write(
        &source,
        format!(
            r##"{{"v":"1.0.0","proj":{{"id":"trim","name":"Trim","compId":"main","compName":"Main","w":320,"h":180,"fps":30,"dur":2,"bg":"#000000","assets":{{}},"comps":{{}},"layers":[{small}]}},"history":{packed}}}"##
        ),
    )
    .unwrap();
    let original = parse(&packed).unwrap();
    drop(packed);
    let mut kernel = Kernel::new(KernelConfig::default());
    let session = open_file(&mut kernel, &source);
    assert_eq!(
        store(&kernel).history().len(),
        1,
        "the oldest step is trimmed"
    );
    call(&mut kernel, commands("Add", vec![add_layer("c")]));
    let path = dir.path("saved.pmv");
    let plan = kernel
        .prepare_session_save(session.clone(), Some(path.clone()), false)
        .unwrap();
    kernel.finish_session_save(plan.run()).unwrap();
    let archived = |why: &str| {
        let file = std::fs::File::open(&path).unwrap();
        let saved = read_container(
            &file,
            ReadOptions {
                verify_media: false,
            },
        )
        .unwrap()
        .document;
        assert!(
            saved.get("nativeHistoryArchive") == Some(&original),
            "{why}"
        );
    };
    archived("the trimmed history is archived as the file held it");
    // A later save judges the same file history the same way (the judgment
    // is kept, not made again): the archive stays, once.
    call(&mut kernel, commands("Add", vec![add_layer("d")]));
    let plan = kernel.prepare_session_save(session, None, false).unwrap();
    kernel.finish_session_save(plan.run()).unwrap();
    archived("a second save keeps the archive, once");
}

/// A large project's history holds this kernel's own operations (an insert
/// into a large layer list): reopened, edited and saved, it is not archived.
#[test]
fn an_edited_operation_history_is_not_archived() {
    let dir = TempDir::new("operation-history");
    let source = dir.path("source.pmv");
    let mut untitled = DocumentStore::new_untitled(project());
    common::save_as(&mut untitled, &source).unwrap();
    drop(untitled);
    let document = |path: &std::path::Path| {
        let file = std::fs::File::open(path).unwrap();
        read_container(
            &file,
            ReadOptions {
                verify_media: false,
            },
        )
        .unwrap()
        .document
    };
    let mut previous = source;
    for round in 0..2 {
        let mut kernel = Kernel::new(KernelConfig::default());
        let session = open_file(&mut kernel, &previous);
        call(
            &mut kernel,
            commands("Add", vec![add_layer(&format!("r{round}"))]),
        );
        let path = dir.path(&format!("round-{round}.pmv"));
        let plan = kernel
            .prepare_session_save(session, Some(path.clone()), false)
            .unwrap();
        kernel.finish_session_save(plan.run()).unwrap();
        drop(kernel);
        let saved = document(&path);
        let history = to_canonical_string(saved.get("history").unwrap());
        assert!(
            history.contains(r#""op":"insert""#),
            "round {round} keeps operations"
        );
        assert!(
            saved.get("nativeHistoryArchive").is_none(),
            "round {round} archived the kernel's own history"
        );
        previous = path;
    }
}
