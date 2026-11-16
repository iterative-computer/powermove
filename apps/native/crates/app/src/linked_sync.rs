//! One cancellable worker per open document. FSEvents callbacks only enqueue.
use pm_builtins::linked_folders::{self as linked, Tree};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
#[derive(Default)]
pub struct Sync {
    key: Option<(u64, Vec<(String, std::path::PathBuf)>)>,
    cancel: Option<Arc<AtomicBool>>,
    changed: Arc<AtomicU64>,
    seen: u64,
    folders: Option<pm_model::Node>,
    document: u64,
}
impl Drop for Sync {
    fn drop(&mut self) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::Release);
        }
    }
}
impl Sync {
    pub fn update(
        &mut self,
        p: &pm_kernel::Published,
        runtime: Arc<pm_builtins::Builtins>,
    ) -> bool {
        let folders = p.snapshot.root().and_then(|r| r.get("assetFolders"));
        let same = match (
            self.folders.as_ref().map(pm_model::Node::kind),
            folders.map(pm_model::Node::kind),
        ) {
            (Some(pm_model::NodeKind::Object(a)), Some(pm_model::NodeKind::Object(b))) => {
                Arc::ptr_eq(a, b)
            }
            (None, None) => true,
            _ => false,
        };
        if same && self.document == p.snapshot.doc_id {
            let now = self.changed.load(Ordering::Acquire);
            let changed = now != self.seen;
            self.seen = now;
            return changed;
        }
        self.folders = folders.cloned();
        self.document = p.snapshot.doc_id;
        let project = serde_json::json!({"assetFolders": folders.map(|n| serde_json::from_str::<serde_json::Value>(&pm_model::json::to_canonical_string(&n.to_value())).unwrap_or_default())});
        let roots = linked::roots(&project);
        let key = (p.snapshot.doc_id, roots.clone());
        if self.key.as_ref() != Some(&key) {
            if let Some(c) = self.cancel.take() {
                c.store(true, Ordering::Release);
            }
            self.key = Some(key);
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancel = Some(cancel.clone());
            let changed = self.changed.clone();
            let document = p.snapshot.doc_id;
            if roots.is_empty() {
                runtime.update_folder_sync(Default::default());
            }
            if !roots.is_empty() {
                std::thread::spawn(move || {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let paths: Vec<_> = roots.iter().map(|(_, p)| p.clone()).collect();
                    let attach = || {
                        let tx = tx.clone();
                        linked::watch::Watch::new(&paths, 0.3, move |events| {
                            let _ = tx.send(events);
                        })
                    };
                    let mut watcher = attach();
                    let handles: BTreeMap<_, _> = roots
                        .iter()
                        .filter_map(|(id, path)| {
                            std::fs::File::open(path).ok().map(|f| (id.clone(), f))
                        })
                        .collect();
                    let mut trees: BTreeMap<String, Tree> = BTreeMap::new();
                    let mut events: Option<Vec<linked::watch::Event>> = None;
                    loop {
                        if cancel.load(Ordering::Acquire) {
                            break;
                        }
                        let mut states = BTreeMap::new();
                        for (id, path) in &roots {
                            states.insert(id.clone(), pm_panel_media::FolderSync::Syncing);
                            let _ = path;
                        }
                        changed.fetch_add(1, Ordering::Release);
                        runtime.update_folder_sync(states);
                        if watcher.is_none() {
                            watcher = attach();
                        }
                        let mut retry = watcher.is_none();
                        let mut states = BTreeMap::new();
                        for (id, original_path) in &roots {
                            let relocated = handles
                                .get(id)
                                .and_then(directory_path)
                                .filter(|p| p.is_dir());
                            let path = relocated.as_ref().unwrap_or(original_path);
                            let tree = match (&events, trees.get_mut(id)) {
                                (Some(events), Some(tree)) if tree.root == *path => {
                                    match linked::dirty_dirs(path, events) {
                                        Some(dirs) => {
                                            tree.refresh(&dirs);
                                            Some(tree.clone())
                                        }
                                        None => Tree::scan(path),
                                    }
                                }
                                _ => Tree::scan(path),
                            };
                            let found = tree.as_ref().is_some_and(|t| t.dirs.contains_key(path));
                            let mut busy = false;
                            if let Some(tree) = tree {
                                busy = runtime.sync_linked(document, id, &tree, &cancel).is_err();
                                retry |= busy;
                                trees.insert(id.clone(), tree);
                            }
                            retry |= !found;
                            states.insert(
                                id.clone(),
                                if !found {
                                    pm_panel_media::FolderSync::NotFound
                                } else if busy {
                                    pm_panel_media::FolderSync::Syncing
                                } else {
                                    pm_panel_media::FolderSync::Synced
                                },
                            );
                        }
                        if cancel.load(Ordering::Acquire) {
                            return;
                        }
                        changed.fetch_add(1, Ordering::Release);
                        runtime.update_folder_sync(states);
                        let deadline = std::time::Instant::now() + Duration::from_secs(2);
                        let first = loop {
                            if cancel.load(Ordering::Acquire) {
                                return;
                            }
                            if retry && std::time::Instant::now() >= deadline {
                                break vec![(std::path::PathBuf::new(), linked::watch::MUST_SCAN)];
                            }
                            if let Ok(e) = rx.recv_timeout(Duration::from_millis(100)) {
                                break e;
                            }
                        };
                        let mut batch = first;
                        while let Ok(e) = rx.recv_timeout(Duration::from_millis(300)) {
                            batch.extend(e);
                            if cancel.load(Ordering::Acquire) {
                                return;
                            }
                        }
                        events = Some(batch);
                    }
                });
            }
        }
        let now = self.changed.load(Ordering::Acquire);
        let changed = now != self.seen;
        self.seen = now;
        changed
    }
}

/// An open directory descriptor follows Finder renames and moves on its volume.
fn directory_path(file: &std::fs::File) -> Option<std::path::PathBuf> {
    use std::os::fd::AsRawFd;
    let mut bytes = [0u8; 1024];
    // SAFETY: F_GETPATH writes at most MAXPATHLEN bytes to this live buffer.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, bytes.as_mut_ptr()) } != 0 {
        return None;
    }
    let end = bytes.iter().position(|b| *b == 0)?;
    use std::os::unix::ffi::OsStringExt;
    Some(std::ffi::OsString::from_vec(bytes[..end].to_vec()).into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn directory_handle_follows_a_finder_move() {
        let dir = tempfile::tempdir().unwrap();
        let before = dir.path().join("Before");
        std::fs::create_dir(&before).unwrap();
        let handle = std::fs::File::open(&before).unwrap();
        let after = dir.path().join("After");
        std::fs::rename(&before, &after).unwrap();
        assert_eq!(
            super::directory_path(&handle),
            Some(after.canonicalize().unwrap())
        );
    }
}

impl crate::session::Controller {
    pub(crate) fn poll_linked_sync(&mut self, cx: &mut gpui::Context<Self>) {
        let (Some(session), Some(runtime)) = (&self.session, self.builtins.runtime.clone()) else {
            return;
        };
        let Some(p) = self.kernel.published(&session.project) else {
            return;
        };
        if self.linked_sync.update(&p, runtime) {
            self.media_refresh.reset();
            self.refresh_media(cx);
            self.update_builtins(cx);
        }
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;
    #[test]
    fn watcher_worker_reports_deletion_without_removing_the_asset() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("Finder");
        std::fs::create_dir(&root).unwrap();
        let file = root.join("still.png");
        std::fs::write(
            &file,
            crate::mcp_services::png(&[12, 34, 56, 255], 1, 1).unwrap(),
        )
        .unwrap();
        let desk = crate::parity_tests::Desk::new(
            serde_json::json!({"id":"p","compId":"c","w":64,"h":64,"fps":30,"dur":1,"layers":[],"assets":{}}),
        );
        let runtime = Arc::new(
            pm_builtins::Builtins::new(desk.kernel(), &desk.session.project, || {}).unwrap(),
        );
        runtime
            .link_folders(
                desk.published().snapshot.doc_id,
                &[root],
                None,
                &AtomicBool::new(false),
            )
            .unwrap();
        let mut sync = Sync::default();
        sync.update(&desk.published(), runtime.clone());
        let wait = |minimum| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while sync.changed.load(Ordering::Acquire) < minimum {
                assert!(
                    std::time::Instant::now() < deadline,
                    "watcher worker timed out"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        wait(2);
        let before = sync.changed.load(Ordering::Acquire);
        let history = desk.published().history.entries;
        std::fs::remove_file(file).unwrap();
        wait(before + 2);
        assert!(sync.update(&desk.published(), runtime.clone()));
        assert_eq!(desk.published().history.entries, history);
        let value = desk.published().snapshot.document.to_value().unwrap();
        assert_eq!(value.get("assets").unwrap().as_object().unwrap().len(), 1);
        drop(sync);
        runtime.close();
    }
}
