//! Retention for app-owned storage. Display measurements can count other files
//! in the same folder, but removal is restricted to the producer's formats.
use std::{
    fs::{self, Metadata},
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub(super) const DAY: Duration = Duration::from_secs(24 * 60 * 60);
pub(super) const BACKUP_LIMIT: u64 = 2_000_000_000;
pub(super) const JOURNAL_LIMIT: u64 = 500_000_000;
pub(super) const HISTORY_LIMIT: u64 = 500_000_000;
pub(super) const LOG_LIMIT: u64 = 100_000_000;
pub(super) const CACHE_DEFAULT: u64 = 1_000_000_000;

/// Lists only real directories. A shortcut or substituted directory is not
/// part of the profile's storage.
pub(super) fn children(root: &Path) -> impl Iterator<Item = fs::DirEntry> {
    fs::symlink_metadata(root)
        .is_ok_and(|metadata| metadata.is_dir())
        .then(|| fs::read_dir(root).ok())
        .flatten()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
}

pub(super) struct File {
    pub(super) path: PathBuf,
    original: Metadata,
    pub(super) modified: SystemTime,
}

impl File {
    pub(super) fn read(path: PathBuf) -> Option<Self> {
        let original = fs::symlink_metadata(&path).ok()?;
        if !original.is_file() {
            return None;
        }
        let modified = original.modified().ok()?;
        Some(Self {
            path,
            original,
            modified,
        })
    }

    pub(super) fn allocated(&self) -> u64 {
        self.original.blocks().saturating_mul(512)
    }

    pub(super) fn age(&self, now: SystemTime) -> Duration {
        now.duration_since(self.modified).unwrap_or_default()
    }

    pub(super) fn len(&self) -> u64 {
        self.original.len()
    }

    pub(super) fn remove(&self) -> io::Result<Option<u64>> {
        let now = fs::symlink_metadata(&self.path)?;
        if !now.is_file()
            || now.dev() != self.original.dev()
            || now.ino() != self.original.ino()
            || now.len() != self.len()
            || now.modified()? != self.modified
        {
            return Ok(None);
        }
        fs::remove_file(&self.path)?;
        Ok(Some(if now.nlink() == 1 {
            self.allocated()
        } else {
            0
        }))
    }
}

/// Walks only generated files in a directory owned by the store's producer.
/// Checks the idle state between entries, including while scanning.
pub(super) fn files(
    root: &Path,
    keep_going: &impl Fn() -> bool,
    list: &mut Vec<File>,
    accept: &impl Fn(&Path) -> bool,
) {
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in children(&directory) {
            if !keep_going() {
                return;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(path),
                Ok(kind) if kind.is_file() && accept(&path) => {
                    if let Some(file) = File::read(path) {
                        list.push(file);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Oldest first. The budget uses logical sizes so it also works on sparse
/// files. The result counts disk blocks released by successful removals.
pub(super) fn prune_files(
    mut files: Vec<File>,
    limit: u64,
    pinned: impl Fn(&Path) -> bool,
    keep_going: &impl Fn() -> bool,
) -> io::Result<u64> {
    let mut total = files.iter().map(File::len).sum::<u64>();
    files.sort_unstable_by(|a, b| (&a.modified, &a.path).cmp(&(&b.modified, &b.path)));
    let mut freed: u64 = 0;
    for file in files {
        if total <= limit || !keep_going() {
            break;
        }
        if !pinned(&file.path) {
            match file.remove() {
                Ok(Some(bytes)) => {
                    freed += bytes;
                    total -= file.len();
                }
                Ok(None) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => total -= file.len(),
                Err(e) => return Err(e),
            }
        }
    }
    Ok(freed)
}

/// Every ancestor below the explicit profile must be a real directory.
pub(super) fn owned_dir(profile: &Path, relative: &str) -> Option<PathBuf> {
    let mut path = profile.to_owned();
    if !fs::symlink_metadata(&path).ok()?.is_dir() {
        return None;
    }
    for part in relative.split('/') {
        path.push(part);
        if !fs::symlink_metadata(&path).ok()?.is_dir() {
            return None;
        }
    }
    Some(path)
}

/// Keep the newest or recent items before enforcing the cap. Every file
/// passed here is a disposable output selected by its owning store.
pub(super) fn expire_runs(
    mut runs: Vec<(SystemTime, Vec<File>)>,
    now: SystemTime,
    limit: u64,
    keep_going: &impl Fn() -> bool,
) -> io::Result<u64> {
    runs.sort_unstable_by_key(|entry| entry.0);
    let cutoff = runs.len().saturating_sub(200);
    let mut total = runs
        .iter()
        .flat_map(|(_, files)| files)
        .map(File::len)
        .sum::<u64>();
    let mut freed: u64 = 0;
    for (index, (modified, files)) in runs.into_iter().enumerate() {
        let expired = index < cutoff
            && now
                .duration_since(modified)
                .is_ok_and(|age| age >= 30 * DAY);
        if !expired && total <= limit {
            continue;
        }
        for file in files {
            if !keep_going() {
                return Ok(freed);
            }
            match file.remove() {
                Ok(Some(bytes)) => {
                    freed = freed.saturating_add(bytes);
                    total -= file.len();
                }
                Ok(None) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => total -= file.len(),
                Err(e) => return Err(e),
            }
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_hygiene_cache_eviction_keeps_pins_and_links() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let cache = root.join("cache");
        fs::create_dir(&cache).unwrap();
        let user = root.join("original.mov");
        fs::write(&user, [1; 4096]).unwrap();
        let alias = cache.join("oldest.mov");
        fs::hard_link(&user, &alias).unwrap();
        let pin = cache.join("pinned.mov");
        fs::write(&pin, [2; 4096]).unwrap();
        let younger = cache.join("younger.mov");
        fs::write(&younger, [3; 4096]).unwrap();
        let now = SystemTime::now();
        fs::File::open(&alias)
            .unwrap()
            .set_modified(now - 2 * DAY)
            .unwrap();
        fs::File::open(&younger)
            .unwrap()
            .set_modified(now - DAY)
            .unwrap();
        let mut candidates = Vec::new();
        files(&cache, &|| true, &mut candidates, &|_| true);
        prune_files(candidates, 2 * 4096, |path| path == pin, &|| true).unwrap();
        assert!(!alias.exists());
        assert!(user.is_file() && pin.is_file() && younger.is_file());
        std::os::unix::fs::symlink(root, cache.join("elsewhere")).unwrap();
        assert!(owned_dir(root, "cache/elsewhere").is_none());
    }

    #[test]
    fn profile_hygiene_idle_gate_stops_removing_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, "keep").unwrap();
        assert_eq!(
            prune_files(
                vec![File::read(path.clone()).unwrap()],
                0,
                |_| false,
                &|| false
            )
            .unwrap(),
            0
        );
        assert!(path.is_file());
    }
}

#[cfg(test)]
#[test]
fn profile_hygiene_run_archive_keeps_recent_runs_and_caps_older_data() {
    let root = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    let mut runs = Vec::new();
    for index in 0..202 {
        let file = root.path().join(index.to_string());
        fs::write(&file, [0; 16]).unwrap();
        let stamp = now - (31 + index) * DAY;
        runs.push((stamp, vec![File::read(file).unwrap()]));
    }
    expire_runs(runs, now, u64::MAX, &|| true).unwrap();
    assert!(!root.path().join("201").exists() && !root.path().join("200").exists());
    assert!(root.path().join("199").is_file() && root.path().join("0").is_file());
}
