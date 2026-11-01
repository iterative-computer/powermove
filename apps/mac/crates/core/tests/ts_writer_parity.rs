//! The Rust PMV4 writer against files written by the OLD TypeScript writer
//! (`ProjectSave`, `main:apps/desktop/src/main/project-save.ts`), produced by
//! tests/fixtures/ts-writer/generate.ts. Same inputs, same bytes.

mod common;

use common::*;
use pm_mac_core::project_io::save::SaveLayout;
use pm_mac_core::project_io::{SaveOptions, SaveTarget};
use pm_mac_core::{Command, Origin};

/// A first save of a project: document, then media, index, footer.
#[test]
fn first_save_is_byte_identical_to_the_typescript_writer() {
    let dir = TempDir::new("ts-fresh");
    let mut store = open_store(&ts_fixture("fresh.pmv"));
    let out = dir.path("copy.pmv");
    save_as(&mut store, &out).unwrap();
    assert_eq!(read(&out), read(&ts_fixture("fresh.pmv")));
    assert!(!store.is_dirty());
}

/// Saving the same file again after an edit: the old writer cloned the file
/// and appended the new document, index and footer, reusing both media
/// records in place. So does this one.
#[test]
fn in_place_save_appends_like_the_typescript_writer() {
    let dir = TempDir::new("ts-append");
    let path = dir.path("project.pmv");
    std::fs::copy(ts_fixture("fresh.pmv"), &path).unwrap();
    let mut store = open_store(&path);
    store
        .execute(
            Command::SetStaticOpacity {
                layer_id: "a".into(),
                value: 50.0,
            },
            Origin::Interface,
        )
        .unwrap()
        .unwrap();
    assert!(store.is_dirty());
    let receipt = run_save(&mut store, SaveTarget::InPlace, SaveOptions::default()).unwrap();
    assert_eq!(receipt.layout, SaveLayout::Appended);
    assert_eq!(receipt.reused_media_bytes, 11 + 3000);
    store.finish_save(receipt).unwrap();
    assert_eq!(read(&path), read(&ts_fixture("append.pmv")));
    assert!(!store.is_dirty());
    assert_eq!(
        dir.entries(),
        ["project.pmv"],
        "no temp file is left behind"
    );
}

/// Save As never reuses the destination: the fresh layout again.
#[test]
fn save_as_after_an_edit_is_byte_identical_to_the_typescript_writer() {
    let dir = TempDir::new("ts-saveas");
    let path = dir.path("project.pmv");
    std::fs::copy(ts_fixture("fresh.pmv"), &path).unwrap();
    let mut store = open_store(&path);
    store
        .execute(
            Command::SetStaticOpacity {
                layer_id: "a".into(),
                value: 50.0,
            },
            Origin::Interface,
        )
        .unwrap();
    let out = dir.path("other.pmv");
    save_as(&mut store, &out).unwrap();
    assert_eq!(read(&out), read(&ts_fixture("save-as.pmv")));
    assert_eq!(store.path(), Some(out.as_path()));
}

/// The appended file opens, its history continues the old app's stack, and
/// undo walks back through both the TypeScript step and ours.
#[test]
fn history_written_by_either_writer_replays() {
    let dir = TempDir::new("ts-history");
    let path = dir.path("project.pmv");
    std::fs::copy(ts_fixture("append.pmv"), &path).unwrap();
    let mut store = open_store(&path);
    assert_eq!(
        store.history().labels(),
        ["Change Opacity", "Change Opacity"]
    );
    assert_eq!(opacity_of(&store, "a"), Some(50.0));
    store.undo().unwrap();
    assert_eq!(opacity_of(&store, "a"), Some(80.0));
    store.undo().unwrap();
    assert_eq!(opacity_of(&store, "a"), Some(100.0));
    assert!(store.undo().is_err());
    store.redo().unwrap();
    store.redo().unwrap();
    assert_eq!(opacity_of(&store, "a"), Some(50.0));
    assert!(!store.is_dirty(), "back at the opened state");
}
