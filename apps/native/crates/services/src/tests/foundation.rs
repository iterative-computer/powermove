use super::test_storage;
use crate::error::Result;
use crate::service_bail as bail;
use crate::{
    fonts, library, settings,
    storage::{self, ServiceStorage},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    io::{self, Read},
};

#[test]
fn settings_survive_reopen_with_unknown_fields() {
    let (temp, mut store) = test_storage();
    let value = json!({"future":[null,{"unknown":"日本語"}],"restoreWindows":false});
    store.set("ext.synthetic", value.clone()).unwrap();
    drop(store);
    let store = ServiceStorage::open(temp.path().join("services")).unwrap();
    assert_eq!(store.setting("ext.synthetic").unwrap(), Some(value));
}
#[test]
fn settings_null_is_distinct_from_absent() {
    let (_temp, mut store) = test_storage();
    assert_eq!(store.setting("value").unwrap(), None);
    store.set("value", Value::Null).unwrap();
    assert_eq!(store.setting("value").unwrap(), Some(Value::Null));
    store.archive_setting("value").unwrap();
    assert_eq!(store.setting("value").unwrap(), None);
}
#[test]
fn deletion_and_replacement_retain_all_generations() {
    let (_temp, mut store) = test_storage();
    store.set("preference", json!(1)).unwrap();
    store.set("preference", json!(2)).unwrap();
    store.archive_setting("preference").unwrap();
    let dir = store.directory("settings", "preference").unwrap();
    let versions = storage::generations(&dir).unwrap();
    assert_eq!(versions.len(), 3);
    let first: Value = serde_json::from_slice(&fs::read(&versions[0].1).unwrap()).unwrap();
    assert_eq!(first["value"], 1);
}
#[test]
fn key_paths_cannot_escape_storage() {
    let (temp, mut store) = test_storage();
    store.set("../../escape", json!(true)).unwrap();
    assert!(!temp.path().join("escape").exists());
    assert!(store.set("bad\nkey", Value::Null).is_err());
    assert!(store.backup("../escape", &b"bytes"[..]).is_err());
}
#[test]
fn second_writer_is_refused_and_lock_releases_on_drop() {
    let (temp, store) = test_storage();
    let root = temp.path().join("services");
    assert!(ServiceStorage::open(&root).is_err());
    drop(store);
    assert!(ServiceStorage::open(root).is_ok());
}
#[test]
fn foreign_directory_and_relative_root_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("authored"), b"kept").unwrap();
    assert!(ServiceStorage::open(temp.path()).is_err());
    assert!(ServiceStorage::open("relative").is_err());
    assert_eq!(fs::read(temp.path().join("authored")).unwrap(), b"kept");
}
#[cfg(unix)]
#[test]
fn symlink_roots_and_generation_files_are_refused() {
    use std::os::unix::fs::symlink;
    let (temp, mut store) = test_storage();
    let alias = temp.path().join("alias");
    symlink(temp.path().join("services"), &alias).unwrap();
    assert!(ServiceStorage::open(alias).is_err());
    store.set("key", json!("before")).unwrap();
    let dir = store.directory("settings", "key").unwrap();
    let external = temp.path().join("external");
    fs::write(&external, b"do not read").unwrap();
    symlink(external, dir.join("00000000000000000002.data")).unwrap();
    assert!(store.setting("key").is_err());
}
#[cfg(unix)]
#[test]
fn storage_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, mut store) = test_storage();
    store.set("private", json!(1)).unwrap();
    assert_eq!(
        fs::metadata(temp.path().join("services"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let entries = storage::generations(&store.directory("settings", "private").unwrap()).unwrap();
    assert_eq!(
        fs::metadata(&entries[0].1).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
#[test]
fn corruption_is_explicit_and_cannot_be_covered_by_new_setting() {
    let (_temp, mut store) = test_storage();
    store.set("key", json!("before")).unwrap();
    let dir = store.directory("settings", "key").unwrap();
    fs::write(dir.join("00000000000000000002.data"), b"interrupted JSON").unwrap();
    assert!(store.setting("key").is_err());
    assert!(store.set("key", json!("after")).is_err());
    assert_eq!(storage::generations(&dir).unwrap().len(), 2);
}
#[test]
fn settings_reject_oversize_without_losing_value() {
    let (_temp, mut store) = test_storage();
    store.set("key", json!(1)).unwrap();
    assert!(
        store
            .set("key", Value::String("x".repeat(storage::MAX_SETTING_BYTES)))
            .is_err()
    );
    assert_eq!(store.setting("key").unwrap(), Some(json!(1)));
}
struct Interrupted(bool);
impl Read for Interrupted {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.0 {
            return Err(io::Error::other("synthetic interrupted source"));
        }
        self.0 = true;
        buffer[..4].copy_from_slice(b"part");
        Ok(4)
    }
}
#[test]
fn interrupted_backup_keeps_previous_and_partial_then_retries() {
    let (_temp, mut store) = test_storage();
    let first = store.backup("project", &b"original"[..]).unwrap();
    assert!(store.backup("project", Interrupted(false)).is_err());
    assert_eq!(store.backups("project").unwrap(), vec![first.clone()]);
    let third = store.backup("project", &b"next"[..]).unwrap();
    assert_eq!(third.generation, 3);
    assert_eq!(fs::read(&first.path).unwrap(), b"original");
    assert!(
        first
            .path
            .parent()
            .unwrap()
            .join("00000000000000000002.pending")
            .exists()
    );
}
#[test]
fn backups_stream_and_never_prune_sixth_generation() {
    let (_temp, mut store) = test_storage();
    let bytes = vec![7; 2 * 1024 * 1024];
    for _ in 0..6 {
        store.backup("project", bytes.as_slice()).unwrap();
    }
    let all = store.backups("project").unwrap();
    assert_eq!(all.len(), 6);
    assert_eq!(all[0].generation, 6);
    assert!(all.iter().all(
        |backup| backup.bytes == bytes.len() as u64 && backup.sha256 == storage::digest(&bytes)
    ));
}

#[test]
fn backup_inventory_detects_corruption_against_publication_receipt() {
    let (_temp, mut store) = test_storage();
    let backup = store.backup("project", &b"original"[..]).unwrap();
    fs::write(&backup.path, b"modified").unwrap();
    assert!(store.backups("project").is_err());
}

#[test]
fn valid_json_corruption_is_not_accepted_as_a_new_setting_value() {
    let (_temp, mut store) = test_storage();
    store.set("key", json!("before")).unwrap();
    let entries = storage::generations(&store.directory("settings", "key").unwrap()).unwrap();
    let mut value: Value = serde_json::from_slice(&fs::read(&entries[0].1).unwrap()).unwrap();
    value["value"] = json!("after");
    fs::write(&entries[0].1, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(store.setting("key").is_err());
    assert!(store.set("key", json!("next")).is_err());
}

#[derive(Default)]
struct LibraryDouble {
    entries: Vec<library::ProjectEntry>,
    recent: Vec<String>,
    actions: Vec<library::LibraryAction>,
    fail: bool,
}
impl library::LibraryHook for LibraryDouble {
    fn list(&mut self) -> Result<Vec<library::ProjectEntry>> {
        Ok(self.entries.clone())
    }
    fn recents(&mut self) -> Result<Vec<String>> {
        Ok(self.recent.clone())
    }
    fn apply(&mut self, action: library::LibraryAction) -> Result<()> {
        if self.fail {
            bail!("kernel refused")
        }
        self.actions.push(action);
        Ok(())
    }
}
#[test]
fn library_preserves_recency_and_filters_archived_missing_duplicates() {
    let entries = ["old", "new", "trash"]
        .map(|id| library::ProjectEntry {
            id: id.into(),
            name: id.into(),
            modified_ms: 1,
            trashed: id == "trash",
        })
        .to_vec();
    let mut service = library::LibraryService(LibraryDouble {
        entries,
        recent: ["new", "new", "missing", "trash", "old"]
            .map(String::from)
            .to_vec(),
        ..Default::default()
    });
    assert_eq!(service.list(false).unwrap().len(), 2);
    assert_eq!(service.list(true).unwrap().len(), 3);
    assert_eq!(
        service
            .recents()
            .unwrap()
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        ["new", "old"]
    );
}
#[test]
fn checkpoint_revision_and_archive_are_forwarded_without_second_writer() {
    let mut service = library::LibraryService(LibraryDouble::default());
    let action = library::LibraryAction::Checkpoint {
        id: "project".into(),
        expected_revision: 42,
    };
    service.apply(action.clone()).unwrap();
    service
        .apply(library::LibraryAction::Archive {
            id: "project".into(),
        })
        .unwrap();
    service
        .apply(library::LibraryAction::Restore {
            id: "project".into(),
        })
        .unwrap();
    assert_eq!(service.0.actions[0], action);
    assert_eq!(service.0.actions.len(), 3);
}
#[test]
fn library_refusal_and_invalid_duplicate_do_not_succeed() {
    let mut service = library::LibraryService(LibraryDouble {
        fail: true,
        ..Default::default()
    });
    assert!(
        service
            .apply(library::LibraryAction::Open {
                id: "project".into()
            })
            .is_err()
    );
    assert!(
        service
            .apply(library::LibraryAction::Duplicate {
                id: "project".into(),
                new_id: "project".into()
            })
            .is_err()
    );
    assert!(service.0.actions.is_empty());
}

struct FontDouble {
    versions: VecDeque<Option<u64>>,
    reads: usize,
    families: Option<Vec<String>>,
    fail: bool,
}
impl fonts::FontHook for FontDouble {
    fn version(&mut self) -> Result<Option<u64>> {
        Ok(self.versions.pop_front().unwrap_or(Some(1)))
    }
    fn families(&mut self) -> Result<Option<Vec<String>>> {
        self.reads += 1;
        if self.fail {
            bail!("font broker unavailable")
        }
        Ok(self.families.clone())
    }
}
fn font_catalog(
    versions: &[Option<u64>],
    families: Option<Vec<String>>,
) -> fonts::FontCatalog<FontDouble> {
    fonts::FontCatalog::new(FontDouble {
        versions: versions.iter().copied().collect(),
        families,
        reads: 0,
        fail: false,
    })
}
#[test]
fn stable_font_registry_caches_owned_copies() {
    let mut fonts = font_catalog(
        &[Some(1), Some(1), Some(1)],
        Some(vec!["Synthetic Family".into()]),
    );
    let mut result = fonts.families().unwrap().unwrap();
    result.clear();
    assert_eq!(fonts.families().unwrap().unwrap().len(), 1);
    assert_eq!(fonts.hook.reads, 1);
}
#[test]
fn registry_change_during_font_read_prevents_cache() {
    let mut fonts = font_catalog(
        &[Some(1), Some(2), Some(2), Some(2)],
        Some(vec!["Synthetic".into()]),
    );
    fonts.families().unwrap();
    fonts.families().unwrap();
    assert_eq!(fonts.hook.reads, 2);
}
#[test]
fn empty_unknown_and_failed_fonts_can_recover() {
    for families in [None, Some(vec![])] {
        let mut fonts = font_catalog(&[], families);
        fonts.families().unwrap();
        fonts.families().unwrap();
        assert_eq!(fonts.hook.reads, 2);
    }
    let mut fonts = font_catalog(&[None, None], Some(vec!["Synthetic".into()]));
    fonts.families().unwrap();
    fonts.families().unwrap();
    assert_eq!(fonts.hook.reads, 2);
    fonts.hook.fail = true;
    assert!(fonts.families().is_err());
    fonts.hook.fail = false;
    assert!(fonts.families().is_ok());
}

#[derive(Default)]
struct SettingsDouble {
    writes: Vec<(String, Option<Value>)>,
    value: Option<Value>,
    fail: bool,
}
impl settings::SettingsHook for SettingsDouble {
    fn get(&mut self, _: &str) -> Result<Option<Value>> {
        Ok(self.value.clone())
    }
    fn apply(&mut self, key: &str, value: Option<Value>) -> Result<u64> {
        if self.fail {
            bail!("kernel state refused")
        }
        self.writes.push((key.into(), value.clone()));
        self.value = value;
        Ok(self.writes.len() as u64)
    }
}
#[test]
fn app_settings_delegate_null_deletion_and_unknown_values_to_kernel() {
    let mut service = settings::SettingsService(SettingsDouble::default());
    assert_eq!(
        service
            .set("unknown.future", json!({"nested":[null,true]}))
            .unwrap(),
        1
    );
    assert_eq!(
        service.get("unknown.future").unwrap(),
        Some(json!({"nested":[null,true]}))
    );
    assert_eq!(service.set("unknown.future", Value::Null).unwrap(), 2);
    assert_eq!(service.get("unknown.future").unwrap(), Some(Value::Null));
    assert_eq!(service.archive("unknown.future").unwrap(), 3);
    assert_eq!(service.get("unknown.future").unwrap(), None);
    assert_eq!(service.0.writes.len(), 3);
}
#[test]
fn app_settings_refuse_invalid_keys_and_propagate_kernel_errors() {
    let mut service = settings::SettingsService(SettingsDouble {
        fail: true,
        ..Default::default()
    });
    assert!(service.set("theme", json!("dark")).is_err());
    assert!(service.archive("theme").is_err());
    assert!(service.get("invalid\nkey").is_err());
    assert!(service.0.writes.is_empty());
}

#[test]
fn font_registry_change_invalidates_previously_cached_family_list() {
    let mut fonts = font_catalog(
        &[Some(1), Some(1), Some(2), Some(2)],
        Some(vec!["Old Family".into()]),
    );
    assert_eq!(fonts.families().unwrap(), Some(vec!["Old Family".into()]));
    fonts.hook.families = Some(vec!["New Family".into()]);
    assert_eq!(fonts.families().unwrap(), Some(vec!["New Family".into()]));
    assert_eq!(fonts.hook.reads, 2);
}
