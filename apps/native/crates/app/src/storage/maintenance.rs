use super::{content_named, retention::*};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub(super) const RECEIPT: &str = "storage-maintenance-v1.json";
const CACHE_FOLDERS: &[&str] = &[
    "media-cache/objects",
    "media-cache/media-thumbnails",
    "media-cache/panel-covers-v1",
    "caches/thumbnails",
    "caches/last-frame",
    "caches/agent-thumbnails",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Bucket {
    Backups,
    Journals,
    Cache,
    AgentHistory,
    Recovery,
    Migration,
    Codex,
    Claude,
}

pub(super) struct Category {
    pub id: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
    pub paths: &'static [&'static str],
    pub bucket: Bucket,
}

impl Category {
    pub fn limit(&self, cache_limit: u64) -> Option<u64> {
        match self.bucket {
            Bucket::Backups => Some(BACKUP_LIMIT),
            Bucket::Journals => Some(JOURNAL_LIMIT),
            Bucket::Cache => Some(cache_limit),
            Bucket::AgentHistory => Some(HISTORY_LIMIT),
            Bucket::Codex | Bucket::Claude => Some(LOG_LIMIT),
            Bucket::Migration | Bucket::Recovery => None,
        }
    }
}

pub(super) const CATEGORIES: &[Category] = &[
    Category {
        id: "backups",
        title: "Project Backups",
        detail: "Keeps recent versions and at least one backup per project.",
        paths: &["backups"],
        bucket: Bucket::Backups,
    },
    Category {
        id: "journals",
        title: "Edit Recovery",
        detail: "Keeps active edit history and up to seven days of discarded recovery points.",
        paths: &["journals"],
        bucket: Bucket::Journals,
    },
    Category {
        id: "cache",
        title: "Media Cache",
        detail: "Powermove can rebuild these media copies and previews.",
        paths: &[
            "media-cache",
            "caches/thumbnails",
            "caches/last-frame",
            "caches/agent-thumbnails",
            "Media Storage/Proxies/store",
            "Media Storage/Optimized/store",
        ],
        bucket: Bucket::Cache,
    },
    Category {
        id: "agent-history",
        title: "Agent Run History",
        detail: "Expires old run data while keeping saved files and exports.",
        paths: &[
            "mcp-runs",
            "mcp-archives",
            "Agent Change History",
            "agent-change-history",
        ],
        bucket: Bucket::AgentHistory,
    },
    Category {
        id: "recovery",
        title: "Recovery Files",
        detail: "Keeps recovered projects and retires expired temporary files.",
        paths: &["recovery-files"],
        bucket: Bucket::Recovery,
    },
    Category {
        id: "migration",
        title: "Migration Snapshots",
        detail: "Finished imports can leave old snapshots. Powermove keeps the ones it still needs.",
        paths: &["first-launch-checkpoint", "first-launch-deltas"],
        bucket: Bucket::Migration,
    },
    Category {
        id: "codex",
        title: "Codex Logs",
        detail: "Rotates old diagnostic logs from Codex.",
        paths: &["codex-runtime/log", "codex-runtime/logs"],
        bucket: Bucket::Codex,
    },
    Category {
        id: "claude",
        title: "Claude Logs",
        detail: "Rotates old diagnostic logs from Claude.",
        paths: &["claude-runtime/debug", "claude-runtime/logs"],
        bucket: Bucket::Claude,
    },
];

fn cached_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let stemmed = |length| {
        name.strip_suffix(".png").is_some_and(|name| {
            name.len() == length && name.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    };
    match path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
    {
        Some("objects") => content_named(name),
        Some("agent-thumbnails" | "media-thumbnails") => stemmed(32),
        Some("panel-covers-v1") => stemmed(64),
        Some("thumbnails" | "last-frame") if name.ends_with(".png") => {
            let stamp = path.with_extension("stamp");
            if !fs::symlink_metadata(&stamp).is_ok_and(|meta| meta.is_file() && meta.len() <= 2048)
            {
                return false;
            }
            fs::read_to_string(stamp).is_ok_and(|text| {
                let mut lines = text.lines();
                lines.next().is_some_and(|line| line.parse::<i64>().is_ok())
                    && lines.next().is_some_and(|line| line.parse::<f64>().is_ok())
                    && lines.next().is_some_and(|line| {
                        line.rsplit_once(" posters ")
                            .is_some_and(|(_, version)| version.parse::<u32>().is_ok())
                    })
            })
        }
        _ => false,
    }
}

fn clean_cache(
    profile: &Path,
    limit: u64,
    pinned: &BTreeSet<PathBuf>,
    idle: &impl Fn() -> bool,
) -> io::Result<u64> {
    let mut reclaimed: u64 = 0;
    if let Some(root) = owned_dir(profile, "media-cache") {
        reclaimed = crate::media::sweep_cache(&root, idle)?;
    }
    let reclaimed_by_budget = crate::media::with_idle_cache(|| {
        let mut list = Vec::new();
        for folder in CACHE_FOLDERS {
            if let Some(root) = owned_dir(profile, folder) {
                for entry in children(&root) {
                    if !idle() {
                        return Ok(0);
                    }
                    let path = entry.path();
                    if cached_file(&path)
                        && let Some(file) = File::read(path)
                    {
                        list.push(file);
                    }
                }
            }
        }
        let mut proxies = BTreeSet::new();
        for path in crate::pro_media::regenerable_proxies(profile, idle) {
            let Ok(relative) = path.strip_prefix(profile) else {
                continue;
            };
            let Some(parent) = relative.parent().and_then(Path::to_str) else {
                continue;
            };
            let own_hierarchy = [
                "Media Storage/Proxies/store/",
                "Media Storage/Optimized/store/",
            ]
            .iter()
            .any(|prefix| {
                parent.strip_prefix(prefix).is_some_and(|hash| {
                    hash.len() == 2 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                })
            });
            if !own_hierarchy || owned_dir(profile, parent).is_none() || pinned.contains(&path) {
                continue;
            }
            if let Some(file) = File::read(path.clone()) {
                proxies.insert(path);
                list.push(file);
            }
        }
        let freed = prune_files(list, limit, |path| pinned.contains(path), idle)?;
        for path in proxies.into_iter().filter(|path| !path.exists()) {
            if !idle() {
                break;
            }
            if let Some(record) = File::read(path.with_extension("json")) {
                let _ = record.remove()?;
            }
        }
        Ok::<_, io::Error>(freed)
    })?;
    Ok(reclaimed.saturating_add(reclaimed_by_budget))
}

fn clean_logs(
    profile: &Path,
    folders: &[&str],
    now: SystemTime,
    idle: &impl Fn() -> bool,
) -> io::Result<u64> {
    let quiet = || idle() && !pm_agents::launch::grants::any_live();
    let mut list = Vec::new();
    for folder in folders {
        if let Some(root) = owned_dir(profile, folder) {
            for entry in children(&root) {
                if !quiet() {
                    return Ok(0);
                }
                let path = entry.path();
                let suffix = path.extension().and_then(|ext| ext.to_str());
                if matches!(suffix, Some("log" | "txt"))
                    && let Some(file) = File::read(path)
                {
                    list.push(file);
                }
            }
        }
    }
    let in_use: BTreeSet<_> = list
        .iter()
        .filter(|file| file.age(now) < Duration::from_secs(300))
        .map(|file| file.path.clone())
        .collect();
    prune_files(list, LOG_LIMIT, |path| in_use.contains(path), &quiet)
}

/// Only these files are written from generated input or the bundled tool
/// catalog. Artifacts, downloaded media and staged extensions remain owned
/// by the person, including in archived workspaces.
fn clean_mcp(
    profile: &Path,
    limit: u64,
    now: SystemTime,
    idle: &impl Fn() -> bool,
) -> io::Result<u64> {
    let Some(archives) = owned_dir(profile, "mcp-archives") else {
        return Ok(0);
    };
    let mut runs = Vec::new();
    for archive in children(&archives) {
        if !idle() {
            return Ok(0);
        }
        let name = archive.file_name();
        let Some(id) = name.to_str().filter(|name| {
            name.strip_prefix("mcp-run").is_some_and(|seed| {
                seed.len() == 7
                    && seed
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            })
        }) else {
            continue;
        };
        let archive_meta = fs::symlink_metadata(archive.path())?;
        if !archive_meta.is_dir() {
            continue;
        }
        let time = archive_meta.modified()?;
        if now.duration_since(time).unwrap_or_default() < Duration::from_secs(300) {
            continue;
        }
        let mut run_files = Vec::new();
        for workspace in [
            format!("mcp-runs/{id}"),
            format!("mcp-archives/{id}/workspace"),
        ] {
            for (folder, name) in [
                ("inputs", "powermove-project.json"),
                ("api", "mcp-tools.json"),
            ] {
                if let Some(location) = owned_dir(profile, &format!("{workspace}/{folder}"))
                    && let Some(file) = File::read(location.join(name))
                {
                    run_files.push(file);
                }
            }
        }
        runs.push((time, run_files));
    }
    expire_runs(runs, now, limit, idle)
}

/// Called from the storage worker. A test calling this from the main thread
/// gets a clear error before any profile IO.
pub(super) fn clean(
    profile: &Path,
    buckets: &BTreeSet<Bucket>,
    cache_limit: u64,
    pinned: &BTreeSet<PathBuf>,
    idle: &impl Fn() -> bool,
) -> anyhow::Result<u64> {
    anyhow::ensure!(
        objc2::MainThreadMarker::new().is_none(),
        "Storage cleanup needs a worker"
    );
    anyhow::ensure!(
        fs::symlink_metadata(profile).is_ok_and(|meta| meta.is_dir()),
        "Storage cleanup needs a real profile folder"
    );
    let mut freed: u64 = 0;
    let now = SystemTime::now();
    for category in CATEGORIES {
        if !idle() {
            break;
        }
        if !buckets.contains(&category.bucket) {
            continue;
        }
        let bytes = match category.bucket {
            Bucket::Backups => pm_kernel::project_io::tidy_backups(
                &profile.join("backups"),
                now,
                BACKUP_LIMIT,
                idle,
            )?,
            Bucket::Journals => pm_kernel::journal::tidy_discarded(
                &profile.join("journals"),
                now,
                JOURNAL_LIMIT,
                idle,
            )?,
            Bucket::Cache => clean_cache(profile, cache_limit, pinned, idle)?,
            Bucket::AgentHistory => clean_history(profile, now, idle)?,
            Bucket::Recovery => clean_recovery(profile, now, idle)?,
            Bucket::Migration => pm_first_launch::later::tidy_superseded(profile, now, idle)?,
            Bucket::Codex | Bucket::Claude => clean_logs(profile, category.paths, now, idle)?,
        };
        freed = freed.saturating_add(bytes);
    }
    Ok(freed)
}

fn clean_history(profile: &Path, now: SystemTime, idle: &impl Fn() -> bool) -> anyhow::Result<u64> {
    let paths = [
        "mcp-runs",
        "mcp-archives",
        "agent-change-history",
        "Agent Change History",
    ];
    let logical = |parts: &[&str]| {
        let mut found = Vec::new();
        for part in parts {
            if let Some(root) = owned_dir(profile, part) {
                files(&root, idle, &mut found, &|_| true);
            }
        }
        found.iter().map(File::len).sum::<u64>()
    };
    let mcp_limit = HISTORY_LIMIT.saturating_sub(logical(&paths[2..]));
    let mut freed = clean_mcp(profile, mcp_limit, now, idle)?;
    for history in &paths[2..] {
        if !idle() {
            break;
        }
        let other: Vec<_> = paths.into_iter().filter(|part| part != history).collect();
        let limit = HISTORY_LIMIT.saturating_sub(logical(&other));
        if let Some(root) = owned_dir(profile, history) {
            freed =
                freed.saturating_add(pm_mcp::extensions::tidy_history(&root, now, limit, idle)?);
        }
    }
    Ok(freed)
}

fn clean_recovery(profile: &Path, now: SystemTime, idle: &impl Fn() -> bool) -> io::Result<u64> {
    let Some(root) = owned_dir(profile, "recovery-files") else {
        return Ok(0);
    };
    // Published PMVs can be the only backing for a library project or an
    // embedded asset. This pass retires only interrupted atomic staging.
    let mut freed: u64 = 0;
    for entry in children(&root) {
        if !idle() {
            break;
        }
        let name = entry.file_name();
        if pm_kernel::project_io::is_recovery_stage(&name)
            && let Some(file) = File::read(entry.path())
            && file.len() == 0
            && file.age(now) >= 14 * DAY
            && let Some(bytes) = file.remove()?
        {
            freed = freed.saturating_add(bytes);
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn profile_hygiene_media_cache_leaves_unrecognized_files() {
        let root = tempfile::tempdir().unwrap();
        let thumbnail_dir = root.path().join("caches/thumbnails");
        fs::create_dir_all(&thumbnail_dir).unwrap();
        let authored = thumbnail_dir.join("my-export.png");
        let generated = thumbnail_dir.join("project.png");
        fs::write(&authored, "keep").unwrap();
        fs::write(&generated, "cache").unwrap();
        fs::write(
            generated.with_extension("stamp"),
            "1\n0\nfixture (1) posters 3\n",
        )
        .unwrap();
        clean_cache(root.path(), 0, &BTreeSet::new(), &|| true).unwrap();
        assert!(authored.is_file());
        assert!(!generated.exists());
    }

    #[test]
    fn profile_hygiene_worker_protects_provider_state_and_linked_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let codex = root.join("codex-runtime");
        fs::create_dir_all(codex.join("log")).unwrap();
        let file = codex.join("log/old.log");
        let log = fs::File::create(&file).unwrap();
        log.set_len(120_000_000).unwrap();
        log.set_modified(SystemTime::now() - DAY).unwrap();
        let account = codex.join("auth.json");
        fs::write(&account, "synthetic account file").unwrap();
        let outside = tempfile::tempdir().unwrap();
        let elsewhere = outside.path().join("anything.log");
        fs::write(&elsewhere, "synthetic file outside the profile").unwrap();
        std::os::unix::fs::symlink(outside.path(), codex.join("logs")).unwrap();
        let selected = BTreeSet::from([Bucket::Codex]);
        let worker_root = root.to_owned();
        std::thread::spawn(move || {
            clean(
                &worker_root,
                &selected,
                CACHE_DEFAULT,
                &BTreeSet::new(),
                &|| true,
            )
            .unwrap()
        })
        .join()
        .unwrap();
        assert!(!file.exists());
        assert!(account.is_file() && elsewhere.is_file());
    }

    #[test]
    fn profile_hygiene_bloated_fixture() {
        let Some(path) = std::env::var_os("PM_HYGIENE_PROFILE") else {
            return;
        };
        let root = PathBuf::from(path);
        assert_eq!(
            fs::read_to_string(root.join(".synthetic-profile")).unwrap(),
            "profile-hygiene fixture v1\n"
        );
        let allocated_before = super::super::used(&root);
        let selected = CATEGORIES.iter().map(|category| category.bucket).collect();
        let worker_root = root.clone();
        let freed = std::thread::spawn(move || {
            let mut kernel = pm_kernel::Kernel::new(Default::default());
            let reply = kernel
                .shell(pm_kernel::session::ShellCall::AttachLibrary {
                    profile: worker_root.clone(),
                    format: pm_kernel::persistence::DiskFormat::V1_2,
                    electron_profile: None,
                })
                .unwrap();
            assert!(matches!(
                reply,
                pm_kernel::session::ShellReply::Library(pm_kernel::locks::OpenDecision::Writable)
            ));
            assert_eq!(
                kernel.state().settings["ext.synthetic-113"]
                    .as_str()
                    .unwrap()
                    .len(),
                1_000_000,
                "legacy settings moved before deleting their snapshot"
            );
            let freed = super::super::worker::reclaim(&worker_root, true, || {
                clean(
                    &worker_root,
                    &selected,
                    CACHE_DEFAULT,
                    &BTreeSet::new(),
                    &|| true,
                )
            })
            .unwrap();
            super::super::worker::record_first(&worker_root, freed).unwrap();
            freed
        })
        .join()
        .unwrap();
        assert!(
            freed <= allocated_before,
            "the receipt cannot exceed allocated bytes in the entire sparse fixture"
        );
        assert!(!root.join("kernel-state.json").exists());
        assert!(!root.join("kernel-state.json.tmp-w-synthetic").exists());
        for path in [
            "exports/keep.mov",
            "Media Store/keep.mov",
            "Media Storage/Originals/keep.mov",
            "mcp-archives/mcp-run0000219/workspace/artifacts/keep.mov",
            "recovery-files/recovery-embedded.pmv",
            "first-launch-checkpoint/checkpoint.json",
        ] {
            assert!(root.join(path).is_file(), "keeper {path} disappeared");
        }
        assert!(
            !root
                .join(
                    "recovery-files/.recovery-orphan.pmv.50000000-4000-4000-8000-000000000000.tmp"
                )
                .exists()
        );
        assert!(
            root.join("journals/P-editing/s999999999999999999999999-0000000001")
                .is_dir()
        );
        let logical = |stores: &[&str]| {
            let mut tally = Vec::new();
            for store in stores {
                files(&root.join(store), &|| true, &mut tally, &|_| true);
            }
            tally.iter().map(File::len).sum::<u64>()
        };
        assert!(logical(&["backups"]) <= BACKUP_LIMIT);
        assert!(logical(&["journals"]) <= JOURNAL_LIMIT);
        assert!(logical(CACHE_FOLDERS) <= CACHE_DEFAULT);
        assert!(logical(&["mcp-runs", "mcp-archives", "Agent Change History"]) <= HISTORY_LIMIT);
        assert!(logical(&["codex-runtime/log"]) <= LOG_LIMIT);
        assert!(logical(&["claude-runtime/debug"]) <= LOG_LIMIT);
        assert!(
            !root
                .join("first-launch-deltas/aaaaaaaaaaaaaaaa-aaaaaaaaaaaaaaaa-aaaaaaaaaaaaaaaa")
                .exists()
        );
        assert!(root.join("first-launch-deltas/bbbbbbbbbbbbbbbb-bbbbbbbbbbbbbbbb-bbbbbbbbbbbbbbbb/checkpoint.json").is_file());
        assert!(root.join("recovery-files/recovery-orphan.pmv").is_file());
    }
}
