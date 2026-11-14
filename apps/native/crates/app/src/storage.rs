//! Settings › Storage: what Powermove and the original app keep on this Mac,
//! measured off the main thread, and the parts of the original app's folder
//! that can go. Nothing here touches the original app's library or media.
use crate::pro_media::bytes;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// A part of the original app's folder that Settings › Storage can remove.
/// None of them is the original app's library or the media projects use.
struct Removable {
    id: &'static str,
    label: &'static str,
    detail: &'static str,
    /// The confirmation's question and what happens after.
    question: &'static str,
    after: &'static str,
    entries: fn(&Path) -> Vec<PathBuf>,
}
const REMOVABLE: &[Removable] = &[
    Removable {
        id: "backups",
        label: "Backups",
        detail: "Copies the original app made of its library over time.",
        question: "Delete the original app’s backups?",
        after: "The original app and its library keep working. It can’t go back to these backups afterwards.",
        entries: backups,
    },
    Removable {
        id: "recovery",
        label: "Recovery Files",
        detail: "Crash recovery and copies saved before restarts and updates.",
        question: "Delete the original app’s recovery files?",
        after: "Unsaved work the original app could still recover after a crash goes with them.",
        entries: recovery,
    },
    Removable {
        id: "agents",
        label: "Agent Work Folders",
        detail: "Files agents made while doing tasks: downloads, tools they installed, test renders.",
        question: "Delete the original app’s agent work folders?",
        after: "Agent chats stay, but files they point to in these folders won’t open.",
        entries: agents,
    },
    Removable {
        id: "tools",
        label: "Downloaded Tools",
        detail: "Codex, Claude and the speech models it downloaded.",
        question: "Delete the original app’s downloaded tools?",
        after: "It downloads them again when it needs them. You may need to sign in to ChatGPT or Claude there again.",
        entries: tools,
    },
    Removable {
        id: "caches",
        label: "Caches",
        detail: "Temporary files the original app makes again.",
        question: "Delete the original app’s caches?",
        after: "It makes them again as it needs them.",
        entries: caches,
    },
    Removable {
        id: "leftovers",
        label: "Leftover Media",
        detail: "Media files no project and no library entry uses.",
        question: "Delete the original app’s leftover media?",
        after: "No project uses these files, in the original app or here.",
        entries: leftover_media,
    },
];
fn children(root: &Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|n| root.join(n))
        .filter(|p| fs::symlink_metadata(p).is_ok())
        .collect()
}
fn backups(root: &Path) -> Vec<PathBuf> {
    let mut out = children(root, &["backups", "restart-backups"]);
    if let Ok(entries) = fs::read_dir(root) {
        out.extend(entries.flatten().map(|e| e.path()).filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("store-backup-"))
        }));
    }
    out.sort();
    out
}
fn recovery(root: &Path) -> Vec<PathBuf> {
    children(root, &["recovery"])
}
fn agents(root: &Path) -> Vec<PathBuf> {
    children(root, &["Agent Workspaces"])
}
fn tools(root: &Path) -> Vec<PathBuf> {
    children(
        root,
        &["codex-runtime", "claude-runtime", "runtimes", "models"],
    )
}
fn caches(root: &Path) -> Vec<PathBuf> {
    children(
        root,
        &[
            "Cache",
            "Code Cache",
            "GPUCache",
            "DawnGraphiteCache",
            "DawnWebGPUCache",
            "Shared Dictionary",
            "VideoDecodeStats",
        ],
    )
}
/// `Media Store/<64 hex>.<ext>` files no Index record names. The original app
/// finds media only through its Index, so it can't reach these either.
fn leftover_media(root: &Path) -> Vec<PathBuf> {
    let store = root.join("Media Store");
    let Ok(index) = fs::read_dir(store.join("Index")) else {
        // Without an Index nothing can be proven unused.
        return Vec::new();
    };
    let mut named = std::collections::BTreeSet::new();
    for entry in index.flatten() {
        let Ok(text) = fs::read(entry.path()) else {
            return Vec::new();
        };
        let Ok(record) = serde_json::from_slice::<Value>(&text) else {
            // An unreadable record might name anything: keep everything.
            return Vec::new();
        };
        if let Some(file) = record["file"].as_str() {
            named.insert(file.to_owned());
        }
        if let Some(source) = record["sourcePath"].as_str() {
            if let Some(name) = Path::new(source).file_name() {
                named.insert(name.to_string_lossy().into_owned());
            }
        }
    }
    let Ok(entries) = fs::read_dir(&store) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            content_named(&name) && !named.contains(&name)
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}
fn content_named(name: &str) -> bool {
    let Some((hash, extension)) = name.split_once('.') else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && (1..=8).contains(&extension.len())
        && extension.bytes().all(|b| b.is_ascii_alphanumeric())
}
/// Space a file or folder takes on disk. Links are counted, never followed.
pub(crate) fn used(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    let own = meta.blocks() * 512;
    if !meta.is_dir() {
        return own;
    }
    fs::read_dir(path).map_or(own, |entries| {
        own + entries.flatten().map(|e| used(&e.path())).sum::<u64>()
    })
}
fn sum(paths: &[PathBuf]) -> u64 {
    paths.iter().map(|p| used(p)).sum()
}

/// One measurement of both folders.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub library: u64,
    pub media: u64,
    pub tools: u64,
    pub native_total: u64,
    pub original: Option<Original>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Original {
    pub path: PathBuf,
    pub total: u64,
    /// Removable part id → bytes.
    pub parts: Vec<(&'static str, u64)>,
    /// Media this library reads in place from inside this folder.
    pub linked_items: u64,
    pub linked_bytes: u64,
}
pub(crate) fn measure(native: &Path, original: Option<&Path>) -> Report {
    let library = sum(&children(
        native,
        &[
            "store",
            "journals",
            "recovery-files",
            "backups",
            "kernel-state.json",
            "session.json",
        ],
    ));
    let media = sum(&children(native, &["Media Store", "Media Storage"]));
    let tools = sum(&children(
        native,
        &[
            "codex-runtime",
            "claude-runtime",
            "models",
            "runtimes",
            "updates",
        ],
    ));
    // Linked records name the original folder by its real (canonical) path.
    let linked = pm_native_migrate::linked_summary(&native.join("Media Store")).ok();
    let original = original.filter(|p| p.is_dir()).map(|root| {
        let real = root.canonicalize().unwrap_or_else(|_| root.to_owned());
        let inside = linked
            .as_ref()
            .and_then(|l| l.folders.get(&real).or_else(|| l.folders.get(root)));
        Original {
            path: root.to_owned(),
            total: used(root),
            parts: REMOVABLE
                .iter()
                .map(|r| (r.id, sum(&(r.entries)(root))))
                .collect(),
            linked_items: inside.map_or(0, |f| f.items),
            linked_bytes: inside.map_or(0, |f| f.bytes),
        }
    });
    Report {
        library,
        media,
        tools,
        native_total: used(native),
        original,
    }
}
/// Removes one part of the original app's folder. Refused while the original
/// app runs. Returns the space it took.
pub(crate) fn remove(original: &Path, id: &str) -> Result<u64, String> {
    let part = REMOVABLE
        .iter()
        .find(|r| r.id == id)
        .ok_or("Unknown storage item")?;
    if pm_kernel::locks::older_electron_running(original) {
        return Err("Quit the original app first.".into());
    }
    let mut freed = 0;
    for path in (part.entries)(original) {
        // Only direct entries of the original folder, or its Media Store.
        let parent = path.parent().unwrap_or(original);
        if parent != original && parent != original.join("Media Store") {
            return Err("Unexpected storage path".into());
        }
        freed += used(&path);
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            // A file, or a link (removed itself, never followed).
            fs::remove_file(&path)
        }
        .map_err(|e| format!("Couldn’t remove {}: {e}", path.display()))?;
    }
    Ok(freed)
}

static LATEST: Mutex<Option<Report>> = Mutex::new(None);
static MEASURING: AtomicBool = AtomicBool::new(false);
/// Bytes moved and to move while Move Into Library runs.
static ADOPTING: Mutex<Option<(u64, u64)>> = Mutex::new(None);

/// Settings › Storage's rows, ready for the panel.
pub(crate) fn rows(generated: Option<u64>, original_running: bool) -> Vec<Value> {
    let report = LATEST.lock().unwrap().clone();
    let Some(report) = report else {
        return vec![
            json!({"id":"storage.measuring","section":"storage","group":"Powermove",
            "label":"Space Used","control":"status","value":"Calculating…"}),
        ];
    };
    let mut rows = vec![
        json!({"id":"storage.native.total","section":"storage","group":"Powermove",
            "label":"Space Used","control":"status","value":bytes(report.native_total)}),
        json!({"id":"storage.native.library","section":"storage","group":"Powermove",
            "label":"Projects and Settings","control":"status","value":bytes(report.library),
            "detail":"Your library, its undo history and recovery copies."}),
        json!({"id":"storage.native.media","section":"storage","group":"Powermove",
            "label":"Media","control":"status","value":bytes(report.media),
            "detail":"Media kept inside Powermove’s own folder."}),
    ];
    if let Some(generated) = generated {
        rows.push(
            json!({"id":"storage.native.generated","section":"storage","group":"Powermove",
            "label":"Generated Media","control":"action","action":"media.clear",
            "actionLabel":"Clear…","value":bytes(generated),
            "detail":"Proxies, optimized media and renders. Powermove can make them again."}),
        );
    }
    rows.push(
        json!({"id":"storage.native.tools","section":"storage","group":"Powermove",
        "label":"Downloaded Tools","control":"status","value":bytes(report.tools),
        "detail":"Agent and speech tools. Powermove downloads them again when it needs them."}),
    );
    rows.push(
        json!({"id":"storage.native.reveal","section":"storage","group":"Powermove",
        "label":"Folder","control":"action","action":"storage.reveal.native",
        "actionLabel":"Show in Finder"}),
    );
    let Some(original) = &report.original else {
        return rows;
    };
    const GROUP: &str = "Original App";
    rows.push(json!({"id":"storage.original.total","section":"storage","group":GROUP,
        "label":"Space Used","control":"status","value":bytes(original.total),
        "detail": if original_running {
            "Quit the original app to remove anything here."
        } else {
            "The Powermove you used before. Removing these never touches its projects or the media they use."
        }}));
    let adopting = *ADOPTING.lock().unwrap();
    if original.linked_items > 0 || adopting.is_some() {
        let (value, detail) = match adopting {
            Some((done, total)) => (
                format!("Moving… {} of {}", bytes(done), bytes(total)),
                "Copying them into Powermove’s own folder. You can keep working.".to_owned(),
            ),
            None => (
                bytes(original.linked_bytes),
                format!(
                    "Your projects here read {} {} from this folder. Move them into Powermove’s own folder before you delete the original app.",
                    original.linked_items,
                    if original.linked_items == 1 {
                        "media file"
                    } else {
                        "media files"
                    }
                ),
            ),
        };
        rows.push(
            json!({"id":"storage.original.linked","section":"storage","group":GROUP,
            "label":"Media Your Projects Use","detail":detail,"control":"action",
            "action":"storage.adopt","actionLabel":"Move Into Library…","value":value,
            "disabled":adopting.is_some()}),
        );
    }
    let mut removable = 0;
    for part in REMOVABLE {
        let size = original
            .parts
            .iter()
            .find(|(id, _)| *id == part.id)
            .map_or(0, |(_, b)| *b);
        removable += size;
        if size == 0 {
            continue;
        }
        rows.push(
            json!({"id":format!("storage.original.{}", part.id),"section":"storage",
            "group":GROUP,"label":part.label,"detail":part.detail,"control":"action",
            "action":format!("storage.delete.{}", part.id),"actionLabel":"Delete…",
            "value":bytes(size),"disabled":original_running}),
        );
    }
    rows.push(json!({"id":"storage.original.library","section":"storage","group":GROUP,
        "label":"Library and Media","control":"status",
        "value":bytes(original.total.saturating_sub(removable)),
        "detail":"The original app’s projects and media. Keep them while you still use it, or while your projects here read media from it."}));
    rows.push(
        json!({"id":"storage.original.reveal","section":"storage","group":GROUP,
        "label":"Folder","control":"action","action":"storage.reveal.original",
        "actionLabel":"Show in Finder"}),
    );
    rows
}

impl crate::session::Controller {
    /// Measures both folders again off the main thread; Settings redraws.
    pub(crate) fn refresh_storage(&self, cx: &mut gpui::Context<Self>) {
        if MEASURING.swap(true, Ordering::AcqRel) {
            return;
        }
        let (native, original) = (self.profile.clone(), self.electron.clone());
        cx.spawn(async move |this, cx| {
            let report = cx
                .background_executor()
                .spawn(async move { measure(&native, original.as_deref()) })
                .await;
            *LATEST.lock().unwrap() = Some(report);
            MEASURING.store(false, Ordering::Release);
            let _ = this.update(cx, |v, cx| v.update_builtins(cx));
        })
        .detach();
    }
    /// Settings › Storage's fact: its rows, built from the last measurement.
    pub(crate) fn storage_settings_fact(&self) -> Value {
        let generated = self
            .pro_media
            .cached_usage()
            .map(|u| u.proxies + u.optimized + u.renders);
        let running = self
            .electron
            .as_deref()
            .is_some_and(pm_kernel::locks::older_electron_running);
        json!({"rows": rows(generated, running)})
    }
    /// A Settings › Storage action (`storage.settings`).
    pub(crate) fn storage_settings(&mut self, input: &Value, cx: &mut gpui::Context<Self>) {
        let action = input["action"].as_str().unwrap_or_default();
        match action {
            "storage.reveal.native" => crate::platform::reveal(&self.profile),
            "storage.reveal.original" => {
                if let Some(original) = &self.electron {
                    crate::platform::reveal(original);
                }
            }
            "storage.adopt" => self.confirm_adopt(cx),
            _ => {
                if let Some(id) = action.strip_prefix("storage.delete.") {
                    self.confirm_storage_delete(id, cx);
                }
            }
        }
    }
    /// The Settings window, for sheets about rows pressed there.
    fn settings_window(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> objc2::rc::Retained<objc2_app_kit::NSWindow> {
        self.builtins
            .windows
            .get("settings")
            .copied()
            .and_then(|h| {
                h.update(cx, |_, w, _| crate::platform::ns_window(w))
                    .ok()
                    .flatten()
            })
            .unwrap_or_else(|| self.host.borrow().window.clone())
    }
    /// Move Into Library: copies media the library reads from elsewhere into
    /// Powermove's own Media Store (clones on this volume), verified, resumable.
    fn confirm_adopt(&mut self, cx: &mut gpui::Context<Self>) {
        if ADOPTING.lock().unwrap().is_some() {
            return;
        }
        let bytes_inside = LATEST
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|r| r.original.as_ref())
            .map_or(0, |o| o.linked_bytes);
        use crate::alert::{Alert, Button};
        let alert = Alert::new(
            "Move your media into Powermove?",
            format!(
                "Powermove copies the {} of media your projects read from the original app into its own folder. Afterwards you can delete the original app without your projects losing media. On the same disk the copies share space with the originals.",
                bytes(bytes_inside)
            ),
            vec![
                Button::new("Move", true).default(),
                Button::new("Cancel", false).cancel(),
            ],
        );
        let answer = alert.sheet(&self.settings_window(cx));
        let media_root = self.profile.join("Media Store");
        cx.spawn(async move |this, cx| {
            if answer.recv().await != Ok(Some(true)) {
                return;
            }
            *ADOPTING.lock().unwrap() = Some((0, 0));
            let _ = this.update(cx, |v, cx| v.update_builtins(cx));
            let job = cx.background_executor().spawn(async move {
                pm_native_migrate::adopt_linked(&media_root, |done, total| {
                    *ADOPTING.lock().unwrap() = Some((done, total));
                    Ok(())
                })
                .map_err(|e| format!("{e:#}"))
            });
            // Progress redraws once a second while it copies.
            let ticker = {
                let this = this.clone();
                cx.spawn(async move |cx| {
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(1))
                            .await;
                        if ADOPTING.lock().unwrap().is_none()
                            || this.update(cx, |v, cx| v.update_builtins(cx)).is_err()
                        {
                            break;
                        }
                    }
                })
            };
            let result = job.await;
            *ADOPTING.lock().unwrap() = None;
            drop(ticker);
            let _ = this.update(cx, |v, cx| {
                v.message = match result {
                    Ok(report) if report.missing > 0 => format!(
                        "Moved {} into Powermove. {} couldn’t be found and stay offline.",
                        bytes(report.adopted_bytes),
                        report.missing
                    ),
                    Ok(report) => format!("Moved {} into Powermove", bytes(report.adopted_bytes)),
                    Err(e) => format!("Couldn’t move your media. {e}"),
                };
                // Open projects pick up their media's new home.
                v.refresh_media(cx);
                v.refresh_storage(cx);
                cx.notify();
            });
        })
        .detach();
    }
    fn confirm_storage_delete(&mut self, id: &str, cx: &mut gpui::Context<Self>) {
        let (Some(original), Some(part)) =
            (self.electron.clone(), REMOVABLE.iter().find(|r| r.id == id))
        else {
            return;
        };
        let size = LATEST
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|r| r.original.as_ref())
            .and_then(|o| o.parts.iter().find(|(p, _)| *p == id))
            .map_or(0, |(_, b)| *b);
        use crate::alert::{Alert, Button};
        let alert = Alert::new(
            part.question,
            format!("This frees {}. {}", bytes(size), part.after),
            vec![
                Button::new("Cancel", false).default().cancel(),
                Button::new("Delete", true).destructive(),
            ],
        );
        // On the Settings window the person pressed it in.
        let answer = alert.sheet(&self.settings_window(cx));
        let id = id.to_owned();
        let label = part.label;
        cx.spawn(async move |this, cx| {
            if answer.recv().await != Ok(Some(true)) {
                return;
            }
            let removed = cx
                .background_executor()
                .spawn(async move { remove(&original, &id) })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.message = match removed {
                    Ok(freed) => format!("Freed {}", bytes(freed)),
                    Err(e) => format!("Couldn’t delete {}. {e}", label.to_lowercase()),
                };
                v.refresh_storage(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    #[test]
    fn media_read_from_the_original_folder_offers_move_into_library() {
        *LATEST.lock().unwrap() = Some(Report {
            original: Some(Original {
                path: PathBuf::from("/synthetic/Powermove"),
                total: 50_000_000_000,
                parts: vec![("backups", 9_000_000_000)],
                linked_items: 1161,
                linked_bytes: 20_028_248_489,
            }),
            ..Report::default()
        });
        let rows = rows(None, false);
        let linked = rows
            .iter()
            .find(|r| r["id"] == "storage.original.linked")
            .unwrap();
        assert_eq!(linked["action"], "storage.adopt");
        assert_eq!(linked["value"], "20.0 GB");
        assert!(
            linked["detail"]
                .as_str()
                .unwrap()
                .contains("1161 media files")
        );
        *ADOPTING.lock().unwrap() = Some((5_000_000_000, 20_000_000_000));
        let rows = super::rows(None, false);
        let moving = rows
            .iter()
            .find(|r| r["id"] == "storage.original.linked")
            .unwrap();
        assert_eq!(moving["value"], "Moving… 5.0 GB of 20.0 GB");
        assert_eq!(moving["disabled"], true);
        *ADOPTING.lock().unwrap() = None;
        *LATEST.lock().unwrap() = None;
    }
    #[test]
    fn leftover_media_is_only_what_no_index_record_names() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let used = format!("{}.webm", "a".repeat(64));
        let left = format!("{}.bin", "b".repeat(64));
        write(&root.join("Media Store").join(&used), b"used");
        write(&root.join("Media Store").join(&left), b"left");
        write(&root.join("Media Store/other.txt"), b"not content-named");
        write(
            &root.join("Media Store/Index/x.json"),
            format!(r#"{{"key":"k","file":"{used}"}}"#).as_bytes(),
        );
        assert_eq!(
            leftover_media(root),
            vec![root.join("Media Store").join(&left)]
        );
        // An unreadable record could name anything: nothing is offered.
        write(&root.join("Media Store/Index/y.json"), b"{broken");
        assert!(leftover_media(root).is_empty());
    }
    #[test]
    fn removing_a_part_takes_only_that_part_and_never_follows_links() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Powermove");
        let outside = t.path().join("outside");
        write(&outside.join("keep.txt"), b"keep");
        write(&root.join("Agent Workspaces/p/work/a.bin"), &[0; 5000]);
        std::os::unix::fs::symlink(&outside, root.join("Agent Workspaces/p/link")).unwrap();
        write(&root.join("store/projects.json"), b"[]");
        write(&root.join("backups/one.json"), b"{}");
        write(&root.join("store-backup-20260101/x.json"), b"{}");
        let report = measure(t.path(), Some(&root));
        let original = report.original.unwrap();
        assert!(
            original
                .parts
                .iter()
                .any(|(id, b)| *id == "agents" && *b > 0)
        );
        assert!(remove(&root, "agents").unwrap() > 0);
        assert!(!root.join("Agent Workspaces").exists());
        assert!(
            outside.join("keep.txt").is_file(),
            "links are never followed"
        );
        remove(&root, "backups").unwrap();
        assert!(!root.join("backups").exists());
        assert!(!root.join("store-backup-20260101").exists());
        assert!(
            root.join("store/projects.json").is_file(),
            "the library stays"
        );
        assert!(remove(&root, "store").is_err());
    }
}
