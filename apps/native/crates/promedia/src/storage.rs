//! Managed storage. Purge is an archive move; no original-byte deletion API exists.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StorageClass {
    Originals,
    Proxies,
    Optimized,
    Renders,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub directory: PathBuf,
    pub quota_bytes: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSettings {
    pub locations: BTreeMap<StorageClass, Location>,
    pub archive: PathBuf,
}
impl StorageSettings {
    pub fn validate(&self) -> io::Result<()> {
        if self.locations.len() != 4 {
            return Err(invalid("all four storage classes required"));
        }
        let paths: Vec<_> = self
            .locations
            .values()
            .map(|l| &l.directory)
            .chain(std::iter::once(&self.archive))
            .collect();
        for (i, p) in paths.iter().enumerate() {
            if !p.is_absolute()
                || p.components()
                    .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            {
                return Err(invalid("absolute normalized storage paths required"));
            }
            for q in paths.iter().skip(i + 1) {
                if p.starts_with(q) || q.starts_with(p) {
                    return Err(invalid("storage locations overlap"));
                }
            }
        }
        Ok(())
    }
}
pub(crate) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
pub(crate) fn regular(path: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(invalid("expected regular file"));
    }
    Ok(())
}
pub(crate) fn no_links(path: &Path) -> io::Result<()> {
    let mut prefix = PathBuf::new();
    for part in path.components() {
        if matches!(part, Component::ParentDir) {
            return Err(invalid("parent traversal"));
        }
        prefix.push(part);
        match fs::symlink_metadata(&prefix) {
            Ok(m) if m.file_type().is_symlink() => return Err(invalid("symlink storage path")),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
/// Refuse adopting a foreign nonempty directory. Dedicated roots are worker-owned;
/// the host must hold its profile lock for their lifetime (no hostile concurrent IO).
pub(crate) fn owned_root(path: &Path, marker: &str) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid("absolute root required"));
    }
    no_links(path)?;
    fs::create_dir_all(path)?;
    let marker_path = path.join(marker);
    if marker_path.exists() {
        regular(&marker_path)?;
        if fs::read(&marker_path)? != b"powermove-native-v1\n" {
            return Err(invalid("foreign storage marker"));
        }
    } else {
        if fs::read_dir(path)?.next().is_some() {
            return Err(invalid("refuse nonempty foreign directory"));
        }
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker_path)?;
        f.write_all(b"powermove-native-v1\n")?;
        f.sync_all()?;
        File::open(path)?.sync_all()?;
    }
    fs::canonicalize(path)
}
/// Create, sync and publish without replacing an existing destination. A failed
/// publication can leave a .partial file; preserve it for inspection/recovery.
pub(crate) fn publish(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let parent = path.parent().ok_or_else(|| invalid("missing parent"))?;
    no_links(parent)?;
    let partial = parent.join(format!(
        ".partial-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::hard_link(&partial, path)?;
    File::open(parent)?.sync_all()?;
    fs::remove_file(partial)?;
    File::open(parent)?.sync_all()
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub active_bytes: u64,
    pub quota_bytes: u64,
    pub over_quota: bool,
}
pub struct Storage {
    settings: StorageSettings,
}
impl Storage {
    pub fn open(settings: StorageSettings) -> io::Result<Self> {
        settings.validate()?;
        // Validate all paths before creating any marker.
        for p in settings
            .locations
            .values()
            .map(|l| &l.directory)
            .chain(std::iter::once(&settings.archive))
        {
            no_links(p)?;
        }
        for location in settings.locations.values() {
            owned_root(&location.directory, ".pm-storage-v1")?;
        }
        owned_root(&settings.archive, ".pm-archive-v1")?;
        Ok(Self { settings })
    }
    pub fn settings(&self) -> &StorageSettings {
        &self.settings
    }
    pub fn path(&self, class: StorageClass, name: &str) -> io::Result<PathBuf> {
        if name.is_empty()
            || name.starts_with('.')
            || Path::new(name).components().count() != 1
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        {
            return Err(invalid("unsafe object name"));
        }
        let path = self.settings.locations[&class].directory.join(name);
        no_links(&path)?;
        Ok(path)
    }
    pub fn usage(&self, class: StorageClass) -> io::Result<Usage> {
        let location = &self.settings.locations[&class];
        no_links(&location.directory)?;
        let mut active_bytes = 0u64;
        let mut directories = vec![location.directory.clone()];
        while let Some(directory) = directories.pop() {
            no_links(&directory)?;
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    directories.push(entry.path());
                    continue;
                }
                regular(&entry.path())?;
                if entry.file_name() == ".pm-storage-v1" || entry.file_name() == ".pm-render-v1" {
                    continue;
                }
                active_bytes = active_bytes
                    .checked_add(entry.metadata()?.len())
                    .ok_or_else(|| invalid("usage overflow"))?;
            }
        }
        Ok(Usage {
            active_bytes,
            quota_bytes: location.quota_bytes,
            over_quota: active_bytes > location.quota_bytes,
        })
    }
    /// Dedicated cache child keeps derived-frame eviction away from originals,
    /// even if an original happens to have a .frame filename.
    pub fn render_cache(&self) -> io::Result<crate::cache::RenderCache> {
        let location = &self.settings.locations[&StorageClass::Renders];
        crate::cache::RenderCache::open(&location.directory.join("frames"), location.quota_bytes)
    }
    pub fn admit(&self, class: StorageClass, bytes: u64) -> io::Result<()> {
        let usage = self.usage(class)?;
        if usage
            .active_bytes
            .checked_add(bytes)
            .is_none_or(|n| n > usage.quota_bytes)
        {
            return Err(io::Error::other(
                "storage quota exceeded; choose a new location or archive explicitly",
            ));
        }
        Ok(())
    }
    pub fn put(&self, class: StorageClass, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        self.admit(class, bytes.len() as u64)?;
        let path = self.path(class, name)?;
        publish(&path, bytes)?;
        Ok(path)
    }
    /// references includes projects, history, trash, takes, agents and rollback.
    /// The host supplies closure; referenced payloads (including proxy-only) refuse.
    /// Archive consumes physical space: this never promises disk space reclamation.
    pub fn purge(&self, class: StorageClass, name: &str, referenced: bool) -> io::Result<PathBuf> {
        if referenced {
            return Err(invalid("referenced media cannot be purged"));
        }
        let source = self.path(class, name)?;
        regular(&source)?;
        no_links(&self.settings.archive)?;
        let prefix = format!("{class:?}-{name}");
        for generation in 0..u32::MAX {
            let target = self.settings.archive.join(format!("{prefix}-{generation}"));
            // Copy supports a distinct archive volume. Never replace; sync before
            // unlink. Interruption can leave a second safe copy, never lost bytes.
            let mut output = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            };
            let mut input = File::open(&source)?;
            let mut hash = blake3::Hasher::new();
            let mut block = [0u8; 65536];
            loop {
                let n = input.read(&mut block)?;
                if n == 0 {
                    break;
                }
                hash.update(&block[..n]);
                output.write_all(&block[..n])?;
            }
            output.sync_all()?;
            // Verify the durable destination before unlinking the active copy.
            let mut verify = File::open(&target)?;
            let mut archived = blake3::Hasher::new();
            loop {
                let n = verify.read(&mut block)?;
                if n == 0 {
                    break;
                }
                archived.update(&block[..n]);
            }
            if archived.finalize() != hash.finalize() {
                return Err(io::Error::other(
                    "archive verification failed; source retained",
                ));
            }
            File::open(&self.settings.archive)?.sync_all()?;
            fs::remove_file(&source)?;
            File::open(source.parent().expect("managed parent"))?.sync_all()?;
            return Ok(target);
        }
        Err(io::Error::other("archive namespace exhausted"))
    }
}
