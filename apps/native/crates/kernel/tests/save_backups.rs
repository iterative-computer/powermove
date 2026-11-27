//! Every user save keeps the file it replaces (`ProjectFiles.commit`).
mod common;
use common::*;
use pm_kernel::project_io::{BackupPolicy, list_backups};
use pm_kernel::session::*;
use pm_kernel::*;
use pm_model::container::{ContainerFormat, ReadOptions, read_container};
use pm_model::json::{Map, Value};
use std::path::Path;

fn kernel(root: &Path) -> Kernel {
    Kernel::new(KernelConfig {
        journal_root: Some(root.join("journals")),
        backup_root: Some(root.join("backups")),
        ..Default::default()
    })
}
fn open(k: &mut Kernel, id: &str, path: &Path) -> SessionHandle {
    match k
        .shell(ShellCall::Open {
            id: id.into(),
            source: SessionSource::File(path.into()),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    }
}
fn edit(k: &mut Kernel, id: &str, asset: &str) {
    let mut metadata = Map::new();
    metadata.insert("name", Value::from(asset));
    metadata.insert("kind", Value::from("image"));
    k.handle(
        id,
        Request::new(
            Caller::native(),
            Call::Transact {
                label: Some("Import".into()),
                body: WriteBody::Commands(vec![
                    MediaCommand::AddAsset {
                        asset_id: asset.into(),
                        asset: Value::Object(metadata),
                    }
                    .into(),
                ]),
                expected_revision: None,
            },
        ),
    )
    .unwrap();
}
fn worker_save(k: &mut Kernel, session: SessionHandle, path: Option<std::path::PathBuf>) {
    let plan = k.prepare_session_save(session, path, false).unwrap();
    k.finish_session_save(plan.run()).unwrap();
}
fn backups(root: &Path, id: &str) -> Vec<std::path::PathBuf> {
    list_backups(&BackupPolicy::new(root.join("backups").join(id))).unwrap()
}
fn format(path: &Path) -> ContainerFormat {
    let file = std::fs::File::open(path).unwrap();
    read_container(
        &file,
        ReadOptions {
            verify_media: false,
        },
    )
    .unwrap()
    .format
}

#[test]
fn upgrading_pmv3_and_json_preserves_the_original_bytes_in_a_backup() {
    for (name, expected) in [
        ("pmv3.pmv", ContainerFormat::Pmv3),
        ("pmv3-no-media.pmv", ContainerFormat::Pmv3),
        ("json-plain.pmv", ContainerFormat::Json),
    ] {
        let root = TempDir::new("backups");
        let path = root.path("project.pmv");
        let original = read(&engine_fixture(&format!("containers/{name}")));
        std::fs::write(&path, &original).unwrap();
        assert_eq!(format(&path), expected, "{name}");
        let mut k = kernel(&root.0);
        let session = open(&mut k, "file-a", &path);
        edit(&mut k, "file-a", "first");
        k.shell(ShellCall::Save(session.clone())).unwrap();
        assert_eq!(format(&path), ContainerFormat::Pmv4, "{name}");
        let saved = backups(&root.0, "file-a");
        assert_eq!(saved.len(), 1, "{name}");
        assert_eq!(read(&saved[0]), original, "{name}: the original format");
        // The app's worker route keeps a version too, and the first stays.
        edit(&mut k, "file-a", "second");
        worker_save(&mut k, session, None);
        let saved = backups(&root.0, "file-a");
        assert_eq!(saved.len(), 2, "{name}");
        assert_eq!(read(saved.last().unwrap()), original, "{name}");
    }
}

#[test]
fn a_failed_backup_fails_the_save_and_leaves_the_file_alone() {
    let root = TempDir::new("backups");
    let path = root.path("project.pmv");
    let original = read(&engine_fixture("containers/pmv4.pmv"));
    std::fs::write(&path, &original).unwrap();
    std::fs::write(root.path("backups"), b"not a directory").unwrap();
    let mut k = kernel(&root.0);
    let session = open(&mut k, "file-a", &path);
    edit(&mut k, "file-a", "first");
    let plan = k.prepare_session_save(session, None, false).unwrap();
    assert!(k.finish_session_save(plan.run()).is_err());
    assert_eq!(read(&path), original);
}

#[test]
fn save_as_replaces_an_existing_file_only_with_consent_and_keeps_it() {
    let root = TempDir::new("backups");
    let source = root.path("source.pmv");
    let other = root.path("other.pmv");
    std::fs::write(&source, read(&engine_fixture("containers/pmv4.pmv"))).unwrap();
    std::fs::write(&other, b"someone else's bytes").unwrap();
    let mut k = kernel(&root.0);
    let session = open(&mut k, "file-a", &source);
    edit(&mut k, "file-a", "first");
    assert!(
        k.shell(ShellCall::SaveAs {
            session: session.clone(),
            path: other.clone(),
            replace: false,
        })
        .is_err()
    );
    assert_eq!(read(&other), b"someone else's bytes");
    k.shell(ShellCall::SaveAs {
        session,
        path: other.clone(),
        replace: true,
    })
    .unwrap();
    assert_eq!(format(&other), ContainerFormat::Pmv4);
    let saved = backups(&root.0, "file-a");
    assert_eq!(saved.len(), 1);
    assert_eq!(read(&saved[0]), b"someone else's bytes");
}
