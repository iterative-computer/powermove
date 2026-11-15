//! Linked folders: a Finder folder mirrored in the Project panel.
//!
//! Import Folder… (or a folder dropped on the panel) links the folder: it
//! becomes a Project folder whose `link` names the folder on disk, its
//! subfolders become linked folders inside it, and every media file becomes
//! an asset that reads the file in place (`sourcePath`, no retained copy:
//! `media::for_session` resolves such an asset by its path). That first
//! import is the person's edit and one Undo step.
//!
//! While the project is open the folders are watched (FSEvents, [`watch`])
//! and synced one way, Finder → Powermove ([`plan`] + [`Builtins::sync_linked`]):
//! - a new file becomes an asset in the folder mirroring its directory;
//! - a deleted file keeps its asset, which shows as Missing; compositions
//!   using it keep their layers;
//! - a renamed or moved file (or folder) is followed by its identity
//!   (device and inode, recorded as `sourceFile`/`link.ino`).
//!
//! Sync writes outside Undo history (`Call::DirectWrite`): the files really
//! are there, so ⌘Z must not take them away. The person's own edits stay
//! undoable. Renaming a linked folder in Powermove changes only its name in
//! the project; nothing on disk is ever renamed, moved or deleted here.
use crate::Builtins;
use pm_kernel::{Call, Caller, Reply, Request};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// A file's identity and version on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
}

fn stat(metadata: &std::fs::Metadata) -> Stat {
    use std::os::unix::fs::MetadataExt;
    Stat {
        dev: metadata.dev(),
        ino: metadata.ino(),
        len: metadata.len(),
    }
}

/// What a linked folder holds on disk: its directories and media files,
/// hidden entries left out. Kept between syncs so an event re-reads only
/// the directories it names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    pub root: PathBuf,
    /// Every directory, the root included, with its identity.
    pub dirs: BTreeMap<PathBuf, Stat>,
    pub files: BTreeMap<PathBuf, Stat>,
}

fn hidden(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

fn media(path: &Path) -> bool {
    crate::media_import::classify(path).is_some()
}

impl Tree {
    /// Reads the whole folder. `None`: the folder isn't there (moved,
    /// deleted, or its volume is offline).
    pub fn scan(root: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(root).ok().filter(|m| m.is_dir())?;
        let mut tree = Tree {
            root: root.to_path_buf(),
            ..Default::default()
        };
        tree.dirs.insert(root.to_path_buf(), stat(&metadata));
        tree.walk(root);
        Some(tree)
    }

    fn walk(&mut self, dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if hidden(&path) {
                continue;
            }
            // Symbolic links are not followed: a link to a parent would never end.
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() {
                self.dirs.insert(path.clone(), stat(&metadata));
                self.walk(&path);
            } else if metadata.is_file() && media(&path) {
                self.files.insert(path, stat(&metadata));
            }
        }
    }

    /// Re-reads `dirs` (one level each; a new subdirectory is read whole and
    /// a gone one drops its subtree). Cheap for a large folder: only what an
    /// event named is looked at again.
    pub fn refresh(&mut self, dirs: &BTreeSet<PathBuf>) {
        for dir in dirs {
            if !dir.starts_with(&self.root) {
                continue;
            }
            match std::fs::symlink_metadata(dir).ok().filter(|m| m.is_dir()) {
                None => {
                    self.dirs.retain(|p, _| !p.starts_with(dir));
                    self.files.retain(|p, _| !p.starts_with(dir));
                }
                Some(metadata) => {
                    self.dirs.insert(dir.clone(), stat(&metadata));
                    // Direct children are read again below.
                    let parent_is = |p: &Path| p.parent() == Some(dir.as_path());
                    self.files.retain(|p, _| !parent_is(p));
                    let gone: Vec<PathBuf> = self
                        .dirs
                        .keys()
                        .filter(|p| {
                            parent_is(p) && !std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
                        })
                        .cloned()
                        .collect();
                    for g in gone {
                        self.dirs.retain(|p, _| !p.starts_with(&g));
                        self.files.retain(|p, _| !p.starts_with(&g));
                    }
                    let Ok(entries) = std::fs::read_dir(dir) else {
                        continue;
                    };
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if hidden(&path) {
                            continue;
                        }
                        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                            continue;
                        };
                        if metadata.is_dir() {
                            if !self.dirs.contains_key(&path) {
                                self.dirs.insert(path.clone(), stat(&metadata));
                                self.walk(&path);
                            }
                        } else if metadata.is_file() && media(&path) {
                            self.files.insert(path, stat(&metadata));
                        }
                    }
                }
            }
        }
    }
}

/// A linked folder as the project records it.
#[derive(Clone, Debug, PartialEq)]
pub struct Linked {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub path: PathBuf,
    pub ino: u64,
    pub dev: u64,
}

/// The project's linked folders, and which of them are linked roots (a
/// linked folder not inside another linked folder).
pub fn linked(project: &Value) -> (BTreeMap<String, Linked>, Vec<String>) {
    let mut out = BTreeMap::new();
    if let Some(folders) = project.get("assetFolders").and_then(Value::as_object) {
        for (id, f) in folders {
            let Some(path) = f["link"]["path"].as_str().filter(|p| !p.is_empty()) else {
                continue;
            };
            out.insert(
                id.clone(),
                Linked {
                    id: id.clone(),
                    name: f["name"].as_str().unwrap_or_default().to_owned(),
                    parent: f["parent"].as_str().map(str::to_owned),
                    path: PathBuf::from(path),
                    ino: f["link"]["ino"].as_u64().unwrap_or(0),
                    dev: f["link"]["dev"].as_u64().unwrap_or(0),
                },
            );
        }
    }
    let roots = out
        .values()
        .filter(|f| f.parent.as_ref().is_none_or(|p| !out.contains_key(p)))
        .map(|f| f.id.clone())
        .collect();
    (out, roots)
}

/// The folders on disk of every linked root: what to watch.
pub fn roots(project: &Value) -> Vec<(String, PathBuf)> {
    let (folders, roots) = linked(project);
    roots
        .into_iter()
        .map(|id| (id.clone(), folders[&id].path.clone()))
        .collect()
}

/// What a sync changes in the project.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// New linked folders, parents first: (id, name, parent, path, ino).
    pub new_folders: Vec<(String, String, Option<String>, PathBuf, u64)>,
    /// Linked folders found elsewhere (renamed or moved on disk):
    /// (id, new path, ino, new parent, new name when it followed the disk).
    pub moved_folders: Vec<(String, PathBuf, u64, Option<String>, Option<String>)>,
    /// Linked folders whose directory is gone and that hold nothing now.
    pub removed_folders: Vec<String>,
    /// Files to add as assets, in the folder mirroring their directory.
    pub new_files: Vec<(PathBuf, String)>,
    /// Assets whose file was renamed or moved: (asset, new path, identity,
    /// new folder when it followed the file, new name when it followed the file).
    pub moved_assets: Vec<(String, PathBuf, Stat, Option<String>, Option<String>)>,
    /// Assets whose file is gone: they stay, and show as Missing.
    pub missing: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.new_folders.is_empty()
            && self.moved_folders.is_empty()
            && self.removed_folders.is_empty()
            && self.new_files.is_empty()
            && self.moved_assets.is_empty()
    }
}

fn base(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Reconciles the linked root `root` with what is on disk. Pure: reads only
/// `project` and `tree`. `next_id` names new folders.
pub fn plan(project: &Value, root: &str, tree: &Tree, next_id: &mut dyn FnMut() -> String) -> Plan {
    let (folders, _) = linked(project);
    let mut plan = Plan::default();
    let Some(top) = folders.get(root) else {
        return plan;
    };
    if tree.root != top.path {
        let name = (top.name == base(&top.path)).then(|| base(&tree.root));
        if let Some(stat) = tree.dirs.get(&tree.root) {
            plan.moved_folders.push((
                root.into(),
                tree.root.clone(),
                stat.ino,
                top.parent.clone(),
                name,
            ));
        }
    }
    // The linked folders of this root.
    let within = |id: &str| {
        let mut at = Some(id.to_owned());
        let mut guard = 0;
        while let Some(id) = at {
            if id == root {
                return true;
            }
            guard += 1;
            if guard > 4096 {
                return false;
            }
            at = folders.get(&id).and_then(|f| f.parent.clone());
        }
        false
    };
    let directory_ids: BTreeSet<_> = tree.dirs.values().map(|s| (s.dev, s.ino)).collect();
    let directory_inodes: BTreeSet<_> = tree.dirs.values().map(|s| s.ino).collect();
    let mine: BTreeMap<String, Linked> = folders
        .iter()
        .filter(|(id, f)| {
            within(id)
                || (f.ino != 0
                    && (directory_ids.contains(&(f.dev, f.ino))
                        || (f.dev == 0 && directory_inodes.contains(&f.ino))))
        })
        .map(|(id, f)| (id.clone(), f.clone()))
        .collect();
    // Directory → folder, from the recorded paths; then follow moved folders by inode.
    let mut dir_folder: HashMap<PathBuf, String> = HashMap::new();
    dir_folder.insert(tree.root.clone(), root.to_owned());
    for f in mine.values() {
        if f.id != root
            && tree
                .dirs
                .get(&f.path)
                .is_some_and(|s| f.ino == s.ino && (f.dev == 0 || f.dev == s.dev))
        {
            dir_folder.insert(f.path.clone(), f.id.clone());
        }
    }
    let lost: Vec<&Linked> = mine
        .values()
        .filter(|f| f.id != root && !dir_folder.values().any(|id| id == &f.id))
        .collect();
    let by_ino: HashMap<u64, &PathBuf> = tree
        .dirs
        .iter()
        .filter(|(p, _)| !dir_folder.contains_key(*p))
        .map(|(p, s)| (s.ino, p))
        .collect();
    let mut gone_folders = vec![];
    let mut moved_dirs: Vec<(String, PathBuf, u64)> = vec![];
    for f in lost {
        match by_ino.get(&f.ino).filter(|_| f.ino != 0) {
            Some(path) if f.dev == 0 || tree.dirs[*path].dev == f.dev => {
                dir_folder.insert((*path).clone(), f.id.clone());
                moved_dirs.push((f.id.clone(), (*path).clone(), f.ino));
            }
            _ => gone_folders.push(f.id.clone()),
        }
    }
    // New directories, parents first (BTreeMap order puts a parent first).
    for (dir, s) in &tree.dirs {
        if dir_folder.contains_key(dir) {
            continue;
        }
        let parent = dir.parent().and_then(|p| dir_folder.get(p)).cloned();
        let Some(parent) = parent else { continue };
        let id = next_id();
        dir_folder.insert(dir.clone(), id.clone());
        plan.new_folders
            .push((id, base(dir), Some(parent), dir.clone(), s.ino));
    }
    for (id, path, ino) in moved_dirs {
        let f = &mine[&id];
        let parent = path.parent().and_then(|p| dir_folder.get(p)).cloned();
        let name = (f.name == base(&f.path) && base(&path) != f.name).then(|| base(&path));
        plan.moved_folders.push((id, path, ino, parent, name));
    }
    // Assets: every asset reading a file under this root, wherever the
    // person filed it, claims its file; the rest are new.
    let identities: BTreeSet<_> = tree.files.values().map(|s| (s.dev, s.ino)).collect();
    let assets = project.get("assets").and_then(Value::as_object);
    let mut claimed: BTreeSet<PathBuf> = BTreeSet::new();
    let mut lost_assets = vec![];
    let mut holds: HashMap<String, usize> = HashMap::new();
    for (id, a) in assets.into_iter().flatten() {
        let folder = a["folder"].as_str().map(str::to_owned);
        if let Some(f) = &folder {
            *holds.entry(f.clone()).or_default() += 1;
        }
        let Some(path) = a["sourcePath"].as_str().map(PathBuf::from) else {
            continue;
        };
        let in_tree = folder.as_ref().is_some_and(|f| mine.contains_key(f));
        let identity = (
            a["sourceFile"]["dev"].as_u64().unwrap_or(0),
            a["sourceFile"]["ino"].as_u64().unwrap_or(0),
        );
        if !path.starts_with(&tree.root)
            && !(in_tree && a.get("sourceFile").is_some())
            && !identities.contains(&identity)
        {
            continue;
        }
        if let Some(s) = tree
            .files
            .get(&path)
            .filter(|s| (s.dev, s.ino) == identity || !identities.contains(&identity))
        {
            if a["sourceFile"]["ino"].as_u64() != Some(s.ino)
                || a["sourceFile"]["dev"].as_u64() != Some(s.dev)
            {
                plan.moved_assets
                    .push((id.clone(), path.clone(), *s, None, None));
            }
            claimed.insert(path);
        } else {
            lost_assets.push((id.clone(), a, path, folder));
        }
    }
    let unclaimed_by_ino: HashMap<(u64, u64), PathBuf> = tree
        .files
        .iter()
        .filter(|(p, _)| !claimed.contains(*p))
        .map(|(p, s)| ((s.dev, s.ino), p.clone()))
        .collect();
    for (id, a, old, folder) in lost_assets {
        let identity = (
            a["sourceFile"]["dev"].as_u64().unwrap_or(0),
            a["sourceFile"]["ino"].as_u64().unwrap_or(0),
        );
        match unclaimed_by_ino.get(&identity).filter(|_| identity.1 != 0) {
            Some(path) if !claimed.contains(path) => {
                claimed.insert(path.clone());
                let s = tree.files[path];
                // The asset follows its file's folder only when it was in
                // the folder mirroring the file's old directory.
                let old_folder = old.parent().and_then(|p| {
                    folders
                        .values()
                        .find(|f| f.path == p)
                        .map(|f| f.id.clone())
                        .or_else(|| (p == top.path).then(|| root.to_owned()))
                });
                let new_folder = path.parent().and_then(|p| dir_folder.get(p)).cloned();
                let follow = (folder.is_some() && folder == old_folder && new_folder != folder)
                    .then_some(new_folder)
                    .flatten();
                let name = a["name"].as_str().unwrap_or_default();
                let rename = (name == base(&old) && base(path) != name).then(|| base(path));
                if let Some(f) = &folder
                    && follow.is_some()
                    && let Some(n) = holds.get_mut(f)
                {
                    *n -= 1;
                }
                plan.moved_assets
                    .push((id.clone(), path.clone(), s, follow, rename));
            }
            _ => plan.missing.push(id.clone()),
        }
    }
    for path in tree.files.keys() {
        if claimed.contains(path) {
            continue;
        }
        if let Some(folder) = path.parent().and_then(|p| dir_folder.get(p)) {
            plan.new_files.push((path.clone(), folder.clone()));
        }
    }
    // A linked folder whose directory is gone goes when nothing is left in
    // it: no asset (a missing file keeps it) and no folder.
    let mut removed: BTreeSet<String> = BTreeSet::new();
    loop {
        let before = removed.len();
        for id in &gone_folders {
            if removed.contains(id) {
                continue;
            }
            let has_assets = holds.get(id).copied().unwrap_or(0) > 0;
            let has_folders = folders.values().any(|f| {
                let parent = plan
                    .moved_folders
                    .iter()
                    .find(|(fid, ..)| fid == &f.id)
                    .map(|(_, _, _, parent, _)| parent.as_deref())
                    .unwrap_or(f.parent.as_deref());
                parent == Some(id.as_str()) && !removed.contains(&f.id)
            }) || project
                .get("assetFolders")
                .and_then(Value::as_object)
                .is_some_and(|all| {
                    all.iter().any(|(fid, f)| {
                        f["parent"].as_str() == Some(id) && !folders.contains_key(fid)
                    })
                });
            if !has_assets && !has_folders {
                removed.insert(id.clone());
            }
        }
        if removed.len() == before {
            break;
        }
    }
    plan.removed_folders = removed.into_iter().collect();
    plan
}

/// An asset record for a linked file: what the panel and layers read, the
/// path it is read from in place, and its identity for following renames.
pub fn asset_record(path: &Path, folder: &str, s: Stat) -> Result<Value, String> {
    let mut asset = crate::media_import::probe(path)?;
    asset["folder"] = json!(folder);
    asset["sourceFile"] = json!({"dev": s.dev, "ino": s.ino});
    Ok(asset)
}

/// The sync state of a linked root, as the panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncState {
    /// Watched; the panel matches the folder.
    Synced,
    /// A change is being read.
    Syncing,
    /// The folder isn't where it was (moved, deleted, or its disk is gone).
    NotFound,
}

/// What a sync did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Synced {
    pub failed: Vec<(String, String)>,
    pub added: usize,
    pub moved: usize,
    pub missing: usize,
    pub folders: usize,
}

const ATTEMPTS: usize = 40;
const WAIT: std::time::Duration = std::time::Duration::from_millis(100);

fn path_of(parts: &[&str]) -> Vec<pm_model::PathPart> {
    parts
        .iter()
        .map(|p| pm_model::PathPart::Key((*p).into()))
        .collect()
}

impl Builtins {
    fn project_json(
        &self,
        document: u64,
    ) -> Result<(std::sync::Arc<pm_kernel::Published>, Value), String> {
        let published = self
            .broker
            .kernel
            .published(&self.broker.project)
            .ok_or("The project closed")?;
        if published.snapshot.doc_id != document {
            return Err("Another project is open".into());
        }
        let root = published
            .snapshot
            .document
            .to_value()
            .ok_or("The project could not be read")?;
        let wire = crate::data::wire(&root);
        Ok((published, wire))
    }

    /// Links folders on disk into the project: Import Folder… and folders
    /// dropped on the panel. One Undo step. Blocks for probing; run it on a
    /// worker. Answers the names of the folders linked and files refused.
    pub fn link_folders(
        &self,
        document: u64,
        paths: &[PathBuf],
        into: Option<String>,
        cancel: &AtomicBool,
    ) -> Result<(Vec<String>, Vec<(String, String)>), String> {
        let mut folders: Vec<Value> = vec![];
        let mut assets: Vec<(String, Value)> = vec![];
        let mut failed = vec![];
        let mut names = vec![];
        for root in paths {
            let root = root.canonicalize().map_err(|e| e.to_string())?;
            let tree = Tree::scan(&root).ok_or("The folder can't be read")?;
            let mut dir_id: HashMap<PathBuf, String> = HashMap::new();
            for (dir, s) in &tree.dirs {
                if cancel.load(Ordering::Acquire) {
                    return Err("Cancelled".into());
                }
                let id = pm_kernel::new_id("f");
                let parent = if *dir == root {
                    into.clone()
                } else {
                    dir.parent().and_then(|p| dir_id.get(p)).cloned()
                };
                dir_id.insert(dir.clone(), id.clone());
                folders.push(json!({"id": id, "name": base(dir), "parent": parent,
                    "link": {"path": dir.to_string_lossy(), "ino": s.ino, "dev": s.dev}}));
            }
            names.push(base(&root));
            for (file, s) in &tree.files {
                if cancel.load(Ordering::Acquire) {
                    return Err("Cancelled".into());
                }
                let folder = file
                    .parent()
                    .and_then(|p| dir_id.get(p))
                    .cloned()
                    .unwrap_or_default();
                match asset_record(file, &folder, *s) {
                    Ok(record) => assets.push((pm_kernel::new_id("a"), record)),
                    Err(error) => failed.push((base(file), error)),
                }
            }
        }
        let label = if names.len() == 1 {
            format!("Link Folder “{}”", names[0])
        } else {
            "Link Folders".into()
        };
        for attempt in 0..ATTEMPTS {
            let (published, mut project) = self.project_json(document)?;
            if published.entry.is_some() {
                std::thread::sleep(WAIT);
                continue;
            }
            if let Some(into) = &into
                && project["assetFolders"].get(into).is_none()
            {
                return Err("The folder to import into is gone".into());
            }
            let object = project
                .as_object_mut()
                .ok_or("The project could not be read")?;
            let all = object.entry("assetFolders").or_insert_with(|| json!({}));
            for f in &folders {
                all[f["id"].as_str().unwrap()] = f.clone();
            }
            let all = object.entry("assets").or_insert_with(|| json!({}));
            for (id, a) in &assets {
                let mut a = a.clone();
                a["id"] = json!(id);
                all[id.as_str()] = a;
            }
            let reply = self
                .broker
                .kernel
                .call(
                    &self.broker.project,
                    Request::new(
                        Caller::native(),
                        Call::Restore {
                            project: crate::data::model(&project)?,
                            label: label.clone(),
                            expected_revision: published.revision(),
                            expected_document: document,
                        },
                    ),
                )
                .wait()
                .ok_or("The project closed")?;
            match reply.map(|ack| ack.reply) {
                Ok(Reply::Throw(_)) | Err(_) if attempt + 1 < ATTEMPTS => std::thread::sleep(WAIT),
                Ok(Reply::Throw(error)) => return Err(error.to_string()),
                Err(error) => return Err(error.to_string()),
                Ok(_) => return Ok((names, failed)),
            }
        }
        Err("The project is busy; link the folder again".into())
    }

    /// Syncs the linked root `root` with `tree`, outside Undo history.
    /// Blocks for probing new files; run it on a worker.
    pub fn sync_linked(
        &self,
        document: u64,
        root: &str,
        tree: &Tree,
        cancel: &AtomicBool,
    ) -> Result<Synced, String> {
        for _ in 0..ATTEMPTS {
            let (published, project) = self.project_json(document)?;
            // Never fold into the person's open edit (it would become part
            // of their Undo step): wait for it to close.
            if published.entry.is_some() {
                std::thread::sleep(WAIT);
                continue;
            }
            let plan = plan(&project, root, tree, &mut || pm_kernel::new_id("f"));
            let mut done = Synced {
                missing: plan.missing.len(),
                ..Default::default()
            };
            if plan.is_empty() {
                return Ok(done);
            }
            let mut ops: Vec<pm_model::Op> = vec![];
            let mut set = |parts: &[&str], value: &Value| -> Result<(), String> {
                ops.push(pm_model::Op::Leaf(pm_model::Patch::set(
                    path_of(parts),
                    crate::data::model(value)?,
                )));
                Ok(())
            };
            for (id, name, parent, path, ino) in &plan.new_folders {
                set(
                    &["assetFolders", id],
                    &json!({"id": id, "name": name, "parent": parent,
                        "link": {"path": path.to_string_lossy(), "ino": ino, "dev": tree.dirs[path].dev}}),
                )?;
                done.folders += 1;
            }
            for (id, path, ino, parent, name) in &plan.moved_folders {
                set(
                    &["assetFolders", id, "link"],
                    &json!({"path": path.to_string_lossy(), "ino": ino, "dev": tree.dirs[path].dev}),
                )?;
                if let Some(parent) = parent {
                    set(&["assetFolders", id, "parent"], &json!(parent))?;
                }
                if let Some(name) = name {
                    set(&["assetFolders", id, "name"], &json!(name))?;
                }
                done.folders += 1;
            }
            for (id, path, s, folder, name) in &plan.moved_assets {
                set(
                    &["assets", id, "sourcePath"],
                    &json!(path.to_string_lossy()),
                )?;
                set(
                    &["assets", id, "sourceFile"],
                    &json!({"dev": s.dev, "ino": s.ino}),
                )?;
                if let Some(folder) = folder {
                    set(&["assets", id, "folder"], &json!(folder))?;
                }
                if let Some(name) = name {
                    set(&["assets", id, "name"], &json!(name))?;
                }
                done.moved += 1;
            }
            for (path, folder) in &plan.new_files {
                if cancel.load(Ordering::Acquire) {
                    return Err("Cancelled".into());
                }
                // A file still being written fails to probe; the next event
                // (its last write) brings it.
                let mut record = match asset_record(path, folder, tree.files[path]) {
                    Ok(record) => record,
                    Err(error) => {
                        if crate::media_import::classify(path).is_some_and(|(kind, _)| kind == "model") {
                            done.failed.push((base(path), error));
                        }
                        continue;
                    }
                };
                let id = pm_kernel::new_id("a");
                record["id"] = json!(id);
                set(&["assets", &id], &record)?;
                done.added += 1;
            }
            for id in &plan.removed_folders {
                ops.push(pm_model::Op::Leaf(pm_model::Patch::delete(path_of(&[
                    "assetFolders",
                    id,
                ]))));
            }
            if ops.is_empty() {
                return Ok(done);
            }
            let reply = self
                .broker
                .kernel
                .call(
                    &self.broker.project,
                    Request::new(Caller::native(), Call::DirectWrite { ops })
                        .with_document(document)
                        .outside_revision(published.revision()),
                )
                .wait()
                .ok_or("The project closed")?;
            match reply.map(|ack| ack.reply) {
                Ok(Reply::Throw(_)) | Err(_) => std::thread::sleep(WAIT),
                Ok(_) => return Ok(done),
            }
        }
        Err("The project stayed busy".into())
    }
}

#[cfg(target_os = "macos")]
pub mod watch {
    //! FSEvents on a folder tree, delivered on a private dispatch queue.
    use std::ffi::{CStr, c_char, c_void};
    use std::path::PathBuf;

    type Ref = *mut c_void;
    #[repr(C)]
    struct Context {
        version: isize,
        info: *mut c_void,
        retain: *const c_void,
        release: *const c_void,
        copy_description: *const c_void,
    }
    type Callback = extern "C" fn(Ref, *mut c_void, usize, *mut c_void, *const u32, *const u64);
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeArrayCallBacks: c_void;
        fn CFStringCreateWithBytes(a: Ref, b: *const u8, n: isize, enc: u32, ext: u8) -> Ref;
        fn CFArrayCreate(a: Ref, v: *const Ref, n: isize, cb: *const c_void) -> Ref;
        fn CFRelease(r: Ref);
    }
    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn FSEventStreamCreate(
            a: Ref,
            cb: Callback,
            ctx: *mut Context,
            paths: Ref,
            since: u64,
            latency: f64,
            flags: u32,
        ) -> Ref;
        fn FSEventStreamSetDispatchQueue(s: Ref, q: Ref);
        fn FSEventStreamStart(s: Ref) -> u8;
        fn FSEventStreamStop(s: Ref);
        fn FSEventStreamInvalidate(s: Ref);
        fn FSEventStreamRelease(s: Ref);
    }
    unsafe extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: Ref) -> Ref;
        fn dispatch_release(q: Ref);
        fn dispatch_sync_f(q: Ref, context: Ref, work: extern "C" fn(Ref));
    }
    const UTF8: u32 = 0x0800_0100;
    const SINCE_NOW: u64 = u64::MAX;
    const NO_DEFER: u32 = 0x2;
    const WATCH_ROOT: u32 = 0x4;
    const FILE_EVENTS: u32 = 0x10;
    /// The event's subtree must be read again (events were coalesced or dropped).
    pub const MUST_SCAN: u32 = 0x1 | 0x2 | 0x4;
    /// The watched folder itself moved or went.
    pub const ROOT_CHANGED: u32 = 0x20;

    /// One change: the path and its FSEvents flags.
    pub type Event = (PathBuf, u32);
    type Sink = Box<dyn Fn(Vec<Event>) + Send>;

    /// Watches folders until dropped. Events arrive on the stream's own
    /// queue, never the UI thread, `latency` seconds after the first of a
    /// burst (FSEvents coalesces them).
    pub struct Watch {
        stream: Ref,
        queue: Ref,
        sink: *mut Sink,
    }
    unsafe impl Send for Watch {}

    extern "C" fn deliver(
        _: Ref,
        info: *mut c_void,
        count: usize,
        paths: *mut c_void,
        flags: *const u32,
        _: *const u64,
    ) {
        // SAFETY: FSEvents passes `count` C strings and flags (no CF types
        // flag); `info` is the sink this watch owns until it is released.
        let events = unsafe {
            let paths = std::slice::from_raw_parts(paths as *const *const c_char, count);
            let flags = std::slice::from_raw_parts(flags, count);
            paths
                .iter()
                .zip(flags)
                .map(|(p, f)| {
                    (
                        PathBuf::from(CStr::from_ptr(*p).to_string_lossy().into_owned()),
                        *f,
                    )
                })
                .collect()
        };
        let sink = unsafe { &*(info as *const Sink) };
        sink(events);
    }

    impl Watch {
        pub fn new(
            paths: &[PathBuf],
            latency: f64,
            sink: impl Fn(Vec<Event>) + Send + 'static,
        ) -> Option<Self> {
            if paths.is_empty() {
                return None;
            }
            // SAFETY: plain CoreFoundation/FSEvents calls; every created
            // object is released here or in Drop.
            unsafe {
                let strings: Vec<Ref> = paths
                    .iter()
                    .map(|p| {
                        let s = p.to_string_lossy();
                        CFStringCreateWithBytes(
                            std::ptr::null_mut(),
                            s.as_ptr(),
                            s.len() as isize,
                            UTF8,
                            0,
                        )
                    })
                    .collect();
                let array = CFArrayCreate(
                    std::ptr::null_mut(),
                    strings.as_ptr(),
                    strings.len() as isize,
                    &kCFTypeArrayCallBacks,
                );
                for s in strings {
                    CFRelease(s);
                }
                let sink: *mut Sink = Box::into_raw(Box::new(Box::new(sink)));
                let mut context = Context {
                    version: 0,
                    info: sink as *mut c_void,
                    retain: std::ptr::null(),
                    release: std::ptr::null(),
                    copy_description: std::ptr::null(),
                };
                let stream = FSEventStreamCreate(
                    std::ptr::null_mut(),
                    deliver,
                    &mut context,
                    array,
                    SINCE_NOW,
                    latency,
                    NO_DEFER | WATCH_ROOT | FILE_EVENTS,
                );
                CFRelease(array);
                if stream.is_null() {
                    drop(Box::from_raw(sink));
                    return None;
                }
                let queue = dispatch_queue_create(
                    c"com.powermove.linked-folders".as_ptr(),
                    std::ptr::null_mut(),
                );
                FSEventStreamSetDispatchQueue(stream, queue);
                if FSEventStreamStart(stream) == 0 {
                    FSEventStreamInvalidate(stream);
                    FSEventStreamRelease(stream);
                    dispatch_release(queue);
                    drop(Box::from_raw(sink));
                    return None;
                }
                Some(Self {
                    stream,
                    queue,
                    sink,
                })
            }
        }
    }

    extern "C" fn drained(_: Ref) {}

    impl Drop for Watch {
        fn drop(&mut self) {
            // SAFETY: Stop + Invalidate guarantee no callback runs after
            // they return, so the sink can be freed.
            unsafe {
                FSEventStreamStop(self.stream);
                FSEventStreamInvalidate(self.stream);
                dispatch_sync_f(self.queue, std::ptr::null_mut(), drained);
                FSEventStreamRelease(self.stream);
                dispatch_release(self.queue);
                drop(Box::from_raw(self.sink));
            }
        }
    }
}

/// The directories to read again for `events` under `root`: an event names
/// a file or folder, so its parent is read (and the folder itself, when it
/// is one). `None`: read the whole tree again.
pub fn dirty_dirs(root: &Path, events: &[(PathBuf, u32)]) -> Option<BTreeSet<PathBuf>> {
    #[cfg(target_os = "macos")]
    let (must, root_changed) = (watch::MUST_SCAN, watch::ROOT_CHANGED);
    #[cfg(not(target_os = "macos"))]
    let (must, root_changed) = (0x7, 0x20);
    let mut dirs = BTreeSet::new();
    for (path, flags) in events {
        if flags & (must | root_changed) != 0 {
            return None;
        }
        if !path.starts_with(root) {
            continue;
        }
        if path.is_dir() {
            dirs.insert(path.clone());
        }
        if let Some(parent) = path.parent().filter(|p| p.starts_with(root)) {
            dirs.insert(parent.to_path_buf());
        }
    }
    Some(dirs)
}

#[cfg(test)]
mod tests;
