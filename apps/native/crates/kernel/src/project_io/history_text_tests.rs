//! A save writes each history step from the text it made once, and the file
//! guard answers from what it judged before: the saved file must be exactly
//! the bytes the writer that rebuilt the whole history writes. Each save's
//! frozen plan runs twice, as that writer and as this one, into two files
//! that start identical; the files must be equal. Covered: a first
//! conversion of a PMV3 file, a first save, a second save, a save after one
//! more edit, after undo (redo states), and with a native-only state current
//! and then undo-reachable. Reopened, the file has every step.
use super::*;
use crate::project_io::{OpenOptions, SaveTarget, open_project};
use crate::{Command, DocumentStore, Origin};
use pm_model::json::{parse, to_canonical_string};
use std::path::{Path, PathBuf};

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let path = root.join(format!(
            "pm-history-text-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Saves `store` to `target` with the writer that rebuilt the history, and
/// the same plan with step texts into `twin` (a copy of what the target
/// held before); asserts the two files are byte-identical.
fn save_both(store: &mut DocumentStore, target: SaveTarget, twin: &Path, what: &str) -> SaveLayout {
    let plan = store
        .prepare_save(
            target,
            SaveOptions {
                reference_writer: true,
                ..SaveOptions::default()
            },
        )
        .unwrap();
    let mut ours = plan.clone();
    ours.options.reference_writer = false;
    let _ = fs::remove_file(twin);
    if plan.destination.exists() {
        fs::copy(&plan.destination, twin).unwrap();
        ours.replace_existing = true;
    }
    ours.destination = twin.to_owned();
    ours.publish = None;
    let destination = plan.destination.clone();
    let old = std::thread::spawn(move || plan.run())
        .join()
        .unwrap()
        .unwrap();
    let new = std::thread::spawn(move || ours.run())
        .join()
        .unwrap()
        .unwrap();
    assert_eq!(old.layout, new.layout, "{what}: layout");
    let (a, b) = (fs::read(&destination).unwrap(), fs::read(twin).unwrap());
    if a != b {
        let at = a
            .iter()
            .zip(&b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len()));
        let show = |v: &[u8]| {
            String::from_utf8_lossy(&v[at.saturating_sub(80)..(at + 80).min(v.len())]).into_owned()
        };
        panic!(
            "{what}: the saved files differ at byte {at} ({} vs {} bytes)\nold: {}\nnew: {}",
            a.len(),
            b.len(),
            show(&a),
            show(&b)
        );
    }
    let layout = old.layout;
    store.finish_save(old).unwrap();
    layout
}

fn open(path: &Path) -> DocumentStore {
    DocumentStore::open(open_project(path, OpenOptions::default()).unwrap())
}

/// Reopened, the file has every step the session had: the same saved
/// history, and undoing walks the same states.
fn reopen_matches(mut session: DocumentStore, path: &Path) {
    let mut reopened = open(path);
    let saved = |s: &DocumentStore| to_canonical_string(&s.history().snapshot().to_saved());
    assert_eq!(saved(&reopened), saved(&session));
    let mut steps = 0;
    loop {
        assert_eq!(
            reopened.root().map(|r| r.to_value()),
            session.root().map(|r| r.to_value()),
            "after undoing {steps} steps"
        );
        match (reopened.undo(), session.undo()) {
            (Ok(_), Ok(_)) => steps += 1,
            (Err(_), Err(_)) => break,
            other => panic!("undo differs after {steps} steps: {other:?}"),
        }
    }
}

fn project(layers: usize, future: bool) -> pm_model::json::Value {
    let layer = |i: usize| {
        format!(
            r#"{{"id":"L{i}","type":"{}","name":"Layer {i}","from":0,"dur":5,"on":true,"p":{{"opacity":{{"v":100,"kf":[]}}}},"d":{{"w":10,"h":10}},"fx":[]}}"#,
            if future && i == 0 {
                "future-layer"
            } else {
                "shape"
            }
        )
    };
    let layers: Vec<String> = (0..layers).map(layer).collect();
    parse(&format!(
        r#"{{"id":"P","compId":"C","w":640,"h":360,"fps":30,"dur":5,"assets":{{}},"layers":[{}]}}"#,
        layers.join(",")
    ))
    .unwrap()
}

fn edit(store: &mut DocumentStore, i: usize) {
    let layer = format!("L{}", i % 6);
    let command = if i.is_multiple_of(2) {
        Command::SetLayerName {
            layer_id: layer,
            name: format!("Step {i}"),
        }
    } else {
        Command::SetStaticOpacity {
            layer_id: layer,
            value: (i % 100) as f64,
        }
    };
    store.execute(command, Origin::Interface).unwrap();
}

fn replace(store: &mut DocumentStore, future: bool) {
    store
        .restore_project(project(6, future), "Replace", Origin::Interface)
        .unwrap();
}

#[test]
fn a_long_history_saves_the_same_bytes() {
    let dir = TempDir::new("long");
    let (path, twin) = (dir.0.join("project.pmv"), dir.0.join("twin.pmv"));
    let mut store = DocumentStore::new_untitled(project(6, false));
    for i in 0..150 {
        edit(&mut store, i);
    }
    let first = SaveTarget::SaveAs {
        path: path.clone(),
        replace: false,
    };
    save_both(&mut store, first, &twin, "a first save");
    let replays = |s: &DocumentStore| s.history().full_replays();
    let first_replays = replays(&store);
    save_both(&mut store, SaveTarget::InPlace, &twin, "a second save");
    edit(&mut store, 1000);
    save_both(
        &mut store,
        SaveTarget::InPlace,
        &twin,
        "a save after one more edit",
    );
    assert_eq!(
        replays(&store),
        first_replays,
        "later saves judge the history from what they judged before"
    );
    // A state only a newer reader opens becomes current, then reachable by
    // undo: the envelope requirement must come out the same.
    replace(&mut store, true);
    save_both(
        &mut store,
        SaveTarget::InPlace,
        &twin,
        "a native-only state current",
    );
    replace(&mut store, false);
    edit(&mut store, 1001);
    save_both(
        &mut store,
        SaveTarget::InPlace,
        &twin,
        "a native-only state undo-reachable",
    );
    for _ in 0..5 {
        store.undo().unwrap();
    }
    let before = replays(&store);
    save_both(
        &mut store,
        SaveTarget::InPlace,
        &twin,
        "five steps undone: redo states",
    );
    assert_eq!(replays(&store), before, "redo states were judged before");
    edit(&mut store, 1002);
    save_both(
        &mut store,
        SaveTarget::InPlace,
        &twin,
        "redo discarded by an edit",
    );
    for _ in 0..140 {
        store.undo().unwrap();
    }
    save_both(
        &mut store,
        SaveTarget::InPlace,
        &twin,
        "undone past the native-only state",
    );
    reopen_matches(store, &path);
}

#[test]
fn converting_a_pmv3_file_saves_the_same_bytes() {
    let dir = TempDir::new("pmv3");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/project_io/fixtures/main-pmv3.pmv");
    let (path, twin) = (dir.0.join("project.pmv"), dir.0.join("twin.pmv"));
    fs::copy(&fixture, &path).unwrap();
    let mut store = open(&path);
    let layout = save_both(
        &mut store,
        SaveTarget::InPlace,
        &twin,
        "the converting save",
    );
    assert_eq!(layout, SaveLayout::Compacted);
    save_both(&mut store, SaveTarget::InPlace, &twin, "a second save");
    // Its history holds only pending redo: redo one step.
    store.redo().unwrap();
    save_both(
        &mut store,
        SaveTarget::InPlace,
        &twin,
        "a save after a redo",
    );
    reopen_matches(store, &path);
}
