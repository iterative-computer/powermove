use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use super::durable::{CHANGED_FILE, file_version, publish_file};
use super::files::{MediaRange, ProjectFiles};
use super::format::{CONTAINER_MAGIC, INCREMENTAL_MAGIC, PROJECT_FOOTER_BYTES, SaveMedia, read_incremental_index, sha256_hex};
use super::{plain_path, sanitize_save_name, sidecar_path};

const DOCUMENT: &[u8] = br#"{"proj":{"layers":[],"w":1920,"h":1080}}"#;

fn files(directory: &Path) -> Arc<ProjectFiles> {
    ProjectFiles::load(directory.join("project-files.json"), directory.join("backups"))
}

fn media(id: &str, revision: &str, data: &[u8]) -> SaveMedia {
    SaveMedia { id: id.into(), revision: revision.into(), kind: "image/png".into(), length: data.len() as u64 }
}

/// Run one incremental save, uploading only what the host asks for.
fn save(files: &Arc<ProjectFiles>, id: &str, items: &[(&str, &str, &[u8])], selected: Option<&Path>) -> (Vec<String>, String) {
    let declared = items.iter().map(|(id, revision, data)| media(id, revision, data)).collect();
    let selected = selected.map(|path| path.to_str().unwrap().to_owned());
    let mut save = files.begin_incremental(id, declared, DOCUMENT.len() as u64, selected.as_deref()).unwrap();
    let required = save.required.clone();
    save.write(None, DOCUMENT).unwrap();
    for (asset, _, data) in items {
        if required.iter().any(|needed| needed == asset) && !data.is_empty() {
            save.write(Some(asset), data).unwrap();
        }
    }
    (required, save.finish().unwrap())
}

fn index_of(path: &Path) -> super::format::ProjectIndex {
    let bytes = fs::read(path).unwrap();
    read_incremental_index(bytes.len() as u64, |offset, length| Ok(bytes[offset as usize..(offset + length) as usize].to_vec())).unwrap()
}

#[test]
fn incremental_save_reuses_media_from_the_previous_file() {
    let directory = tempfile::tempdir().unwrap();
    let files = files(directory.path());
    let target = directory.path().join("Film.pmv");

    let (required, path) = save(&files, "p1", &[("a", "1", b"alpha"), ("b", "1", b"bravo")], Some(&target));
    assert_eq!(required, ["a", "b"]);
    assert_eq!(Path::new(&path), target);
    let first = index_of(&target);
    let before = fs::read(&target).unwrap();

    // Same project, no Save As: only the changed asset is uploaded again.
    let (required, _) = save(&files, "p1", &[("a", "1", b"alpha"), ("b", "2", b"bravo two"), ("c", "1", b"")], None);
    assert_eq!(required, ["b", "c"]);
    let second = index_of(&target);
    let reused = second.media.iter().find(|item| item.id == "a").unwrap();
    assert_eq!(reused, first.media.iter().find(|item| item.id == "a").unwrap(), "unchanged media keeps its range");
    let bytes = fs::read(&target).unwrap();
    assert!(bytes.starts_with(&before), "a small change appends instead of compacting");
    let changed = second.media.iter().find(|item| item.id == "b").unwrap();
    assert_eq!(&bytes[changed.offset as usize..(changed.offset + changed.length) as usize], b"bravo two");
    assert_eq!(second.media.iter().find(|item| item.id == "c").unwrap().length, 0);

    // The overwritten version was backed up, and the result opens cleanly.
    assert_eq!(files.backups("p1").unwrap().len(), 1);
    let opened = files.open(&target).unwrap();
    assert_eq!(opened.document, serde_json::from_slice::<Value>(DOCUMENT).unwrap());
    assert_eq!(opened.media.len(), 3);
    files.close(&opened.token, true).unwrap();
    assert_eq!(files.destination("p1").as_deref(), target.to_str());
}

#[test]
fn save_as_rewrites_media_instead_of_trusting_the_chosen_file() {
    let directory = tempfile::tempdir().unwrap();
    let files = files(directory.path());
    let first = directory.path().join("A.pmv");
    save(&files, "p1", &[("a", "1", b"alpha")], Some(&first));
    let copy = directory.path().join("B.pmv");
    let (required, _) = save(&files, "p2", &[("a", "1", b"alpha")], Some(&copy));
    assert_eq!(required, ["a"]);
}

#[test]
fn refuses_to_overwrite_a_file_changed_outside_the_app() {
    let directory = tempfile::tempdir().unwrap();
    let files = files(directory.path());
    let target = directory.path().join("Film.pmv");
    save(&files, "p1", &[("a", "1", b"alpha")], Some(&target));

    fs::write(&target, b"someone else's work").unwrap();
    let declared = vec![media("a", "1", b"alpha")];
    let error = files.begin_incremental("p1", declared, DOCUMENT.len() as u64, None).err().unwrap();
    assert_eq!(error.to_string(), CHANGED_FILE);
    let error = files.save("p1", &mut &DOCUMENT[..], None).unwrap_err();
    assert_eq!(error.to_string(), CHANGED_FILE);
    assert_eq!(fs::read(&target).unwrap(), b"someone else's work");

    // A file that appears at a new destination after the check is never replaced.
    let temporary = directory.path().join("staged.tmp");
    fs::write(&temporary, b"ours").unwrap();
    let late = directory.path().join("Late.pmv");
    fs::write(&late, b"theirs").unwrap();
    assert_eq!(publish_file(&temporary, &late, None).unwrap_err().to_string(), CHANGED_FILE);
    assert_eq!(fs::read(&late).unwrap(), b"theirs");
}

#[test]
fn a_change_during_a_save_aborts_the_commit() {
    let directory = tempfile::tempdir().unwrap();
    let files = files(directory.path());
    let target = directory.path().join("Film.pmv");
    save(&files, "p1", &[], Some(&target));

    let mut pending = files.begin_incremental("p1", vec![], DOCUMENT.len() as u64, None).unwrap();
    pending.write(None, DOCUMENT).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(&target, b"edited elsewhere").unwrap();
    assert_eq!(pending.finish().unwrap_err().to_string(), CHANGED_FILE);
    assert_eq!(fs::read(&target).unwrap(), b"edited elsewhere");
    let leftovers: Vec<_> = fs::read_dir(directory.path()).unwrap().flatten().filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp")).collect();
    assert!(leftovers.is_empty(), "the transaction file is removed");
}

#[test]
fn opens_pmv3_and_legacy_json() {
    let directory = tempfile::tempdir().unwrap();
    let files = files(directory.path());

    let header = serde_json::to_vec(&json!({ "document": { "layers": [], "w": 1, "h": 1 }, "media": [{ "id": "m", "type": "audio/wav", "offset": 0, "length": 3 }] })).unwrap();
    let mut container = CONTAINER_MAGIC.to_vec();
    container.extend_from_slice(&(header.len() as u32).to_le_bytes());
    container.extend_from_slice(&header);
    container.extend_from_slice(b"wav");
    let pmv3 = directory.path().join("old.pmv");
    fs::write(&pmv3, &container).unwrap();
    let opened = files.open(&pmv3).unwrap();
    let MediaRange::Container(range) = &opened.media[0] else { panic!("expected a PMV3 range") };
    assert_eq!(range.offset as usize, 9 + header.len());
    assert_eq!(files.read(&opened.token, range.offset, range.length).unwrap(), b"wav");
    assert!(files.read(&opened.token, range.offset, 4).is_err(), "reads stay inside the file");

    let legacy = directory.path().join("legacy.json");
    fs::write(&legacy, DOCUMENT).unwrap();
    let opened = files.open(&legacy).unwrap();
    assert!(opened.media.is_empty());
    assert_eq!(opened.document["proj"]["w"], 1920);
    // Opening grants Save to the new project id.
    assert_eq!(files.destination(&opened.project_id).as_deref(), legacy.to_str());

    let other = directory.path().join("other.json");
    fs::write(&other, br#"{"hello":"world"}"#).unwrap();
    assert_eq!(files.open(&other).unwrap_err().to_string(), "This is not a Powermove project.");
}

#[test]
fn opening_pmv4_verifies_media_checksums() {
    let directory = tempfile::tempdir().unwrap();
    let files = files(directory.path());
    let target = directory.path().join("Film.pmv");
    save(&files, "p1", &[("a", "1", b"alpha")], Some(&target));
    let index = index_of(&target);

    let mut bytes = fs::read(&target).unwrap();
    assert!(bytes.starts_with(INCREMENTAL_MAGIC));
    assert_eq!(index.document.sha256, sha256_hex(DOCUMENT));
    assert_eq!(bytes.len() as u64, index.media[0].offset + b"alpha".len() as u64 + PROJECT_FOOTER_BYTES + serde_json::to_vec(&index).unwrap().len() as u64);
    bytes[index.media[0].offset as usize] ^= 1;
    fs::write(&target, bytes).unwrap();
    assert_eq!(files.open(&target).unwrap_err().to_string(), "The project media checksum does not match.");
}

#[test]
fn associations_survive_a_restart() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("Film.pmv");
    save(&files(directory.path()), "p1", &[], Some(&target));
    let reloaded = files(directory.path());
    assert_eq!(reloaded.destination("p1").as_deref(), target.to_str());
    assert!(file_version(&target).unwrap().is_some());
}

#[test]
fn sanitizes_names_and_paths() {
    assert_eq!(sanitize_save_name(Some(&json!("../dir\\Film.mp4"))).as_deref(), Some("Film.mp4"));
    assert_eq!(sanitize_save_name(Some(&json!(".."))), None);
    assert_eq!(sanitize_save_name(Some(&json!(7))), None);
    assert_eq!(sidecar_path(Path::new("C:\\out\\Film.mp4"), "vtt", Some("en")), Path::new("C:\\out\\Film.en.vtt"));
    #[cfg(windows)]
    assert_eq!(plain_path(Path::new(r"\\?\C:\Users\x\a.pmv")), Path::new(r"C:\Users\x\a.pmv"));
}
