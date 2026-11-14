//! A save writes the undo history in the packed history form, from text
//! each step made once. Each save's frozen plan runs three times into files
//! that start identical: as `pack(to_saved())` rebuilt whole (the new
//! writer's bytes must equal it exactly), as this writer, and as the writer
//! before the packed form (the unpacked history). At every save the file
//! guard's requirement must be the same for the packed and unpacked files,
//! and unpacking the written member (the reader every V2 build runs first,
//! unchanged since the first one) must give back the session's history. At
//! the end both files reopen to the same history, and undo and redo walk
//! the same states. Covered: a PMV3 conversion (history kept as read), a
//! long history with a native-only state current and undo-reachable, undo
//! and redo, steps a realm runs, and two saves of one state.
use super::*;
use crate::history::{ExternalRef, unpack};
use crate::project_io::{OpenOptions, SaveTarget, open_project};
use crate::{Command, DocumentStore, Origin};
use pm_model::container::{ReadOptions, read_container};
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
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn document(path: &Path) -> Value {
    read_container(&File::open(path).unwrap(), ReadOptions::default())
        .unwrap()
        .document
}

fn first_difference(a: &[u8], b: &[u8]) -> String {
    let at = a
        .iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let show = |v: &[u8]| {
        String::from_utf8_lossy(&v[at.saturating_sub(80)..(at + 80).min(v.len())]).into_owned()
    };
    format!(
        "at byte {at} ({} vs {} bytes)\nreference: {}\nwritten:   {}",
        a.len(),
        b.len(),
        show(a),
        show(b)
    )
}

/// The three files a project's saves go to: `packed` (the store's own file,
/// written by the rebuilt-whole packed reference), `twin` (this writer) and
/// `plain` (the unpacked writer).
struct Files {
    packed: PathBuf,
    twin: PathBuf,
    plain: PathBuf,
}
impl Files {
    fn new(dir: &TempDir) -> Self {
        Self {
            packed: dir.path("packed.pmv"),
            twin: dir.path("twin.pmv"),
            plain: dir.path("plain.pmv"),
        }
    }
}

fn run(plan: SavePlan) -> SaveReceipt {
    std::thread::spawn(move || plan.run())
        .join()
        .unwrap()
        .unwrap()
}

/// One save, three ways; see the module comment.
fn save_three(
    store: &mut DocumentStore,
    target: SaveTarget,
    files: &Files,
    what: &str,
) -> SaveLayout {
    let plan = store
        .prepare_save(
            target,
            SaveOptions {
                reference_writer: Some(ReferenceWriter::Packed),
                ..SaveOptions::default()
            },
        )
        .unwrap();
    let history = match &plan.history {
        HistoryToWrite::Session(snapshot) => snapshot.to_saved(),
        HistoryToWrite::Raw(value) => value.clone(),
    };
    let other = |twin: &Path, writer: Option<ReferenceWriter>| {
        let _ = fs::remove_file(twin);
        let mut other = plan.clone();
        if plan.destination.exists() {
            fs::copy(&plan.destination, twin).unwrap();
            other.replace_existing = true;
        }
        other.destination = twin.to_owned();
        other.publish = None;
        other.options.reference_writer = writer;
        other
    };
    let ours = other(&files.twin, None);
    let plain = other(&files.plain, Some(ReferenceWriter::Unpacked));
    let destination = plan.destination.clone();
    let reference = run(plan);
    let written = run(ours);
    run(plain);
    assert_eq!(reference.layout, written.layout, "{what}: layout");
    let (a, b) = (
        fs::read(&destination).unwrap(),
        fs::read(&files.twin).unwrap(),
    );
    assert!(
        a == b,
        "{what}: the written file differs from pack(to_saved()) {}",
        first_difference(&a, &b)
    );
    // The packed and unpacked files say the same history, with the same
    // requirement.
    let (packed, unpacked) = (document(&destination), document(&files.plain));
    assert_eq!(
        packed.get(file_guard::MARKER),
        unpacked.get(file_guard::MARKER),
        "{what}: the file guard judges the packed history as the unpacked one"
    );
    let member = packed.get("history").expect("a history member");
    let read_back = unpack(member).expect("an unpackable history");
    // A history kept as read may itself be packed already: compare the
    // histories both say.
    let history = unpack(&history).expect("an unpackable history");
    assert_eq!(
        to_canonical_string(&read_back),
        to_canonical_string(&history),
        "{what}: unpacking the written member gives back the history"
    );
    assert_eq!(
        to_canonical_string(&unpack(unpacked.get("history").unwrap()).unwrap()),
        to_canonical_string(&history)
    );
    // For the old app's own reader (tests/v1_reader/read_history.ts):
    // PM_HISTORY_WITNESS=<dir> keeps each saved member and its history.
    if let Some(dir) = std::env::var_os("PM_HISTORY_WITNESS") {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir).unwrap();
        let name: String = what
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let tag = format!(
            "{}-{name}",
            files
                .packed
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
        );
        fs::write(
            dir.join(format!("{tag}.packed.json")),
            to_canonical_string(member),
        )
        .unwrap();
        fs::write(
            dir.join(format!("{tag}.plain.json")),
            to_canonical_string(&history),
        )
        .unwrap();
    }
    let layout = reference.layout;
    store.finish_save(reference).unwrap();
    layout
}

fn open(path: &Path) -> DocumentStore {
    DocumentStore::open(open_project(path, OpenOptions::default()).unwrap())
}

/// Both files reopen to the same history, and every state undo and redo
/// reach is the same: all the way back, then all the way forward.
fn reopened_alike(files: &Files) {
    let mut packed = open(&files.packed);
    let mut plain = open(&files.plain);
    let saved = |s: &DocumentStore| to_canonical_string(&s.history().snapshot().to_saved());
    assert_eq!(saved(&packed), saved(&plain));
    let root = |s: &DocumentStore| s.root().map(|r| r.to_value());
    assert_eq!(root(&packed), root(&plain));
    let mut back = 0;
    loop {
        match (packed.undo(), plain.undo()) {
            (Ok(_), Ok(_)) => back += 1,
            (Err(_), Err(_)) => break,
            other => panic!("undo differs after {back} steps: {other:?}"),
        }
        assert_eq!(root(&packed), root(&plain), "undoing step {back}");
    }
    let mut forward = 0;
    loop {
        match (packed.redo(), plain.redo()) {
            (Ok(_), Ok(_)) => forward += 1,
            (Err(_), Err(_)) => break,
            other => panic!("redo differs after {forward} steps: {other:?}"),
        }
        assert_eq!(root(&packed), root(&plain), "redoing step {forward}");
    }
    assert!(back + forward > 0, "the reopened history has steps to walk");
}

fn project(layers: usize, future: bool) -> Value {
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

/// A new layer: a step that saves the composition's whole layers array,
/// the kind the packed form stores as references.
fn add_layer(store: &mut DocumentStore, id: &str) {
    let add = crate::commands::layers::AddLayer {
        id: id.into(),
        layer_type: "solid".into(),
        comp: None,
        name: None,
        from: None,
        duration: None,
        content: Default::default(),
        properties: Default::default(),
        color: None,
        index: None,
        parent: None,
        blend: None,
        motion_blur: None,
        visible: None,
        shy: None,
        collapsed: None,
        time: 0.,
    };
    store
        .execute(
            crate::LayerCommand::Add(Box::new(add)).into(),
            Origin::Interface,
        )
        .unwrap();
}

/// A whole-project replace (with a layer type only a newer reader opens).
fn replace(store: &mut DocumentStore, future: bool) {
    store
        .restore_project(project(6, future), "Replace", Origin::Interface)
        .unwrap();
}

#[test]
fn a_long_history_saves_packed() {
    let dir = TempDir::new("long");
    let files = Files::new(&dir);
    let mut store = DocumentStore::new_untitled(project(6, false));
    for i in 0..150 {
        edit(&mut store, i);
        if i % 30 == 0 {
            add_layer(&mut store, &format!("A{i}"));
        }
    }
    let first = SaveTarget::SaveAs {
        path: files.packed.clone(),
        replace: false,
    };
    save_three(&mut store, first, &files, "a first save");
    assert_eq!(
        document(&files.packed)
            .get("history")
            .and_then(|h| h.get("format"))
            .and_then(Value::as_str),
        Some("powermove-history-dictionary"),
        "steps with whole layer arrays are written packed"
    );
    let replays = |s: &DocumentStore| s.history().full_replays();
    let first_replays = replays(&store);
    save_three(&mut store, SaveTarget::InPlace, &files, "a second save");
    edit(&mut store, 1000);
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "a save after one more edit",
    );
    assert_eq!(
        replays(&store),
        first_replays,
        "later saves judge from what they judged"
    );
    // A state only a newer reader opens becomes current, then undo-reachable.
    replace(&mut store, true);
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "a native-only state current",
    );
    replace(&mut store, false);
    edit(&mut store, 1001);
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "a native-only state undo-reachable",
    );
    for _ in 0..5 {
        store.undo().unwrap();
    }
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "five steps undone: redo states",
    );
    edit(&mut store, 1002);
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "redo discarded by an edit",
    );
    for _ in 0..140 {
        store.undo().unwrap();
    }
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "undone past the native-only state",
    );
    drop(store);
    reopened_alike(&files);
}

#[test]
fn a_history_with_steps_a_realm_runs_saves_packed() {
    let dir = TempDir::new("realm");
    let files = Files::new(&dir);
    let mut store = DocumentStore::new_untitled(project(6, false));
    for i in 0..40 {
        edit(&mut store, i);
        if i % 10 == 3 {
            store.kernel_push_external(
                format!("Realm step {i}"),
                Origin::Interface,
                ExternalRef {
                    realm: 1,
                    generation: 1,
                    handle: i as u64,
                    cleanup: false,
                },
                512,
            );
        }
        if i % 10 == 6 {
            add_layer(&mut store, &format!("A{i}"));
        }
    }
    let first = SaveTarget::SaveAs {
        path: files.packed.clone(),
        replace: false,
    };
    save_three(&mut store, first, &files, "with realm steps on the stack");
    for _ in 0..4 {
        store.undo().unwrap();
    }
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "realm steps undone",
    );
    drop(store);
    reopened_alike(&files);
}

#[test]
fn converting_a_pmv3_file_saves_packed() {
    let dir = TempDir::new("pmv3");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/project_io/fixtures/main-pmv3.pmv");
    let files = Files::new(&dir);
    fs::copy(&fixture, &files.packed).unwrap();
    let mut store = open(&files.packed);
    let layout = save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "the converting save",
    );
    assert_eq!(layout, SaveLayout::Compacted);
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "a second save, history as read",
    );
    // Its history holds only pending redo: redo one step, now the session's.
    store.redo().unwrap();
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "a save after a redo",
    );
    drop(store);
    reopened_alike(&files);
}

/// The same project state saved twice gives the same bytes.
#[test]
fn one_state_saves_the_same_bytes_twice() {
    let dir = TempDir::new("twice");
    let mut store = DocumentStore::new_untitled(project(6, false));
    for i in 0..60 {
        edit(&mut store, i);
        if i % 20 == 0 {
            add_layer(&mut store, &format!("A{i}"));
        }
    }
    let save_as = |store: &mut DocumentStore, path: &Path| {
        let plan = store
            .prepare_save(
                SaveTarget::SaveAs {
                    path: path.to_owned(),
                    replace: false,
                },
                SaveOptions::default(),
            )
            .unwrap();
        let receipt = run(plan);
        store.finish_save(receipt).unwrap();
    };
    let (a, b) = (dir.path("a.pmv"), dir.path("b.pmv"));
    save_as(&mut store, &a);
    save_as(&mut store, &b);
    assert!(fs::read(&a).unwrap() == fs::read(&b).unwrap());
    let history = document(&a);
    assert_eq!(
        history
            .get("history")
            .and_then(|h| h.get("format"))
            .and_then(Value::as_str),
        Some("powermove-history-dictionary"),
        "a history with whole layer arrays is written packed"
    );
}

/// A write no history entry covers can undo a recorded change; undo and redo
/// then reach states whose content the state token alone does not name. The
/// file guard must judge them by replay, not by what it judged under the
/// same token before (review w31 F1). A project with a recorded switch to
/// linear blending (only a newer reader opens it), a rename, then an
/// outside write switching blending off again; saves between undos. Undone
/// to the start, redo would restore linear blending: the requirement must
/// be written, as the full replay writes it.
#[test]
fn an_outside_write_over_a_recorded_change_keeps_the_requirement() {
    let dir = TempDir::new("outside");
    let files = Files::new(&dir);
    let blending = |on: bool| {
        let mut p = project(1, false);
        p.as_object_mut().unwrap().insert(
            "color",
            parse(&format!(r#"{{"linearBlending":{on}}}"#)).unwrap(),
        );
        p
    };
    let mut store = DocumentStore::new_untitled(blending(false));
    // 1. Recorded: linear blending on. 2. Recorded: a rename.
    store
        .restore_project(blending(true), "Linear blending", Origin::Interface)
        .unwrap();
    edit(&mut store, 0);
    // 3. Outside any entry: linear blending off.
    let ops = pm_model::ops::ops_from_value(
        &parse(r#"[{"path":["color","linearBlending"],"exists":true,"value":false}]"#).unwrap(),
    )
    .unwrap();
    let written = store
        .kernel_apply(
            &crate::WriteBody::Ops(ops),
            Origin::Interface,
            crate::document::ChangeCause::Write {
                label: "Direct edit".into(),
                origin: Origin::Interface,
                entry: None,
            },
        )
        .unwrap();
    assert!(written.is_some());
    store.kernel_outside_write();
    let first = SaveTarget::SaveAs {
        path: files.packed.clone(),
        replace: false,
    };
    // 4. Save; 5. undo the rename, save; 6. undo the switch, save.
    save_three(&mut store, first, &files, "after the outside write");
    store.undo().unwrap();
    save_three(&mut store, SaveTarget::InPlace, &files, "the rename undone");
    store.undo().unwrap();
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "the switch undone: redo restores it",
    );
    let document = pm_model::container::read_container(
        &File::open(&files.packed).unwrap(),
        pm_model::container::ReadOptions::default(),
    )
    .unwrap()
    .document;
    assert_eq!(
        document.get("requiresEngine").and_then(Value::as_str),
        Some("native"),
        "redo reaches linear blending: the file needs the native reader"
    );
}

/// The file guard's kept verdicts stay within the history's reachable
/// states and the current one, over more edits than the history keeps,
/// whether or not the project itself needs the native reader (review w31
/// F4); clearing the history clears them.
#[test]
fn the_guard_keeps_no_more_verdicts_than_the_history_reaches() {
    for future in [false, true] {
        let mut store = DocumentStore::new_untitled(project(6, future));
        for i in 0..1_100 {
            edit(&mut store, i);
            let snapshot = store.history().snapshot();
            let _ = file_guard::save_extra(
                store.root(),
                file_guard::GuardHistory::Session {
                    snapshot: &snapshot,
                    state_token: store.kernel_state_token(),
                    outside_writes: 0,
                },
                &pm_model::Map::new(),
            );
            let bound = store.history().records().len() + 1;
            assert!(
                store.history().guard_verdicts() <= bound,
                "{} verdicts kept for {bound} states after {i} edits (native project: {future})",
                store.history().guard_verdicts()
            );
        }
        assert!(store.history().records().len() <= crate::history::MAX_ENTRIES);
        store.kernel_clear();
        assert_eq!(store.history().guard_verdicts(), 0);
    }
}

/// A recorded write that only sets an otherwise absent edit log to `[]`:
/// its patch is a record array with no item, so `pack` keeps the history
/// plain and so must the writer (review w31 F3).
#[test]
fn a_history_whose_only_record_array_is_empty_stays_plain() {
    let dir = TempDir::new("empty-records");
    let files = Files::new(&dir);
    let mut store = DocumentStore::new_untitled(project(6, false));
    edit(&mut store, 0);
    let before = store.current().clone();
    let ops = pm_model::ops::ops_from_value(
        &parse(r#"[{"path":["edits"],"exists":true,"value":[]}]"#).unwrap(),
    )
    .unwrap();
    let applied = store
        .kernel_apply(
            &crate::WriteBody::Ops(ops),
            Origin::Interface,
            crate::document::ChangeCause::Write {
                label: "Clear edits".into(),
                origin: Origin::Interface,
                entry: None,
            },
        )
        .unwrap()
        .expect("a change");
    store.kernel_record(
        "Clear edits".into(),
        Origin::Interface,
        applied.forward,
        applied.inverse,
        &before,
        applied.scopes,
    );
    let first = SaveTarget::SaveAs {
        path: files.packed.clone(),
        replace: false,
    };
    save_three(&mut store, first, &files, "only an empty record array");
    assert!(
        document(&files.packed)
            .get("history")
            .and_then(|h| h.get("format"))
            .is_none(),
        "no record: the history stays plain"
    );
    drop(store);
    reopened_alike(&files);
}

/// A project showing one embedded image, `m`.
fn with_media() -> DocumentStore {
    let mut p = project(6, false);
    let root = p.as_object_mut().unwrap();
    root.insert(
        "assets",
        parse(r#"{"m":{"id":"m","name":"Picture","kind":"image","type":"image/png"}}"#).unwrap(),
    );
    let layers = root.get_mut("layers").unwrap().as_array_mut().unwrap();
    layers.push(
        parse(r#"{"id":"pic","type":"image","name":"Picture","from":0,"dur":5,"on":true,"p":{"opacity":{"v":100,"kf":[]}},"d":{"asset":"m"},"fx":[]}"#)
            .unwrap(),
    );
    let mut store = DocumentStore::new_untitled(p);
    store.media_mut().insert(MediaEntry {
        id: "m".into(),
        media_type: "image/png".into(),
        revision: "r1".into(),
        source: MediaSource::Memory(std::sync::Arc::from(&b"synthetic picture bytes"[..])),
    });
    store
}

fn remove_media(store: &mut DocumentStore) {
    store
        .execute(
            crate::MediaCommand::RemoveAsset {
                asset_id: "m".into(),
            }
            .into(),
            Origin::Interface,
        )
        .unwrap();
}

/// Media only an undo step refers to (a removed picture) is restored by the
/// old app only when the history it walks is plain, before it imports the
/// history: such a file's history is written plain, and its bytes are kept
/// (review w31 F2).
#[test]
fn history_only_media_keeps_the_history_plain() {
    let dir = TempDir::new("history-media");
    let files = Files::new(&dir);
    let mut store = with_media();
    for i in 0..10 {
        edit(&mut store, i);
    }
    add_layer(&mut store, "extra");
    remove_media(&mut store);
    let first = SaveTarget::SaveAs {
        path: files.packed.clone(),
        replace: false,
    };
    save_three(&mut store, first, &files, "the picture removed");
    let saved = document(&files.packed);
    assert!(
        saved.get("history").and_then(|h| h.get("format")).is_none(),
        "the history stays plain while it alone refers to embedded media"
    );
    let container =
        read_container(&File::open(&files.packed).unwrap(), ReadOptions::default()).unwrap();
    assert!(
        container
            .media
            .iter()
            .chain(&container.archived_media)
            .any(|m| m.id.as_str() == Some("m")),
        "the picture's bytes stay in the file"
    );
    // Undone: the project lists the picture again, the history is packed.
    store.undo().unwrap();
    save_three(
        &mut store,
        SaveTarget::InPlace,
        &files,
        "the removal undone",
    );
    assert_eq!(
        document(&files.packed)
            .get("history")
            .and_then(|h| h.get("format"))
            .and_then(Value::as_str),
        Some("powermove-history-dictionary")
    );
    drop(store);
    reopened_alike(&files);
}

/// The old app's whole file-open path (`tests/v1_reader/read_file.ts`, run
/// on exported v1.2.0 and v1.3.2 sources listed in `PM_V1_SRC`, separated by
/// `:`): a fresh media store, open, Undo, Save As, reopen. The picture
/// survives in the plain case (removed media) and the packed case (media the
/// project still lists); the same removal written packed regardless loses
/// it, which the witness must catch.
#[test]
fn the_old_app_keeps_the_media_through_open_undo_and_save_as() {
    let Some(sources) = std::env::var_os("PM_V1_SRC") else {
        eprintln!("PM_V1_SRC not set: the old app's open path was not run");
        return;
    };
    let dir = TempDir::new("v1-open");
    let write = |store: &mut DocumentStore, path: &Path, writer: Option<ReferenceWriter>| {
        let plan = store
            .prepare_save(
                SaveTarget::SaveAs {
                    path: path.to_owned(),
                    replace: false,
                },
                SaveOptions {
                    reference_writer: writer,
                    ..SaveOptions::default()
                },
            )
            .unwrap();
        let receipt = run(plan);
        store.finish_save(receipt).unwrap();
    };
    // Removed media: plain history (the writer), and packed regardless.
    let mut removed = with_media();
    add_layer(&mut removed, "extra");
    remove_media(&mut removed);
    let plain = dir.path("removed-plain.pmv");
    write(&mut removed, &plain, None);
    let forced = dir.path("removed-packed-regardless.pmv");
    write(
        &mut removed,
        &forced,
        Some(ReferenceWriter::PackedRegardless),
    );
    // Media the project still lists: packed history.
    let mut kept = with_media();
    add_layer(&mut kept, "extra");
    let packed = dir.path("listed-packed.pmv");
    write(&mut kept, &packed, None);
    assert!(
        document(&packed)
            .get("history")
            .and_then(|h| h.get("format"))
            .is_some()
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/v1_reader/read_file.ts");
    let bun = std::env::var_os("PM_TEST_BUN").unwrap_or_else(|| "bun".into());
    for source in std::env::split_paths(&sources) {
        for (file, keeps) in [(&plain, true), (&packed, true), (&forced, false)] {
            let out = std::process::Command::new(&bun)
                .arg(&script)
                .arg(file)
                .arg("m")
                .env("V1_SRC", &source)
                .output()
                .expect("bun runs the old app's reader");
            let text = String::from_utf8_lossy(&out.stdout);
            eprintln!("{} {}:\n{text}", source.display(), file.display());
            assert_eq!(
                out.status.success(),
                keeps,
                "{} on {}: {text}{}",
                source.display(),
                file.display(),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
