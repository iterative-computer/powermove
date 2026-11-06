use pm_services::media_adapter::FileMedia;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;

#[test]
fn flat_and_linked_ranges_keep_unknown_metadata_and_refuse_changed_or_unsafe_sources() {
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    fs::create_dir(root.join("Index")).unwrap();
    let hash = format!("{:x}", Sha256::digest(b"abcdef"));
    let source = root.join("source.mov");
    fs::write(&source, b"abcdef").unwrap();
    let owned = format!("{hash}.mov");
    fs::write(root.join(&owned), b"abcdef").unwrap();
    for (key, location) in [
        ("media:owned", json!({"file":owned})),
        ("media:linked", json!({"sourcePath":source})),
    ] {
        let mut raw = json!({"key":key,"revision":"r1","size":6,"type":"video/quicktime","sha256":hash,"mtime":0,"future":{"kept":true}});
        raw.as_object_mut()
            .unwrap()
            .extend(location.as_object().unwrap().clone());
        fs::write(
            root.join("Index")
                .join(format!("{:x}.json", Sha256::digest(key))),
            raw.to_string(),
        )
        .unwrap();
    }
    let adapter = FileMedia::open(&root).unwrap();
    for key in ["media:owned", "media:linked"] {
        assert_eq!(adapter.read_range(key, 2, 3).unwrap(), b"cde");
        assert_eq!(adapter.record(key).unwrap().raw["future"]["kept"], true);
    }
    assert!(adapter.read_range("media:linked", u64::MAX, 1).is_err());
    assert!(
        adapter
            .read_range("media:linked", 0, 8 * 1024 * 1024 + 1)
            .is_err()
    );
    let old_mtime = fs::metadata(&source).unwrap().modified().unwrap();
    fs::write(&source, b"change").unwrap();
    fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old_mtime))
        .unwrap();
    assert!(adapter.read_range("media:linked", 0, 1).is_err());
    fs::remove_file(&source).unwrap();
    assert_eq!(
        adapter.status("media:linked").unwrap(),
        pm_native_migrate::library_media::SourceStatus::Missing
    );
    std::os::unix::fs::symlink(root.join(&owned), &source).unwrap();
    assert!(adapter.read_range("media:linked", 0, 1).is_err());
    assert_eq!(adapter.read_range("media:owned", 0, 6).unwrap(), b"abcdef");
}

#[test]
fn service_import_admits_into_real_kernel_and_saves_owned_bytes_offline() {
    use pm_kernel::{
        DocumentStore, Origin,
        project_io::{OpenOptions, SaveOptions, SaveTarget, open_project},
    };
    use pm_services::{
        media::{ImportRequest, Probe, Provenance},
        media_adapter::{ImportMetadata, admit_import},
    };
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    let archive = root.join("archive");
    fs::create_dir(&archive).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&archive, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let owned = root.join("owned.mov");
    fs::write(&owned, b"abc").unwrap();
    let mut store = DocumentStore::new_untitled(
        pm_model::json::parse(
            r#"{"id":"p","layers":[],"w":100,"h":100,"assets":{},"assetFolders":{"f":{"id":"f"}}}"#,
        )
        .unwrap(),
    );
    let request = ImportRequest {
        project: "p".into(),
        asset: "a".into(),
        source: owned.clone(),
        provenance: Provenance::Original,
    };
    let probe = Probe {
        content_sha256: format!("{:x}", Sha256::digest(b"abc")),
        bytes: 3,
        duration_ticks: 30,
        ticks_per_second: 30,
        width: 10,
        height: 10,
        audio_channels: 0,
    };
    let asset = pm_model::json::parse(r#"{"name":"synthetic.mov","folder":"f","storageKey":"media:a","cloud":{"keep":true},"future":7}"#).unwrap();
    let admit = |s: &mut DocumentStore| {
        admit_import(
            s,
            "synthetic-job",
            &request,
            &probe,
            ImportMetadata {
                asset: asset.clone(),
                mime: "video/quicktime".into(),
                revision: "r1".into(),
            },
            &archive,
        )
    };
    admit(&mut store).unwrap();
    let revision = store.revision();
    admit(&mut store).unwrap();
    assert_eq!(revision, store.revision());
    assert_eq!(store.media().len(), 1);
    store.undo().unwrap();
    assert!(
        store
            .root()
            .unwrap()
            .get("assets")
            .unwrap()
            .get("a")
            .is_none()
    );
    store.redo().unwrap();
    store
        .execute(
            pm_kernel::MediaCommand::SetAssetFolder {
                asset_ids: vec!["a".into()],
                folder: None,
            }
            .into(),
            Origin::Interface,
        )
        .unwrap();
    fs::remove_file(&owned).unwrap(); // The admitted private copy already survives offline.
    let output = root.join("saved.pmv");
    let receipt = store
        .prepare_save(
            SaveTarget::SaveAs {
                path: output.clone(),
                replace: false,
            },
            SaveOptions::default(),
        )
        .unwrap()
        .run()
        .unwrap();
    store.finish_save(receipt).unwrap();

    let reopened = DocumentStore::open(open_project(&output, OpenOptions::default()).unwrap());
    let metadata = reopened
        .root()
        .unwrap()
        .get("assets")
        .unwrap()
        .get("a")
        .unwrap()
        .to_value();
    assert_eq!(metadata.get("future").unwrap().as_f64(), Some(7.0));
    assert!(metadata.get("folder").is_none());
    let copied = root.join("recovered.mov");
    reopened.media().get("a").unwrap().copy_to(&copied).unwrap();
    assert_eq!(fs::read(copied).unwrap(), b"abc");
}

#[test]
fn queue_generation_fences_real_kernel_admission_and_failed_batches_are_atomic() {
    use pm_services::{media::*, media_adapter::KernelImportHook};
    let t = private_tempdir();
    let owned_root = t.path().canonicalize().unwrap();
    let source = owned_root.join("owned.mov");
    fs::write(&source, b"abc").unwrap();
    let mut store = pm_kernel::DocumentStore::new_untitled(
        pm_model::json::parse(r#"{"id":"p","layers":[],"w":1,"h":1,"assets":{}}"#).unwrap(),
    );
    let request = ImportRequest {
        project: "p".into(),
        asset: "a".into(),
        source: source.clone(),
        provenance: Provenance::ProxyOnly,
    };
    let probe = Probe {
        content_sha256: format!("{:x}", Sha256::digest(b"abc")),
        bytes: 3,
        duration_ticks: 30,
        ticks_per_second: 30,
        width: 1,
        height: 1,
        audio_channels: 0,
    };
    let mut queue = ImportQueue::default();
    queue.enqueue("job", request).unwrap();
    let stale = queue.start_next().unwrap().unwrap();
    queue.cancel("job").unwrap();
    queue.retry("job").unwrap();
    let current = queue.start_next().unwrap().unwrap();
    let mut hook = KernelImportHook {
        store: &mut store,
        owned_root: &owned_root,
        probe: |_: &std::path::Path| Ok(probe.clone()),
        metadata: |_: &ImportRequest, _: &Probe| {
            Ok((
                pm_model::json::parse(r#"{"name":"synthetic","future":true}"#).unwrap(),
                "video/quicktime".into(),
                "r1".into(),
            ))
        },
    };
    assert!(queue.finish(stale, Ok(probe.clone()), &mut hook).is_err());
    assert!(
        hook.store
            .root()
            .unwrap()
            .get("assets")
            .unwrap()
            .to_value()
            .as_object()
            .unwrap()
            .is_empty()
    );
    queue.finish(current, Ok(probe.clone()), &mut hook).unwrap();
    assert_eq!(
        hook.store
            .root()
            .unwrap()
            .get("assets")
            .unwrap()
            .get("a")
            .unwrap()
            .get("playbackProxy")
            .unwrap()
            .to_value(),
        pm_model::Value::Bool(true)
    );
    let before = hook.store.revision();
    let input = pm_kernel::import::ImportInput {
        asset_id: "b".into(),
        asset: pm_model::json::parse("{}").unwrap(),
        owned_path: source.clone(),
        mime: "video/quicktime".into(),
        revision: "r1".into(),
        size: 3,
        sha256: probe.content_sha256.clone(),
    };
    let mut collision = input.clone();
    collision.asset_id = "a".into();
    let batch =
        pm_kernel::import::PreparedImport::capture("batch".into(), vec![input, collision]).unwrap();
    assert!(
        hook.store
            .execute(batch.command(), pm_kernel::Origin::Interface)
            .is_err()
    );
    assert_eq!(hook.store.revision(), before);
    assert!(
        hook.store
            .root()
            .unwrap()
            .get("assets")
            .unwrap()
            .get("b")
            .is_none()
    );
    fs::write(&source, b"bad").unwrap();
    assert!(
        pm_kernel::import::PreparedImport::capture(
            "changed".into(),
            vec![pm_kernel::import::ImportInput {
                asset_id: "c".into(),
                asset: pm_model::json::parse("{}").unwrap(),
                owned_path: source,
                mime: "video/quicktime".into(),
                revision: "r1".into(),
                size: 3,
                sha256: probe.content_sha256
            }]
        )
        .is_err()
    );
}

#[test]
fn live_actor_import_publishes_owned_media_and_refuses_completion_from_closed_document() {
    use pm_services::{
        media::*,
        media_adapter::{ActorImportHook, ImportMetadata, prepare_import},
    };
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    let source = root.join("source.mov");
    fs::write(&source, b"abc").unwrap();
    let project =
        || pm_model::json::parse(r#"{"id":"p","layers":[],"w":1,"h":1,"assets":{}}"#).unwrap();
    let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
    let kernel = actor.handle();
    kernel
        .open("p", pm_kernel::OpenSource::Untitled(project()))
        .wait()
        .unwrap()
        .unwrap();
    let document = kernel.published("p").unwrap().snapshot.doc_id;
    let request = ImportRequest {
        project: "p".into(),
        asset: "a".into(),
        source: source.clone(),
        provenance: Provenance::Original,
    };
    let probe = Probe {
        content_sha256: format!("{:x}", Sha256::digest(b"abc")),
        bytes: 3,
        duration_ticks: 30,
        ticks_per_second: 30,
        width: 1,
        height: 1,
        audio_channels: 0,
    };
    let mut queue = ImportQueue::default();
    queue.enqueue("job", request.clone()).unwrap();
    let ticket = queue.start_next().unwrap().unwrap();
    let mut hook = ActorImportHook {
        kernel: &kernel,
        document,
        owned_root: &root,
        probe: |_: &std::path::Path| Ok(probe.clone()),
        metadata: |_: &ImportRequest, _: &Probe| {
            Ok((
                pm_model::json::parse(r#"{"future":true}"#).unwrap(),
                "video/quicktime".into(),
                "r1".into(),
            ))
        },
    };
    queue.finish(ticket, Ok(probe.clone()), &mut hook).unwrap();
    let published = kernel.published("p").unwrap();
    assert_eq!(published.media.len(), 1);
    assert!(
        published
            .snapshot
            .root()
            .unwrap()
            .get("assets")
            .unwrap()
            .get("a")
            .is_some()
    );
    let mut late = request;
    late.asset = "b".into();
    let prepared = prepare_import(
        "late",
        &late,
        &probe,
        ImportMetadata {
            asset: pm_model::json::parse("{}").unwrap(),
            mime: "video/quicktime".into(),
            revision: "r1".into(),
        },
        &root,
    )
    .unwrap();
    assert!(kernel.close("p").wait().unwrap());
    kernel
        .open("p", pm_kernel::OpenSource::Untitled(project()))
        .wait()
        .unwrap()
        .unwrap();
    assert_ne!(kernel.published("p").unwrap().snapshot.doc_id, document);
    let ack = kernel
        .admit_import("p", document, prepared)
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    assert!(
        kernel
            .published("p")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .get("assets")
            .unwrap()
            .get("b")
            .is_none()
    );
    actor.shutdown();
}

#[test]
fn folder_batch_combines_verified_outputs_as_one_undo_step_without_losing_unknown_metadata() {
    use pm_services::{
        media::*,
        media_adapter::{ImportMetadata, prepare_import},
    };
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    let source = root.join("source.mov");
    fs::write(&source, b"abc").unwrap();
    let probe = Probe {
        content_sha256: format!("{:x}", Sha256::digest(b"abc")),
        bytes: 3,
        duration_ticks: 30,
        ticks_per_second: 30,
        width: 1,
        height: 1,
        audio_channels: 0,
    };
    let mut inputs = Vec::new();
    for id in ["a", "b"] {
        let request = ImportRequest {
            project: "p".into(),
            asset: id.into(),
            source: source.clone(),
            provenance: Provenance::Original,
        };
        inputs.push(
            prepare_import(
                id,
                &request,
                &probe,
                ImportMetadata {
                    asset: pm_model::json::parse(r#"{"folder":"child","cloud":{"future":true}}"#)
                        .unwrap(),
                    mime: "video/quicktime".into(),
                    revision: "r1".into(),
                },
                &root,
            )
            .unwrap(),
        );
    }
    let folders=pm_model::json::parse(r#"{"parent":{"id":"parent","name":"Folder","parent":null,"future":7},"child":{"id":"child","name":"Child","parent":"parent"}}"#).unwrap();
    let combined = pm_kernel::import::PreparedImport::combine("folder-batch".into(), inputs)
        .unwrap()
        .with_folders(folders.clone())
        .unwrap();
    let mut store = pm_kernel::DocumentStore::new_untitled(
        pm_model::json::parse(r#"{"id":"p","layers":[],"w":1,"h":1,"assets":{}}"#).unwrap(),
    );
    store
        .execute(combined.command(), pm_kernel::Origin::Interface)
        .unwrap();
    assert_eq!(store.media().len(), 2);
    assert_eq!(
        store
            .root()
            .unwrap()
            .get("assetFolders")
            .unwrap()
            .to_value(),
        folders
    );
    assert_eq!(store.history().labels(), ["Import"]);
    store.undo().unwrap();
    assert!(store.root().unwrap().get("assetFolders").is_none());
    assert!(
        store
            .root()
            .unwrap()
            .get("assets")
            .unwrap()
            .to_value()
            .as_object()
            .unwrap()
            .is_empty()
    );
    store.redo().unwrap();
    assert_eq!(
        store
            .root()
            .unwrap()
            .get("assetFolders")
            .unwrap()
            .to_value(),
        folders
    );
    fs::remove_file(source).unwrap();
    let owned = root.join("retained");
    store.media().get("a").unwrap().copy_to(&owned).unwrap();
    assert_eq!(fs::read(owned).unwrap(), b"abc");
}

#[test]
fn import_cannot_claim_retained_original_metadata() {
    let t = private_tempdir();
    let source = t.path().canonicalize().unwrap().join("input.bin");
    fs::write(&source, b"abc").unwrap();
    let input = import_input(
        &source,
        "new",
        json!({"originalVersion":"retained"}),
        b"abc",
    );
    assert!(pm_kernel::import::PreparedImport::capture("forged".into(), vec![input]).is_err());
    assert_eq!(fs::read(source).unwrap(), b"abc");
}

#[test]
fn import_respects_retained_original_and_history_id_namespaces_atomically() {
    use pm_kernel::{DocumentStore, Origin, import::PreparedImport};
    let t = private_tempdir();
    let source = t.path().canonicalize().unwrap().join("input.bin");
    fs::write(&source, b"abc").unwrap();
    for section in ["originals", "baselines", "owners"] {
        let mut manifest = json!({"version":1,"originals":{},"baselines":{},"owners":{}});
        manifest[section]["reserved"] = if section == "originals" {
            json!({"sha256":format!("{:x}",Sha256::digest(b"abc")),"length":3,"mime":"application/octet-stream"})
        } else {
            json!({"future":true})
        };
        let project =
            json!({"id":"p","w":1,"h":1,"layers":[],"assets":{},"nativeMediaVersions":manifest});
        let mut store =
            DocumentStore::new_untitled(pm_model::json::parse(&project.to_string()).unwrap());
        let before = store.root().unwrap().to_value();
        let revision = store.revision();
        let prepared = PreparedImport::capture(
            "batch".into(),
            vec![
                import_input(&source, "new", json!({"future":true}), b"abc"),
                import_input(&source, "reserved", json!({}), b"abc"),
            ],
        )
        .unwrap()
        .with_folders(pm_model::json::parse(r#"{"folder":{"id":"folder","future":true}}"#).unwrap())
        .unwrap();
        assert!(
            store
                .execute(prepared.command(), Origin::Interface)
                .is_err(),
            "{section}"
        );
        assert_eq!(store.revision(), revision);
        assert_eq!(store.root().unwrap().to_value(), before);
        assert_eq!(store.media().len(), 0);
    }
}

#[test]
fn same_length_and_revision_import_cannot_replace_bytes_retained_for_redo() {
    use pm_kernel::{DocumentStore, Origin, import::PreparedImport};
    let t = private_tempdir();
    let root = t.path().canonicalize().unwrap();
    let original = root.join("original.bin");
    let different = root.join("different.bin");
    fs::write(&original, b"abc").unwrap();
    fs::write(&different, b"xyz").unwrap();
    let mut store = DocumentStore::new_untitled(
        pm_model::json::parse(r#"{"id":"p","w":1,"h":1,"layers":[],"assets":{}}"#).unwrap(),
    );
    let first = PreparedImport::capture(
        "first".into(),
        vec![import_input(&original, "a", json!({}), b"abc")],
    )
    .unwrap();
    store.execute(first.command(), Origin::Interface).unwrap();
    store.undo().unwrap();
    let before = store.root().unwrap().to_value();
    let revision = store.revision();
    let collision = PreparedImport::capture(
        "collision".into(),
        vec![import_input(&different, "a", json!({}), b"xyz")],
    )
    .unwrap();
    assert!(
        store
            .execute(collision.command(), Origin::Interface)
            .is_err()
    );
    assert_eq!(store.root().unwrap().to_value(), before);
    assert_eq!(store.revision(), revision);
    store.redo().unwrap();
    let recovered = root.join("recovered.bin");
    store.media().get("a").unwrap().copy_to(&recovered).unwrap();
    assert_eq!(fs::read(recovered).unwrap(), b"abc");
}

fn import_input(
    path: &std::path::Path,
    id: &str,
    metadata: serde_json::Value,
    bytes: &[u8],
) -> pm_kernel::import::ImportInput {
    pm_kernel::import::ImportInput {
        asset_id: id.into(),
        asset: pm_model::json::parse(&metadata.to_string()).unwrap(),
        owned_path: path.into(),
        mime: "application/octet-stream".into(),
        revision: "r1".into(),
        size: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

fn private_tempdir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}
