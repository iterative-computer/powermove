//! A project whose media exist only inside its v1 project file. The fixtures
//! were written by v1.2.0's own save path (`fixtures/v1-embedded/generate.ts`):
//! the library entry's `savedHash` is v1's fingerprint of the project JSON,
//! never the file's SHA-256, so media must be found per asset.
mod support;
use pm_first_launch::{Config, Outcome, run};
use pm_kernel::{
    Kernel, KernelConfig,
    locks::OpenDecision,
    persistence::DiskFormat,
    session::{SessionSource, ShellCall, ShellReply},
};
use serde_json::{Value, json};
use std::{fs, path::Path};
use support::*;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/v1-embedded")
        .join(name)
}

/// The synthetic profile plus project `e`, saved by v1 to `file` with its media
/// in that file only (neither the media database nor the Media Store holds it,
/// unless `seed` puts it in the original's profile). `saved` names the fixture
/// whose library entry v1 kept.
fn migrated(root: &Path, saved: &str, file: &Path, seed: impl FnOnce(&Path)) -> (Config, String) {
    let config = profile(root);
    let source = &config.electron_profile;
    let entry: Value = read(&fixture(&format!("{saved}.json")));
    seed(source);
    write(
        &source.join("store/projects.json"),
        &json!([{"id":"p","name":"Synthetic project"},{"id":"e","name":"Embedded media"}]),
    );
    let mut project = entry["project"].clone();
    project["proj"]["id"] = "e".into();
    write(&source.join("store/project.e.json"), &project);
    write(
        &source.join("store/projectMeta.e.json"),
        &json!({"time":3,"file":{"path":file,"savedHash":entry["savedHash"],"baselineHash":entry["baselineHash"]}}),
    );
    let outcome = run(&config, |_| Ok(())).unwrap();
    assert!(matches!(outcome, Outcome::Complete { .. }), "{outcome:?}");
    let id = checkpoint(&config).projects["e"].clone();
    (config, id)
}

fn open(config: &Config, project: &str) -> Kernel {
    let mut kernel = Kernel::new(KernelConfig::default());
    assert!(matches!(
        kernel
            .shell(ShellCall::AttachLibrary {
                profile: config.native_profile.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None,
            })
            .unwrap(),
        ShellReply::Library(OpenDecision::Writable)
    ));
    kernel
        .shell(ShellCall::Open {
            id: "copy".into(),
            source: SessionSource::Library {
                profile: config.native_profile.clone(),
                project: project.into(),
            },
            electron_profile: None,
        })
        .unwrap();
    kernel
}

/// SHA-256 of the asset's bytes as the editor reads them, `None` when offline.
fn bytes(kernel: &Kernel, root: &Path) -> Option<String> {
    let store = kernel.project("copy").unwrap().store();
    let item = store.media().get("gradient")?;
    assert!(
        !store.media().is_proxy_only("gradient"),
        "an original, never a proxy"
    );
    let out = root.join("read-gradient");
    let _ = fs::remove_file(&out);
    item.copy_to(&out).ok()?;
    Some(digest(&fs::read(out).unwrap()))
}
fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn embedded_bytes(file: &Path) -> String {
    let opened =
        pm_kernel::project_io::open_project(file, pm_kernel::project_io::OpenOptions::default())
            .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("expected");
    opened
        .file
        .media
        .get("gradient")
        .unwrap()
        .copy_to(&out)
        .unwrap();
    digest(&fs::read(out).unwrap())
}

#[test]
fn a_migrated_copy_finds_media_its_v1_project_file_holds() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let file = root.join("Embedded.pmv");
    fs::copy(fixture("Embedded.pmv"), &file).unwrap();
    let before = fs::read(&file).unwrap();
    let (config, id) = migrated(&root, "Embedded.pmv", &file, |_| {});
    let kernel = open(&config, &id);
    assert_eq!(bytes(&kernel, &root), Some(embedded_bytes(&file)));
    // The copy takes the media, not the file: it is no file's project.
    assert_eq!(kernel.project("copy").unwrap().store().path(), None);
    // The original's file is read, never written or locked.
    assert_eq!(fs::read(&file).unwrap(), before);
    assert!(!pm_kernel::locks::project_lock_path(&file).exists());
}

#[test]
fn another_project_saved_at_that_path_lends_no_media() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let file = root.join("Embedded.pmv");
    // The same asset id, saved by another project with other media.
    fs::copy(fixture("Other.pmv"), &file).unwrap();
    let (config, id) = migrated(&root, "Embedded.pmv", &file, |_| {});
    let kernel = open(&config, &id);
    assert_eq!(bytes(&kernel, &root), None);
}

#[test]
fn media_whose_bytes_no_longer_match_their_record_stay_offline() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let file = root.join("Embedded.pmv");
    let mut data = fs::read(fixture("Embedded.pmv")).unwrap();
    // The PNG follows the document; flip one byte inside it.
    let at = data.windows(4).position(|w| w == b"IDAT").unwrap() + 16;
    data[at] ^= 0xff;
    fs::write(&file, &data).unwrap();
    let (config, id) = migrated(&root, "Embedded.pmv", &file, |_| {});
    let kernel = open(&config, &id);
    assert_eq!(bytes(&kernel, &root), None);
}

#[test]
fn a_pmv3_file_lends_media_its_asset_fingerprint_proves() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let file = root.join("Embedded.pmv");
    fs::copy(fixture("Embedded-pmv3.pmv"), &file).unwrap();
    let (config, id) = migrated(&root, "Embedded-pmv3.pmv", &file, |_| {});
    let kernel = open(&config, &id);
    assert_eq!(bytes(&kernel, &root), Some(embedded_bytes(&file)));
}

/// No checksum, and the fingerprint names the file chosen before conversion:
/// nothing proves these bytes, so they are not used.
#[test]
fn unproven_pmv3_media_stay_offline() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let file = root.join("Embedded.pmv");
    fs::copy(fixture("Converted-pmv3.pmv"), &file).unwrap();
    let (config, id) = migrated(&root, "Converted-pmv3.pmv", &file, |_| {});
    let kernel = open(&config, &id);
    assert_eq!(bytes(&kernel, &root), None);
}

/// Bytes the library's Media Store holds win: a library save never copies
/// them out of the original's file again.
#[test]
fn media_the_library_store_holds_are_read_from_the_store() {
    use sha2::{Digest, Sha256};
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let file = root.join("Embedded.pmv");
    fs::copy(fixture("Embedded.pmv"), &file).unwrap();
    let image = {
        let opened = pm_kernel::project_io::open_project(
            &file,
            pm_kernel::project_io::OpenOptions::default(),
        )
        .unwrap();
        let out = root.join("image");
        opened
            .file
            .media
            .get("gradient")
            .unwrap()
            .copy_to(&out)
            .unwrap();
        fs::read(out).unwrap()
    };
    let entry: Value = read(&fixture("Embedded.pmv.json"));
    let key = entry["project"]["proj"]["assets"]["gradient"]["storageKey"]
        .as_str()
        .unwrap()
        .to_owned();
    let (config, id) = migrated(&root, "Embedded.pmv", &file, |source| {
        let store = source.join("Media Store");
        let hash = blake3::hash(&image).to_hex().to_string();
        fs::create_dir_all(store.join("objects").join(&hash[..2])).unwrap();
        fs::write(store.join("objects").join(&hash[..2]).join(&hash), &image).unwrap();
        write(
            &store
                .join("keys")
                .join(format!("{:x}.json", Sha256::digest(key.as_bytes()))),
            &json!({"key":key,"hash":hash,"size":image.len(),"type":"image/png",
                "revision":"v1-revision-320","at":1,"source":"stream"}),
        );
    });
    let kernel = open(&config, &id);
    assert_eq!(bytes(&kernel, &root), Some(digest(&image)));
    let item = kernel
        .project("copy")
        .unwrap()
        .store()
        .media()
        .get("gradient")
        .unwrap()
        .clone();
    assert!(
        matches!(&item.source, pm_kernel::project_io::MediaSource::File { path, .. }
            if path.starts_with(config.native_profile.join("Media Store/objects"))),
        "{:?}",
        item.source
    );
}

/// File > Open of a v1 project file reads the media it embeds (both formats).
#[test]
fn a_v1_project_file_opened_directly_shows_its_embedded_media() {
    for saved in ["Embedded.pmv", "Embedded-pmv3.pmv"] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let file = root.join(saved);
        fs::copy(fixture(saved), &file).unwrap();
        let (config, _) = migrated(&root, saved, &root.join("elsewhere.pmv"), |_| {});
        let mut kernel = Kernel::new(KernelConfig::default());
        kernel
            .shell(ShellCall::AttachLibrary {
                profile: config.native_profile.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None,
            })
            .unwrap();
        kernel
            .shell(ShellCall::Open {
                id: "copy".into(),
                source: SessionSource::File(file.clone()),
                electron_profile: None,
            })
            .unwrap();
        assert_eq!(
            bytes(&kernel, &root),
            Some(embedded_bytes(&file)),
            "{saved}"
        );
    }
}

/// Closing the copy keeps the media it read from the original's file in the
/// library, so the copy still has them once that file is gone.
#[test]
fn the_copy_keeps_embedded_media_after_the_original_file_is_gone() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let file = root.join("Embedded.pmv");
    fs::copy(fixture("Embedded.pmv"), &file).unwrap();
    let expected = embedded_bytes(&file);
    let (config, id) = migrated(&root, "Embedded.pmv", &file, |_| {});
    let mut kernel = open(&config, &id);
    let session = kernel.session_metadata("copy").unwrap().handle.clone();
    kernel.shell(ShellCall::Close(session)).unwrap();
    fs::remove_file(&file).unwrap();
    kernel
        .shell(ShellCall::Open {
            id: "copy".into(),
            source: SessionSource::Library {
                profile: config.native_profile.clone(),
                project: id.clone(),
            },
            electron_profile: None,
        })
        .unwrap();
    assert_eq!(bytes(&kernel, &root), Some(expected));
}
