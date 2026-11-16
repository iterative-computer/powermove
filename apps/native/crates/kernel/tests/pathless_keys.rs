//! PM-462: v1.2.0 stores a shape path's vertex keys without ids. When a
//! project is read such a key gets an id derived from its composition,
//! layer, property path and place in its track (the same on every open). It
//! stays on the key for the session and is taken off whenever the project is
//! written, so v1.2.0 reads the file as it wrote it.

mod common;

use common::*;
use pm_kernel::commands::layers::LayerCommand;
use pm_kernel::model::{derived_key_id, is_derived_key_id};
use pm_kernel::project_io::{SaveOptions, SaveTarget};
use pm_kernel::{Command, DocumentStore, KeyframeCommand, Origin};
use pm_model::canonical::project_of;
use pm_model::container::{ReadOptions, read_container};
use pm_model::json::{Value, parse, to_canonical_string};

fn key(t: f64, v: f64) -> String {
    format!(
        r#"{{"t":{t},"v":{v},"inInterp":"linear","outInterp":"linear","inEase":{{"speed":0,"influence":16.666666666666664}},"outEase":{{"speed":0,"influence":16.666666666666664}},"autoBezier":false,"continuous":false}}"#
    )
}

fn project() -> Value {
    let xs = [key(0.0, 0.0), key(1.0, 10.0), key(2.0, 20.0)].join(",");
    let ys = [key(0.0, 5.0), key(2.0, 25.0)].join(",");
    parse(&format!(
        r##"{{"id":"P","name":"Paths","compId":"C1","compName":"Main","w":200,"h":100,"fps":30,"dur":4,"bg":"#000000","assets":{{}},
        "layers":[{{"id":"L","type":"shape","name":"Outline","from":0,"dur":4,"p":{{"opacity":{{"v":100,"kf":[{{"t":0,"v":100,"i":"op"}}],"expr":null}}}},
          "d":{{"shape":"path","color":"#ffffff","paths":[{{"id":"outline","p":{{"closed":{{"v":true,"kf":[],"expr":null}}}},
            "vertices":[{{"id":"v0","p":{{"x":{{"v":0,"kf":[{xs}],"expr":null}},"y":{{"v":5,"kf":[{ys}],"expr":null}}}}}}]}}]}}}}]}}"##
    ))
    .unwrap()
}

const X: &str = "d.paths.outline.vertices.v0.p.x";

fn id(index: usize) -> String {
    derived_key_id("C1", "L", X, index)
}

/// The keys of the vertex's `axis` channel in the store.
fn keys(store: &DocumentStore, axis: &str) -> Vec<Value> {
    let root = store.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, "L").unwrap();
    let layer = pm_kernel::model::layer_at(root, &loc).unwrap().to_value();
    let path = format!("d.paths.outline.vertices.v0.p.{axis}");
    lookup(&layer, &path)
        .get("kf")
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

/// `path` (as a derived id names it: records by id) inside `v`.
fn lookup<'a>(v: &'a Value, path: &str) -> &'a Value {
    path.split('.').fold(v, |at, step| match at {
        Value::Array(items) => items
            .iter()
            .find(|i| i.get("id").and_then(Value::as_str) == Some(step))
            .or_else(|| step.parse::<usize>().ok().and_then(|n| items.get(n)))
            .unwrap(),
        _ => at.get(step).unwrap(),
    })
}

fn lookup_mut<'a>(v: &'a mut Value, path: &str) -> &'a mut Value {
    path.split('.').fold(v, |at, step| match at {
        Value::Array(items) => {
            let n = items
                .iter()
                .position(|i| i.get("id").and_then(Value::as_str) == Some(step))
                .or_else(|| step.parse().ok())
                .unwrap();
            &mut items[n]
        }
        Value::Object(m) => m.get_mut(step).unwrap(),
        _ => panic!("{step}"),
    })
}

fn ids(keys: &[Value]) -> Vec<String> {
    keys.iter()
        .map(|k| k.get("i").and_then(Value::as_str).unwrap().to_owned())
        .collect()
}

fn times(keys: &[Value]) -> Vec<f64> {
    keys.iter()
        .map(|k| k.get("t").unwrap().as_f64().unwrap())
        .collect()
}

fn run(store: &mut DocumentStore, command: KeyframeCommand) {
    store
        .execute(Command::Keyframe(command), Origin::Interface)
        .unwrap()
        .expect("changed");
}

/// The stored project of a file.
fn stored(path: &std::path::Path) -> Value {
    let file = std::fs::File::open(path).unwrap();
    project_of(
        &read_container(&file, ReadOptions::default())
            .unwrap()
            .document,
    )
    .clone()
}

/// Every key under the open composition's stored layers (v1's status line
/// counts these) and those stored without an id.
fn counts(project: &Value) -> (usize, usize) {
    fn walk(v: &Value, out: &mut (usize, usize)) {
        match v {
            Value::Object(m) => {
                for (k, child) in m.iter() {
                    if k.to_string_lossy() == "kf"
                        && let Some(keys) = child.as_array()
                    {
                        out.0 += keys.len();
                        out.1 += keys.iter().filter(|k| k.get("i").is_none()).count();
                    }
                    walk(child, out);
                }
            }
            Value::Array(a) => a.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
    let mut out = (0, 0);
    if let Some(layers) = project.get("layers") {
        walk(layers, &mut out);
    }
    out
}

/// The ids of the open composition's keys in a store.
fn key_ids(store: &DocumentStore) -> std::collections::HashSet<String> {
    let root = store.root().unwrap();
    let comp =
        pm_kernel::model::comp_node(root, pm_kernel::model::open_comp_id(root).as_deref()).unwrap();
    let mut ids = std::collections::HashSet::new();
    pm_kernel::model::keyframe_ids(comp, &mut ids);
    ids
}

#[test]
fn keys_stored_without_ids_are_named_on_read_and_keep_their_names() {
    let mut store = DocumentStore::new_untitled(project());
    assert_eq!(ids(&keys(&store, "x")), [id(0), id(1), id(2)]);
    assert_eq!(
        ids(&keys(&store, "y")),
        [
            derived_key_id("C1", "L", "d.paths.outline.vertices.v0.p.y", 0),
            derived_key_id("C1", "L", "d.paths.outline.vertices.v0.p.y", 1)
        ]
    );
    assert!(is_derived_key_id(&id(1)) && !is_derived_key_id("op") && !is_derived_key_id("~x"));
    let before = to_canonical_string(&store.root().unwrap().to_value());

    // Hold, by its id.
    run(
        &mut store,
        KeyframeCommand::SetHold {
            key_ids: vec![id(1)],
            hold: true,
        },
    );
    assert_eq!(
        keys(&store, "x")[1]
            .get("outInterp")
            .and_then(Value::as_str),
        Some("hold")
    );
    store.undo().unwrap();
    assert_eq!(
        to_canonical_string(&store.root().unwrap().to_value()),
        before
    );

    // A move past its neighbour: the key keeps its id, so the selection and
    // the commands after it still name it.
    store.select_keys([id(1)]);
    run(
        &mut store,
        KeyframeCommand::Move {
            key_ids: vec![id(1)],
            delta: 1.5,
        },
    );
    let x = keys(&store, "x");
    assert_eq!(times(&x), [0.0, 2.0, 2.5]);
    assert_eq!(ids(&x), [id(0), id(2), id(1)]);
    assert_eq!(store.selection().keys(), [id(1)]);
    run(
        &mut store,
        KeyframeCommand::Edit {
            key_id: id(1),
            time: None,
            value: Some(Value::from(12.0)),
        },
    );
    assert_eq!(
        keys(&store, "x")[2].get("v").and_then(Value::as_f64),
        Some(12.0)
    );
    store.undo().unwrap();
    store.undo().unwrap();
    assert_eq!(store.selection().keys(), [id(1)]);
    assert_eq!(
        to_canonical_string(&store.root().unwrap().to_value()),
        before
    );

    // Delete takes the key and its selection; the neighbours keep theirs.
    run(
        &mut store,
        KeyframeCommand::Delete {
            key_ids: vec![id(1)],
        },
    );
    assert_eq!(ids(&keys(&store, "x")), [id(0), id(2)]);
    assert!(store.selection().keys().is_empty());
}

#[test]
fn a_saved_file_keeps_keys_without_ids_and_a_reopen_names_them_the_same() {
    let dir = TempDir::new("pathless-keys");
    let path = dir.path("paths.pmv");
    let mut store = DocumentStore::new_untitled(project());
    run(
        &mut store,
        KeyframeCommand::Move {
            key_ids: vec![id(1)],
            delta: 1.5,
        },
    );
    save_as(&mut store, &path).unwrap();
    assert_eq!(
        counts(&stored(&path)),
        (6, 5),
        "only the opacity key is stored with an id"
    );
    // Read again, the keys are named by where they now stand.
    let reopened = open_store(&path);
    assert_eq!(ids(&keys(&reopened, "x")), [id(0), id(1), id(2)]);
    assert_eq!(times(&keys(&reopened, "x")), [0.0, 2.0, 2.5]);
    // The store that saved keeps its names.
    assert_eq!(ids(&keys(&store, "x")), [id(0), id(2), id(1)]);
    drop(reopened);
    edit_one_and_compare(&path, &id(1));
}

#[test]
fn a_duplicated_layer_gets_its_own_key_ids() {
    let mut store = DocumentStore::new_untitled(project());
    store
        .execute(
            Command::Layer(LayerCommand::Duplicate {
                layer_ids: vec!["L".into()],
                new_ids: vec!["L2".into()],
            }),
            Origin::Interface,
        )
        .unwrap();
    let all = key_ids(&store);
    // Three x keys, two y keys and the opacity key on each layer.
    assert_eq!(all.len(), 12, "{all:?}");
}

/// History saved with a file names keys by where they stood in that
/// session. Read again, a key an Undo brings back may carry a derived id
/// another key now has: it is named afresh, and no two keys share an id.
#[test]
fn an_undo_after_a_reopen_never_leaves_two_keys_one_id() {
    let dir = TempDir::new("pathless-keys-undo");
    let path = dir.path("paths.pmv");
    let mut store = DocumentStore::new_untitled(project());
    run(
        &mut store,
        KeyframeCommand::Delete {
            key_ids: vec![id(1)],
        },
    );
    save_as(&mut store, &path).unwrap();
    drop(store);
    let mut reopened = open_store(&path);
    assert_eq!(ids(&keys(&reopened, "x")), [id(0), id(1)]);
    reopened.undo().expect("the delete is in the saved history");
    let x = keys(&reopened, "x");
    assert_eq!(times(&x), [0.0, 1.0, 2.0]);
    let named = ids(&x);
    let unique: std::collections::HashSet<_> = named.iter().collect();
    assert_eq!(unique.len(), 3, "{named:?}");
    assert!(named.iter().all(|i| is_derived_key_id(i)), "{named:?}");
    // And the file the undo leads to is stored without ids.
    let again = dir.path("again.pmv");
    save_as(&mut reopened, &again).unwrap();
    assert_eq!(counts(&stored(&again)), (6, 5));
}

/// Opens `path`, edits the key `id` (stored without an id) to a new value,
/// saves in place, reopens, and checks the stored project is the same byte
/// for byte except that key's value.
fn edit_one_and_compare(path: &std::path::Path, id: &str) {
    let before = stored(path);
    let mut store = open_store(path);
    assert!(key_ids(&store).contains(id), "{id}");
    let edited = 123.25;
    run(
        &mut store,
        KeyframeCommand::Edit {
            key_id: id.to_owned(),
            time: None,
            value: Some(Value::from(edited)),
        },
    );
    common::save(&mut store, SaveTarget::InPlace, SaveOptions::default()).unwrap();
    drop(store);
    let mut after = stored(path);
    assert!(key_ids(&open_store(path)).contains(id), "reopened: {id}");
    // The edited key, then everything else.
    let rest = id.strip_prefix('~').unwrap();
    let (head, index) = rest.rsplit_once('~').unwrap();
    let (_comp, rest) = head.split_once('~').unwrap();
    let (layer, property) = rest.split_once('~').unwrap();
    let index: usize = index.parse().unwrap();
    let old = stored_key(&mut before.clone(), layer, property, index)
        .get("v")
        .cloned()
        .unwrap();
    let key = stored_key(&mut after, layer, property, index);
    assert_eq!(key.get("v").and_then(Value::as_f64), Some(edited));
    assert!(key.get("i").is_none(), "stored without an id");
    key.as_object_mut().unwrap().insert("v", old);
    // A save already gives a v1 video asset V2's reading of it
    // (`videoColor: "electron"`, see `project_io.rs`); v1.2.0 ignores the
    // field. Nothing else may differ.
    let mut differ = Vec::new();
    differences(&before, &after, &mut String::new(), &mut differ);
    differ.retain(|d| !(d.starts_with(".assets.") && d.ends_with(".videoColor added")));
    assert!(
        differ.is_empty(),
        "changed beyond the edited key: {differ:?}"
    );
}

/// The stored key `index` of `property` on layer `layer`, in any
/// composition.
fn stored_key<'a>(
    project: &'a mut Value,
    layer: &str,
    property: &str,
    index: usize,
) -> &'a mut Value {
    let m = project.as_object_mut().unwrap();
    let mut lists: Vec<&mut Value> = Vec::new();
    for (k, v) in m.iter_mut() {
        match k.to_string_lossy().as_str() {
            "layers" => lists.push(v),
            "comps" => {
                for (_, comp) in v.as_object_mut().into_iter().flat_map(|c| c.iter_mut()) {
                    if let Some(l) = comp.as_object_mut().and_then(|c| c.get_mut("layers")) {
                        lists.push(l);
                    }
                }
            }
            _ => {}
        }
    }
    let layer = lists
        .into_iter()
        .flat_map(|l| l.as_array_mut().unwrap().iter_mut())
        .find(|l| l.get("id").and_then(Value::as_str) == Some(layer))
        .unwrap();
    let kf = lookup_mut(layer, property)
        .as_object_mut()
        .unwrap()
        .get_mut("kf")
        .unwrap();
    &mut kf.as_array_mut().unwrap()[index]
}

/// The paths where `a` and `b` differ (no values: a real project's data
/// stays out of the output), at most 40.
fn differences(a: &Value, b: &Value, at: &mut String, out: &mut Vec<String>) {
    if out.len() >= 40 {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x.iter() {
                let len = at.len();
                at.push('.');
                at.push_str(&k.to_string_lossy());
                match y.get_key(k) {
                    Some(w) => differences(v, w, at, out),
                    None => out.push(format!("{at} removed")),
                }
                at.truncate(len);
            }
            for (k, _) in y.iter() {
                if x.get_key(k).is_none() {
                    out.push(format!("{at}.{} added", k.to_string_lossy()));
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (v, w)) in x.iter().zip(y).enumerate() {
                let len = at.len();
                at.push_str(&format!("[{i}]"));
                differences(v, w, at, out);
                at.truncate(len);
            }
        }
        _ if to_canonical_string(a) != to_canonical_string(b) => out.push(at.clone()),
        _ => {}
    }
}

/// The same on a real project, when `PM_PATHLESS_KEYS_PROJECT` names one
/// (a v1.2.0 file with keys stored without ids; it is copied, never
/// written): every key v1 counts has an id, and an edit of one saves
/// everything else byte for byte.
#[test]
fn a_real_project_with_keys_without_ids() {
    let Some(source) = std::env::var_os("PM_PATHLESS_KEYS_PROJECT") else {
        eprintln!("PM_PATHLESS_KEYS_PROJECT unset: skipped");
        return;
    };
    let dir = TempDir::new("pathless-keys-real");
    let path = dir.path("project.pmv");
    std::fs::copy(&source, &path).unwrap();
    let (count, without) = counts(&stored(&path));
    assert!(without > 0, "the project has no keys stored without ids");
    let store = open_store(&path);
    let ids = key_ids(&store);
    let mut derived: Vec<&String> = ids.iter().filter(|i| is_derived_key_id(i)).collect();
    derived.sort();
    eprintln!("{count} keys as v1 counts them; {without} stored without ids");
    assert_eq!(
        derived.len(),
        without,
        "every key stored without an id is named"
    );
    let first = derived[0].clone();
    drop(store);
    edit_one_and_compare(&path, &first);
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A session that moved keys stored without ids saves (a reopen names them
/// otherwise) while its first checkpoint is held behind a slow disk, is
/// edited while the save runs and after it publishes, then the app is gone.
/// Recovery lands every edit on the key it was made to and leaves no two
/// keys one id: the save waited for a checkpoint of this session's ids, so
/// recovery never replays the session's lines onto a reopen's ids.
#[test]
fn recovery_after_a_save_of_moved_keys_lands_every_edit_on_its_key() {
    use pm_kernel::journal::JournalConfig;
    use pm_kernel::session::{
        RecoveryDecision, RecoveryResult, SessionSource, ShellCall, ShellReply,
    };
    use pm_kernel::{Call, Caller, Kernel, KernelActor, KernelConfig, Request, WriteBody};
    let dir = TempDir::new("pathless-keys-recovery");
    let file = dir.path("project.pmv");
    save_as(&mut DocumentStore::new_untitled(project()), &file).unwrap();
    use std::sync::{Arc, Condvar, Mutex};
    let hold = Arc::new((Mutex::new(true), Condvar::new()));
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.path("journals")),
        backup_root: Some(dir.path("backups")),
        journal: JournalConfig {
            checkpoint_revisions: 1000,
            hold_checkpoints: Some(hold.clone()),
            ..Default::default()
        },
        ..Default::default()
    });
    // The disk catches up later; a save that needs a checkpoint waits for it.
    let release = {
        let hold = hold.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            *hold.0.lock().unwrap() = false;
            hold.1.notify_all();
        })
    };
    let k = actor.handle();
    let session = match k
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    };
    let edit = |command: KeyframeCommand| {
        k.call(
            "P",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: None,
                    body: WriteBody::Commands(vec![Command::Keyframe(command)]),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    };
    // The middle key past the last: the session's ids now differ from a
    // reopen's ([0, 2, 1] by place).
    edit(KeyframeCommand::Move {
        key_ids: vec![id(1)],
        delta: 1.5,
    });
    let plan = k.prepare_save(session, None).wait().unwrap().unwrap();
    // While the save runs, the first key past the one now second: another
    // reorder, journalled after the save point under this session's ids.
    edit(KeyframeCommand::Move {
        key_ids: vec![id(0)],
        delta: 2.2,
    });
    let completion = plan.run();
    edit(KeyframeCommand::Edit {
        key_id: id(1),
        time: None,
        value: Some(Value::from(77.0)),
    });
    // Completing the save may wait on the held checkpoint; the app is gone
    // before it does (the journals synced from outside, as Quit Now does).
    let finishing = k.finish_save(completion);
    k.sync_journals().unwrap();
    let exited = dir.path("after-exit");
    copy_tree(&dir.path("journals"), &exited.join("journals"));
    std::fs::copy(&file, exited.join("project.pmv")).unwrap();
    release.join().unwrap();
    finishing.wait().unwrap().unwrap();
    drop(actor);

    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(exited.join("journals")),
        ..Default::default()
    });
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "P".into(),
            source: SessionSource::File(exited.join("project.pmv")),
            electron_profile: None,
        })
        .unwrap();
    let ShellReply::RecoveryRequired { ticket, recovery } = reply else {
        panic!("{reply:?}");
    };
    assert!(
        matches!(recovery, RecoveryResult::Recovered { .. }),
        "{recovery:?}"
    );
    k.shell(ShellCall::DecideRecovery {
        ticket,
        decision: RecoveryDecision::Recover,
    })
    .unwrap();
    let store = k.project("P").unwrap().store();
    let x = keys(store, "x");
    // Stored order: the key once second (20 at 2 s), the first (0 at 2.2 s),
    // the one once middle (edited to 77 at 2.5 s).
    assert_eq!(times(&x), [2.0, 2.2, 2.5]);
    let values: Vec<f64> = x.iter().map(|k| k.get("v").unwrap().as_f64().unwrap()).collect();
    assert_eq!(values, [20.0, 0.0, 77.0], "every edit found its key");
    let named = ids(&x);
    let unique: std::collections::HashSet<_> = named.iter().collect();
    assert_eq!(unique.len(), 3, "{named:?}");
}
