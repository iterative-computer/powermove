//! Synthetic lifecycle fixtures: recovery files remain ordinary durable projects.
mod common;
use pm_kernel::{persistence::DiskFormat, session::*, *};

fn kernel(root: &std::path::Path) -> Kernel {
    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(root.join("journals")),
        backup_root: Some(root.join("backups")),
        ..Default::default()
    });
    k.shell(ShellCall::AttachLibrary {
        profile: root.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
    k
}
fn new(k: &mut Kernel, id: &str) -> SessionHandle {
    let value = pm_model::json::parse(r##"{"name":"Synthetic","compId":"c","w":320,"h":180,"fps":30,"dur":1,"bg":"#000000","layers":[],"assets":{},"comps":{}}"##).unwrap();
    match k
        .shell(ShellCall::Open {
            id: id.into(),
            source: SessionSource::New(value),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    }
}

#[test]
fn file_only_entry_in_recovery_folder_reopens_without_a_journal() {
    let dir = common::TempDir::new("recovery-file-entry");
    let path = dir.path("recovery-files/p.pmv");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut k = kernel(&dir.0);
    let session = new(&mut k, "p");
    k.shell(ShellCall::SaveAs {
        session: session.clone(),
        path: path.clone(),
        replace: false,
    })
    .unwrap();
    k.shell(ShellCall::Close(session)).unwrap();
    k.shell(ShellCall::PrepareQuit).unwrap();
    k.shell(ShellCall::ReleaseLibrary).unwrap();
    drop(k);
    let before = std::fs::read(&path).unwrap();
    let mut k = kernel(&dir.0);
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "p".into(),
            source: SessionSource::Library {
                profile: dir.0.clone(),
                project: "p".into(),
            },
            electron_profile: None,
        })
        .expect("an intact file must not be mistaken for missing recovery work");
    assert!(matches!(reply, ShellReply::Opened { .. }), "{reply:?}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn closing_one_of_two_projects_then_quitting_releases_library() {
    let dir = common::TempDir::new("two-window-quit");
    let mut k = kernel(&dir.0);
    let a = new(&mut k, "a");
    let b = new(&mut k, "b");
    k.shell(ShellCall::Close(a)).unwrap();
    k.shell(ShellCall::DurabilityBarrier).unwrap();
    k.shell(ShellCall::Close(b)).unwrap();
    k.shell(ShellCall::PrepareQuit).unwrap();
    k.shell(ShellCall::ReleaseLibrary).unwrap();
    assert_eq!(k.projects().count(), 0);
}

#[test]
fn recovery_backing_file_does_not_replace_a_newer_library_body() {
    let dir = common::TempDir::new("recovery-with-library-body");
    let path = dir.path("recovery-files/p.pmv");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut k = kernel(&dir.0);
    let session = new(&mut k, "p");
    k.shell(ShellCall::SaveAs {
        session: session.clone(),
        path: path.clone(),
        replace: false,
    })
    .unwrap();
    k.shell(ShellCall::Close(session)).unwrap();
    k.shell(ShellCall::ReleaseLibrary).unwrap();
    drop(k);
    // Older recovery copies deliberately retain a body, with the file used
    // only for its embedded media. The newer body must remain authoritative.
    std::fs::write(dir.path("store/project.p.json"), r##"{"v":"1.2.0","proj":{"id":"p","name":"Newer library body","compId":"c","w":320,"h":180,"fps":30,"dur":1,"bg":"#000000","layers":[],"assets":{},"comps":{}}}"##).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut k = kernel(&dir.0);
    let reply = k
        .shell(ShellCall::PrepareOpen {
            id: "p".into(),
            source: SessionSource::Library {
                profile: dir.0.clone(),
                project: "p".into(),
            },
            electron_profile: None,
        })
        .unwrap();
    assert!(matches!(reply, ShellReply::Opened { .. }), "{reply:?}");
    assert_eq!(
        k.project("p")
            .unwrap()
            .store()
            .snapshot()
            .root()
            .unwrap()
            .get("name")
            .unwrap()
            .to_value(),
        pm_model::Value::from("Newer library body")
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
#[ignore = "diagnostic for an explicitly supplied disposable profile copy"]
fn copied_profile_open_diagnostic() {
    let root = std::path::PathBuf::from(
        std::env::var_os("PM_RECOVERY_PROFILE").expect("disposable profile path"),
    );
    let id = std::env::var("PM_RECOVERY_PROJECT").expect("project id");
    let mut k = kernel(&root);
    let reply = k.shell(ShellCall::PrepareOpen {
        id: id.clone(),
        source: SessionSource::Library {
            profile: root,
            project: id.clone(),
        },
        electron_profile: None,
    });
    eprintln!("Copied project open: {reply:?}");
    assert!(reply.is_ok(), "{reply:?}");
}
