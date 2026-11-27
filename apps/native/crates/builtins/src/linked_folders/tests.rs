use super::*;
use std::fs;

/// A valid one-second 8 kHz mono WAV (synthetic silence).
fn wav(path: &Path) {
    let samples = 8000u32;
    let mut b = vec![];
    b.extend(b"RIFF");
    b.extend((36 + samples * 2).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend((samples * 2).to_le_bytes());
    b.extend(vec![0u8; samples as usize * 2]);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b).unwrap();
}

fn temp() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().join("Footage");
    fs::create_dir_all(&root).unwrap();
    (dir, root)
}

fn ids() -> impl FnMut() -> String {
    let mut n = 0;
    move || {
        n += 1;
        format!("new{n}")
    }
}

/// A project linking `root` as folder `L`, with its tree as the link left it.
fn linked_project(root: &Path) -> Value {
    let tree = Tree::scan(root).unwrap();
    let mut project = json!({"assets": {}, "assetFolders": {
        "L": {"id": "L", "name": "Footage", "parent": null,
              "link": {"path": root.to_string_lossy(), "ino": tree.dirs[root].ino}}}});
    let mut plan = plan(&project, "L", &tree, &mut ids());
    for (id, name, parent, path, ino) in plan.new_folders.drain(..) {
        project["assetFolders"][&id] = json!({"id": id, "name": name, "parent": parent,
            "link": {"path": path.to_string_lossy(), "ino": ino}});
    }
    for (n, (path, folder)) in plan.new_files.iter().enumerate() {
        let s = tree.files[path];
        project["assets"][format!("a{n}")] = json!({"name": base(path), "kind": "audio",
            "folder": folder, "sourcePath": path.to_string_lossy(),
            "sourceFile": {"dev": s.dev, "ino": s.ino}});
    }
    project
}

fn asset<'a>(project: &'a Value, name: &str) -> (&'a String, &'a Value) {
    project["assets"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, a)| a["name"] == name)
        .unwrap()
}

#[test]
fn a_scan_lists_media_and_folders_and_skips_hidden_and_other_files() {
    let (_d, root) = temp();
    wav(&root.join("a.wav"));
    wav(&root.join("Sub/b.wav"));
    wav(&root.join(".hidden/c.wav"));
    fs::write(root.join("notes.txt"), "x").unwrap();
    fs::write(root.join(".DS_Store"), "x").unwrap();
    let tree = Tree::scan(&root).unwrap();
    assert_eq!(
        tree.files.keys().cloned().collect::<Vec<_>>(),
        [root.join("Sub/b.wav"), root.join("a.wav")]
    );
    assert_eq!(tree.dirs.len(), 2, "the root and Sub");
    assert!(Tree::scan(&root.join("gone")).is_none());
}

#[test]
fn refreshing_the_named_directories_matches_a_full_scan() {
    let (_d, root) = temp();
    wav(&root.join("a.wav"));
    wav(&root.join("Sub/b.wav"));
    wav(&root.join("Old/c.wav"));
    let mut tree = Tree::scan(&root).unwrap();
    fs::remove_file(root.join("a.wav")).unwrap();
    wav(&root.join("Sub/new.wav"));
    wav(&root.join("Sub/Deeper/d.wav"));
    fs::remove_dir_all(root.join("Old")).unwrap();
    let events = vec![
        (root.join("a.wav"), 0),
        (root.join("Sub/new.wav"), 0),
        (root.join("Sub/Deeper"), 0),
        (root.join("Old"), 0),
    ];
    tree.refresh(&dirty_dirs(&root, &events).unwrap());
    assert_eq!(tree, Tree::scan(&root).unwrap());
    assert!(
        dirty_dirs(&root, &[(root.clone(), 0x1)]).is_none(),
        "must-scan reads it all"
    );
}

#[test]
fn new_files_and_folders_on_disk_join_the_mirrored_folders() {
    let (_d, root) = temp();
    wav(&root.join("a.wav"));
    let project = linked_project(&root);
    wav(&root.join("b.wav"));
    wav(&root.join("Shots/Day 1/c.wav"));
    let tree = Tree::scan(&root).unwrap();
    let plan = plan(&project, "L", &tree, &mut ids());
    assert_eq!(
        plan.new_folders
            .iter()
            .map(|(id, name, parent, ..)| (id.as_str(), name.as_str(), parent.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("new1", "Shots", Some("L")),
            ("new2", "Day 1", Some("new1"))
        ],
        "parents first"
    );
    assert_eq!(
        plan.new_files,
        [
            (root.join("Shots/Day 1/c.wav"), "new2".to_owned()),
            (root.join("b.wav"), "L".to_owned())
        ]
    );
    assert!(plan.moved_assets.is_empty() && plan.missing.is_empty());
}

#[test]
fn a_deleted_file_goes_missing_and_is_never_removed() {
    let (_d, root) = temp();
    wav(&root.join("a.wav"));
    wav(&root.join("Sub/b.wav"));
    let project = linked_project(&root);
    fs::remove_file(root.join("Sub/b.wav")).unwrap();
    fs::remove_dir(root.join("Sub")).unwrap();
    let plan = plan(&project, "L", &Tree::scan(&root).unwrap(), &mut ids());
    let (b, _) = asset(&project, "b.wav");
    assert_eq!(plan.missing, [b.clone()]);
    assert!(
        plan.is_empty(),
        "nothing to write: the asset just shows as Missing"
    );
    assert!(
        plan.removed_folders.is_empty(),
        "a folder holding a missing file stays"
    );
}

#[test]
fn renamed_and_moved_files_are_followed_by_identity() {
    let (_d, root) = temp();
    wav(&root.join("a.wav"));
    wav(&root.join("Sub/b.wav"));
    let mut project = linked_project(&root);
    // The person renamed b in Powermove: its name is theirs to keep.
    let (b, _) = asset(&project, "b.wav");
    let b = b.clone();
    project["assets"][&b]["name"] = json!("Hero take");
    fs::rename(root.join("a.wav"), root.join("a renamed.wav")).unwrap();
    fs::rename(root.join("Sub/b.wav"), root.join("b.wav")).unwrap();
    let plan = plan(&project, "L", &Tree::scan(&root).unwrap(), &mut ids());
    assert!(
        plan.new_files.is_empty(),
        "no duplicates: {:?}",
        plan.new_files
    );
    assert!(plan.missing.is_empty());
    let moved: BTreeMap<_, _> = plan
        .moved_assets
        .iter()
        .map(|(id, path, _, folder, name)| {
            (id.clone(), (path.clone(), folder.clone(), name.clone()))
        })
        .collect();
    let (a, _) = asset(&project, "a.wav");
    assert_eq!(
        moved[a],
        (
            root.join("a renamed.wav"),
            None,
            Some("a renamed.wav".into())
        ),
        "a file name that followed the disk follows it again"
    );
    assert_eq!(
        moved[&b],
        (root.join("b.wav"), Some("L".into()), None),
        "it moves with its file; the person's name stays"
    );
}

#[test]
fn a_renamed_folder_keeps_its_project_folder_and_a_gone_empty_one_goes() {
    let (_d, root) = temp();
    wav(&root.join("Sub/b.wav"));
    fs::create_dir_all(root.join("Empty")).unwrap();
    let project = linked_project(&root);
    let sub = project["assets"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()["folder"]
        .as_str()
        .unwrap()
        .to_owned();
    let empty = project["assetFolders"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, f)| f["name"] == "Empty")
        .unwrap()
        .0
        .clone();
    fs::rename(root.join("Sub"), root.join("Selects")).unwrap();
    fs::remove_dir(root.join("Empty")).unwrap();
    let plan = plan(&project, "L", &Tree::scan(&root).unwrap(), &mut ids());
    assert!(
        plan.new_folders.is_empty() && plan.new_files.is_empty(),
        "{plan:?}"
    );
    assert_eq!(
        plan.moved_folders,
        [(
            sub,
            root.join("Selects"),
            project["assetFolders"]
                .as_object()
                .unwrap()
                .values()
                .find(|f| f["name"] == "Sub")
                .unwrap()["link"]["ino"]
                .as_u64()
                .unwrap(),
            Some("L".into()),
            Some("Selects".into())
        )]
    );
    assert_eq!(plan.moved_assets.len(), 1, "its file has a new path");
    assert_eq!(plan.removed_folders, [empty]);
}

#[test]
fn media_filed_elsewhere_by_the_person_is_not_added_again() {
    let (_d, root) = temp();
    wav(&root.join("a.wav"));
    let mut project = linked_project(&root);
    let (a, _) = asset(&project, "a.wav");
    let a = a.clone();
    project["assetFolders"]["mine"] = json!({"id": "mine", "name": "Selects", "parent": null});
    project["assets"][&a]["folder"] = json!("mine");
    let plan = plan(&project, "L", &Tree::scan(&root).unwrap(), &mut ids());
    assert!(plan.is_empty(), "{plan:?}");
}

#[cfg(target_os = "macos")]
#[test]
fn the_watch_hears_a_new_file_and_a_rename() {
    let (_d, root) = temp();
    let (tx, rx) = std::sync::mpsc::channel();
    let _watch = watch::Watch::new(&[root.clone()], 0.05, move |events| {
        let _ = tx.send(events);
    })
    .expect("FSEvents stream");
    // FSEvents may report history from just before the stream started.
    std::thread::sleep(std::time::Duration::from_millis(300));
    wav(&root.join("new.wav"));
    let heard = |want: &Path| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(events) = rx.recv_timeout(std::time::Duration::from_millis(200))
                && events.iter().any(|(p, _)| p == want)
            {
                return true;
            }
        }
        false
    };
    assert!(heard(&root.join("new.wav")));
    fs::rename(root.join("new.wav"), root.join("renamed.wav")).unwrap();
    assert!(heard(&root.join("renamed.wav")));
    fs::remove_file(root.join("renamed.wav")).unwrap();
    assert!(heard(&root.join("renamed.wav")));
}

#[test]
fn linking_is_one_undo_step_and_sync_is_none() {
    let (_d, root) = temp();
    wav(&root.join("a.wav"));
    wav(&root.join("Sub/b.wav"));
    let actor = KernelActorForTest::new();
    let panels = Builtins::new(actor.0.handle(), "linked", || {}).unwrap();
    let document = actor
        .0
        .handle()
        .published("linked")
        .unwrap()
        .snapshot
        .doc_id;
    let cancel = AtomicBool::new(false);
    let (names, failed) = panels
        .link_folders(document, std::slice::from_ref(&root), None, &cancel)
        .unwrap();
    assert_eq!((names, failed), (vec!["Footage".to_owned()], vec![]));
    let published = actor.0.handle().published("linked").unwrap();
    assert_eq!(published.history.entries, 1);
    let (_, project) = panels.project_json(document).unwrap();
    assert_eq!(project["assets"].as_object().unwrap().len(), 2);
    let (folders, roots) = linked(&project);
    assert_eq!((folders.len(), roots.len()), (2, 1));
    // Finder adds a file: it appears, without an Undo step.
    wav(&root.join("Sub/c.wav"));
    let tree = Tree::scan(&root).unwrap();
    let done = panels
        .sync_linked(document, &roots[0], &tree, &cancel)
        .unwrap();
    assert_eq!(done.added, 1);
    let published = actor.0.handle().published("linked").unwrap();
    assert_eq!(published.history.entries, 1, "sync is not an Undo step");
    assert!(published.dirty, "but it is saved with the project");
    let (_, project) = panels.project_json(document).unwrap();
    assert_eq!(project["assets"].as_object().unwrap().len(), 3);
    // A second sync has nothing to do.
    let again = panels
        .sync_linked(document, &roots[0], &tree, &cancel)
        .unwrap();
    assert_eq!(again.added, 0);
    // Undo takes back the person's link, not Finder's file.
    actor
        .0
        .handle()
        .call(
            "linked",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    let (_, project) = panels.project_json(document).unwrap();
    let names: Vec<_> = project["assets"]
        .as_object()
        .unwrap()
        .values()
        .map(|a| a["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        names,
        ["c.wav"],
        "the synced file was never part of the link step"
    );
    panels.close();
}

struct KernelActorForTest(pm_kernel::KernelActor);
impl KernelActorForTest {
    fn new() -> Self {
        let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
        let project = pm_model::json::parse(
            r##"{"id":"linked","name":"Linked","compId":"c","w":64,"h":36,"fps":30,"dur":1,"bg":"#000000","assets":{},"comps":{},"layers":[]}"##,
        )
        .unwrap();
        actor
            .handle()
            .open("linked", pm_kernel::OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        Self(actor)
    }
}

#[test]
fn sync_waits_for_an_open_edit_and_keeps_its_history_clean() {
    let (_d, root) = temp();
    let actor = KernelActorForTest::new();
    let panels = std::sync::Arc::new(Builtins::new(actor.0.handle(), "linked", || {}).unwrap());
    let h = actor.0.handle();
    let document = h.published("linked").unwrap().snapshot.doc_id;
    panels
        .link_folders(document, &[root.clone()], None, &AtomicBool::new(false))
        .unwrap();
    let (_, project) = panels.project_json(document).unwrap();
    let root_id = roots(&project)[0].0.clone();
    h.call(
        "linked",
        Request::new(
            Caller::native(),
            pm_kernel::HistoryCall::Begin {
                label: "Person edit".into(),
            },
        ),
    )
    .wait()
    .unwrap()
    .unwrap();
    wav(&root.join("new.wav"));
    let tree = Tree::scan(&root).unwrap();
    let p = panels.clone();
    let worker = std::thread::spawn(move || {
        p.sync_linked(document, &root_id, &tree, &AtomicBool::new(false))
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        panels.project_json(document).unwrap().1["assets"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    h.call(
        "linked",
        Request::new(
            Caller::native(),
            pm_kernel::HistoryCall::Commit { label: None },
        ),
    )
    .wait()
    .unwrap()
    .unwrap();
    assert_eq!(worker.join().unwrap().unwrap().added, 1);
    assert_eq!(h.published("linked").unwrap().history.entries, 1);
    panels.close();
}

#[test]
fn same_inode_on_another_device_does_not_claim_a_renamed_directory() {
    let (_d, root) = temp();
    fs::create_dir(root.join("Sub")).unwrap();
    let mut project = linked_project(&root);
    let folder = project["assetFolders"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .find(|f| f["name"] == "Sub")
        .unwrap();
    folder["link"]["dev"] = json!(u64::MAX);
    fs::rename(root.join("Sub"), root.join("Other")).unwrap();
    let plan = plan(&project, "L", &Tree::scan(&root).unwrap(), &mut ids());
    assert!(plan.moved_folders.is_empty());
    assert_eq!(plan.new_folders.len(), 1);
}

#[test]
fn moving_a_linked_root_follows_its_files_and_preserves_display_names() {
    let (dir, root) = temp();
    wav(&root.join("a.wav"));
    let mut project = linked_project(&root);
    project["assetFolders"]["L"]["name"] = json!("My selects");
    let moved = dir.path().canonicalize().unwrap().join("Renamed");
    fs::rename(&root, &moved).unwrap();
    let plan = plan(&project, "L", &Tree::scan(&moved).unwrap(), &mut ids());
    assert_eq!(plan.moved_folders.len(), 1);
    assert_eq!(plan.moved_folders[0].0, "L");
    assert_eq!(plan.moved_folders[0].4, None);
    assert_eq!(plan.moved_assets.len(), 1);
    assert_eq!(plan.moved_assets[0].1, moved.join("a.wav"));
    assert!(plan.new_files.is_empty());
}

#[test]
fn a_file_moved_between_linked_roots_keeps_its_asset() {
    let (_d, root) = temp();
    wav(&root.join("a.wav"));
    let mut project = linked_project(&root);
    let other = root.parent().unwrap().join("Other");
    fs::create_dir(&other).unwrap();
    let tree = Tree::scan(&other).unwrap();
    project["assetFolders"]["R"] = json!({"id":"R","name":"Other","parent":null,"link":{"path":other,"ino":tree.dirs[&other].ino}});
    fs::rename(root.join("a.wav"), other.join("a.wav")).unwrap();
    let plan = plan(&project, "R", &Tree::scan(&other).unwrap(), &mut ids());
    assert!(plan.new_files.is_empty());
    assert_eq!(plan.moved_assets.len(), 1);
    assert_eq!(plan.moved_assets[0].3.as_deref(), Some("R"));
}

#[test]
fn a_folder_moved_between_linked_roots_keeps_its_folder_and_assets() {
    let (_d, root) = temp();
    wav(&root.join("Sub/a.wav"));
    let mut project = linked_project(&root);
    let (asset_id, asset) = asset(&project, "a.wav");
    let asset_id = asset_id.clone();
    let folder_id = asset["folder"].as_str().unwrap().to_owned();
    let other = root.parent().unwrap().join("Other");
    fs::create_dir(&other).unwrap();
    let tree = Tree::scan(&other).unwrap();
    project["assetFolders"]["R"] = json!({"id":"R","name":"Other","parent":null,"link":{"path":other,"ino":tree.dirs[&other].ino}});
    fs::rename(root.join("Sub"), other.join("Sub")).unwrap();
    let plan = plan(&project, "R", &Tree::scan(&other).unwrap(), &mut ids());
    assert!(plan.new_files.is_empty() && plan.new_folders.is_empty());
    assert_eq!(plan.moved_folders[0].0, folder_id);
    assert_eq!(plan.moved_folders[0].3.as_deref(), Some("R"));
    assert_eq!(plan.moved_assets[0].0, asset_id);
}

#[test]
fn files_that_exchange_names_follow_their_inodes() {
    let (_d, root) = temp();
    wav(&root.join("a.wav"));
    wav(&root.join("b.wav"));
    let project = linked_project(&root);
    let a = asset(&project, "a.wav").0.clone();
    let b = asset(&project, "b.wav").0.clone();
    fs::rename(root.join("a.wav"), root.join("temp.wav")).unwrap();
    fs::rename(root.join("b.wav"), root.join("a.wav")).unwrap();
    fs::rename(root.join("temp.wav"), root.join("b.wav")).unwrap();
    let plan = plan(&project, "L", &Tree::scan(&root).unwrap(), &mut ids());
    assert!(plan.new_files.is_empty() && plan.missing.is_empty());
    let paths: BTreeMap<_, _> = plan
        .moved_assets
        .iter()
        .map(|(id, path, ..)| (id.clone(), path.clone()))
        .collect();
    assert_eq!(paths[&a], root.join("b.wav"));
    assert_eq!(paths[&b], root.join("a.wav"));
}
