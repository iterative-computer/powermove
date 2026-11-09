//! N4 safety net: projects saved by the shipped app at current main
//! (`tests/fixtures/main-current`, written by main's own code, see
//! `tests/main_current/generate.mjs`) open in V2, save untouched without a
//! changed byte of document, and survive a person's edit, Undo/Redo, save and
//! reopen with every other field preserved: effect revisions and inputs,
//! Figma fields, timing/3D/blend versions, memory, workspace, unknown fields,
//! linked and owned media, and main's own undo history.
mod common;
use common::TempDir;
use pm_kernel::commands::layers::LayerPatch;
use pm_kernel::{DocumentStore, LayerCommand, OpenOptions, Origin, open_project};
use pm_model::container::{ReadOptions, read_container};
use pm_model::json::{Value, to_canonical_string};
use std::path::{Path, PathBuf};

const FIXTURE: &str = "tests/fixtures/main-current/current.pmv";

fn fixture(dir: &TempDir) -> PathBuf {
    let path = dir.path("current.pmv");
    std::fs::copy(common::crate_dir().join(FIXTURE), &path).unwrap();
    path
}

/// The decoded document member of a project file (`{v, proj, history, ws}`).
fn document(path: &Path) -> Value {
    let file = std::fs::File::open(path).unwrap();
    read_container(&file, ReadOptions { verify_media: true })
        .unwrap()
        .document
}

/// Paths (`a.b.3.c`) at which two JSON values differ, keys in either.
fn diff(a: &Value, b: &Value, at: &str, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<String> = x
                .keys()
                .chain(y.keys())
                .map(|k| k.to_string_lossy())
                .collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let path = format!("{at}.{k}");
                match (x.get(k.as_str()), y.get(k.as_str())) {
                    (Some(p), Some(q)) => diff(p, q, &path, out),
                    _ => out.push(path),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                diff(p, q, &format!("{at}.{i}"), out);
            }
        }
        _ if a == b => {}
        _ => out.push(at.to_owned()),
    }
}

fn layer_index(doc: &Value, name: &str) -> usize {
    doc.get("proj")
        .unwrap()
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .position(|l| l.get("name").and_then(Value::as_str) == Some(name))
        .unwrap_or_else(|| panic!("no layer {name}"))
}

fn save_as(store: &mut DocumentStore, path: &Path) {
    common::save_as(store, path).unwrap();
}

#[test]
fn main_saved_project_opens_and_an_untouched_save_keeps_every_document_value() {
    let dir = TempDir::new("main-current-untouched");
    let input = fixture(&dir);
    let original = document(&input);
    let opened = open_project(
        &input,
        OpenOptions {
            verify_media: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        opened.history.is_some(),
        "main's saved Undo history is read"
    );
    let mut store = DocumentStore::open(opened);
    assert!(
        !store.upgraded_on_open(),
        "opening must not rewrite a current-main document"
    );
    assert!(
        store.can_undo() && store.can_redo(),
        "main's undo cursor and pending redo survive"
    );
    // Linked originals stay links; owned media stays embedded.
    assert!(store.media().get("owned").is_some());
    assert!(store.media().get("linked").is_none());

    // The fixture carries main-delta.md's data section (guards regeneration).
    let text = to_canonical_string(&original);
    for needle in [
        r#""version":99"#,
        r#""type":"futureEffect""#,
        r#""kind":"path""#,
        r#""kind":"asset""#,
        r#""stage":"masks""#,
        r#""space":"source""#,
        r#""effectSpace":"source""#,
        r#""timingVersion":2"#,
        r#""legacyStartDuration":4"#,
        r#""switch3DVersion":2"#,
        r#""scene3dDefaultsVersion":2"#,
        r#""blendVersion":2"#,
        r#""blend":"colorburn""#,
        r#""blend":"linearburn""#,
        r#""fillGradient""#,
        r#""strokeStyle""#,
        r#""matteSources""#,
        r#""runs""#,
        r#""imageMapping""#,
        r#""arc""#,
        r#""type":"figmaShadow""#,
        r#""memoryEnabled":false"#,
        r#""kind":"constraint""#,
        r#""agentDockMigration":3"#,
        r#""futureProjectField""#,
        r#""futureCompField""#,
        r#""futureAssetField""#,
        r#""sourcePath":"/Volumes/Synthetic/Linked clip.mov""#,
        r#""continuousRaster":true"#,
        r#""separateDimensions":false"#,
        r#""sections""#,
    ] {
        assert!(text.contains(needle), "fixture lacks {needle}");
    }

    let output = dir.path("saved.pmv");
    save_as(&mut store, &output);
    let saved = document(&output);
    let mut changed = vec![];
    diff(&original, &saved, "", &mut changed);
    assert!(changed.is_empty(), "untouched save changed {changed:?}");
    // V2 writes canonical JSON (sorted keys); every number and string is
    // otherwise identical to main's bytes. Main's reader re-checks it below.
    assert_eq!(to_canonical_string(&saved), to_canonical_string(&original));
    let reopened = open_project(
        &output,
        OpenOptions {
            verify_media: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(reopened.file.media.len(), 1);
}

#[test]
fn an_edit_with_undo_and_redo_saves_and_reopens_changing_only_the_edited_field() {
    let dir = TempDir::new("main-current-edit");
    let input = fixture(&dir);
    let original = document(&input);
    let at = layer_index(&original, "Stagger title");
    let id = original
        .get("proj")
        .unwrap()
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap()[at]
        .get("id")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    let mut store = DocumentStore::open(open_project(&input, OpenOptions::default()).unwrap());

    store
        .execute(
            LayerCommand::Set {
                layer_id: id.as_str().into(),
                patch: LayerPatch {
                    name: Some("Stagger title (V2)".into()),
                    ..Default::default()
                },
            }
            .into(),
            Origin::Interface,
        )
        .unwrap();
    let edited = dir.path("edited.pmv");
    save_as(&mut store, &edited);
    let after = document(&edited);
    let mut changed = vec![];
    diff(
        original.get("proj").unwrap(),
        after.get("proj").unwrap(),
        "proj",
        &mut changed,
    );
    let allowed = [
        format!("proj.layers.{at}.name"),
        "proj.revision".into(),
        "proj.edits".into(),
    ];
    let unexpected: Vec<_> = changed
        .iter()
        .filter(|p| {
            !allowed
                .iter()
                .any(|a| p == &a || p.starts_with(&format!("{a}.")))
        })
        .collect();
    assert!(
        unexpected.is_empty(),
        "an edit changed unrelated fields {unexpected:?}"
    );
    assert!(changed.contains(&format!("proj.layers.{at}.name")));
    for key in ["v", "ws"] {
        assert_eq!(after.get(key), original.get(key), "{key}");
    }

    // Reopen the V2 save: Undo returns main's exact project; Redo the edit.
    let mut reopened = DocumentStore::open(open_project(&edited, OpenOptions::default()).unwrap());
    reopened.undo().unwrap();
    let undone = dir.path("undone.pmv");
    save_as(&mut reopened, &undone);
    let back = document(&undone);
    let mut changed = vec![];
    diff(
        original.get("proj").unwrap(),
        back.get("proj").unwrap(),
        "proj",
        &mut changed,
    );
    let unexpected: Vec<_> = changed
        .iter()
        .filter(|p| !p.starts_with("proj.revision") && !p.starts_with("proj.edits"))
        .collect();
    assert!(
        unexpected.is_empty(),
        "Undo after reopen left {unexpected:?}"
    );
    reopened.redo().unwrap();
    let redone = dir.path("redone.pmv");
    save_as(&mut reopened, &redone);
    assert_eq!(
        to_canonical_string(
            document(&redone)
                .get("proj")
                .unwrap()
                .get("layers")
                .unwrap()
        ),
        to_canonical_string(after.get("proj").unwrap().get("layers").unwrap())
    );
}

#[test]
fn mains_pending_redo_and_earlier_undo_apply_in_v2() {
    let dir = TempDir::new("main-current-history");
    let input = fixture(&dir);
    let mut store = DocumentStore::open(open_project(&input, OpenOptions::default()).unwrap());
    let name = |s: &DocumentStore| -> Vec<String> {
        s.layers()
            .into_iter()
            .map(|l| l.name)
            .filter(|n| n.starts_with("Effected shape") || n.starts_with("Renamed"))
            .collect()
    };
    assert_eq!(name(&store), ["Effected shape (edited)"]);
    store.redo().unwrap();
    assert_eq!(name(&store), ["Renamed then undone"]);
    store.undo().unwrap();
    store.undo().unwrap();
    store.undo().unwrap();
    assert_eq!(name(&store), ["Effected shape"]);
    assert!(!store.can_undo());
    while store.can_redo() {
        store.redo().unwrap();
    }
    store.undo().unwrap();
    // Main's own history patches predate its keyframe normalisation, so the
    // replayed project equals what main's Undo produces, not the saved
    // project; current_main_opens_v2_saves_and_undoes_the_v2_edit checks that.
    assert_eq!(name(&store), ["Effected shape (edited)"]);
}

/// The shipped app at current main opens what V2 saved (main's own reader,
/// hydrate and Undo, `tests/main_current/reopen.mjs`; Bun 1.4 required).
#[test]
fn current_main_opens_v2_saves_and_undoes_the_v2_edit() {
    let dir = TempDir::new("main-current-reverse");
    let input = fixture(&dir);
    let mut store = DocumentStore::open(open_project(&input, OpenOptions::default()).unwrap());
    let untouched = dir.path("untouched.pmv");
    save_as(&mut store, &untouched);
    let id = store
        .layers()
        .into_iter()
        .find(|l| l.name == "Stagger title")
        .unwrap()
        .id;
    store
        .execute(
            LayerCommand::Set {
                layer_id: id.as_str().into(),
                patch: LayerPatch {
                    name: Some("Stagger title (V2)".into()),
                    ..Default::default()
                },
            }
            .into(),
            Origin::Interface,
        )
        .unwrap();
    let edited = dir.path("edited.pmv");
    save_as(&mut store, &edited);
    let mut replay = DocumentStore::open(open_project(&input, OpenOptions::default()).unwrap());
    replay.redo().unwrap();
    for _ in 0..3 {
        replay.undo().unwrap();
    }
    while replay.can_redo() {
        replay.redo().unwrap();
    }
    replay.undo().unwrap();
    let replayed = dir.path("history.pmv");
    save_as(&mut replay, &replayed);
    let bun = std::env::var_os("PM_TEST_BUN").unwrap_or_else(|| "bun".into());
    let result = std::process::Command::new(bun)
        .arg(common::crate_dir().join("tests/main_current/reopen.mjs"))
        .args([&input, &untouched, &edited, &replayed])
        .args(["Stagger title", "Stagger title (V2)"])
        .current_dir(common::crate_dir())
        .output()
        .expect("Bun 1.4 runs current main's reader (source the native env)");
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
