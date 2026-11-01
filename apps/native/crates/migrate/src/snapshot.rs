//! Snapshot creation never opens a LevelDB engine. Files are immutable copies;
//! the inventory must remain unchanged across the entire capture. A busy source
//! returns `source changed`; the caller retries or waits for Electron to quit.
use crate::Result;
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MIN_FREE: u64 = 4 * 1024 * 1024 * 1024;
pub const MAX_BLOB_COPY: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy)]
pub enum BlobCopy {
    All,
    Sample { max_bytes: u64, max_files: usize },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Stamp {
    size: u64,
    modified: u128,
    inode: u64,
    device: u64,
}
#[derive(Serialize, Deserialize)]
struct Inventory {
    version: u32,
    source_roots: Vec<PathBuf>,
    leveldb: BTreeMap<PathBuf, Stamp>,
    leveldb_hashes: BTreeMap<PathBuf, String>,
    blobs: BTreeMap<PathBuf, Stamp>,
    blob_files_before: usize,
    blob_bytes_before: u64,
    complete: bool,
}

pub struct Snapshot {
    root: PathBuf,
    inventory: Inventory,
}
impl Snapshot {
    pub fn capture(
        leveldb: &Path,
        blobs: Option<&Path>,
        destination: &Path,
        policy: BlobCopy,
    ) -> Result<Self> {
        ensure!(leveldb.is_dir(), "LevelDB source is not a directory");
        ensure!(!destination.exists(), "snapshot destination already exists");
        let parent = destination
            .parent()
            .context("snapshot needs a parent directory")?
            .canonicalize()?;
        let target = parent.join(destination.file_name().context("snapshot needs a name")?);
        let source = leveldb.canonicalize()?;
        ensure!(
            !target.starts_with(&source) && !source.starts_with(&target),
            "snapshot must be outside source"
        );
        if let Some(p) = blobs {
            let p = p.canonicalize()?;
            ensure!(
                !target.starts_with(&p) && !p.starts_with(&target),
                "snapshot must be outside blob source"
            );
        }
        check_space(&parent, 0)?;
        let before = inventory(leveldb, false)?;
        ensure!(
            before.contains_key(Path::new("CURRENT")),
            "source has no CURRENT"
        );
        let blob_before = if let Some(p) = blobs {
            inventory(p, true)?
        } else {
            BTreeMap::new()
        };
        let blob_bytes_before = total(&blob_before)?;
        let reserve = match policy {
            BlobCopy::All => blob_bytes_before,
            BlobCopy::Sample { max_bytes, .. } => {
                max_bytes.min(MAX_BLOB_COPY).min(blob_bytes_before)
            }
        };
        check_space(
            &parent,
            reserve
                .checked_add(total(&before)?)
                .context("snapshot size overflow")?,
        )?;
        fs::create_dir(&target)?;
        #[cfg(unix)]
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?;
        // On failure remove only the directory created by THIS capture.
        let result = (|| {
            fs::create_dir(target.join("leveldb"))?;
            fs::create_dir(target.join("blobs"))?;
            let mut hashes = BTreeMap::new();
            for (relative, stamp) in &before {
                let out = target.join("leveldb").join(relative);
                copy_verified_length(&leveldb.join(relative), &out, stamp.size)?;
                hashes.insert(relative.clone(), hash_file(&out)?.0);
            }
            let mut copied_blobs = BTreeMap::new();
            let mut copied_bytes = 0u64;
            if let Some(source) = blobs {
                for (relative, stamp) in &blob_before {
                    if let BlobCopy::Sample {
                        max_bytes,
                        max_files,
                    } = policy
                        && (copied_blobs.len() >= max_files
                            || copied_bytes.saturating_add(stamp.size)
                                > max_bytes.min(MAX_BLOB_COPY))
                    {
                        continue;
                    }
                    let out = target.join("blobs").join(relative);
                    fs::create_dir_all(out.parent().unwrap())?;
                    if let Err(error) =
                        copy_verified_length(&source.join(relative), &out, stamp.size)
                    {
                        let permission = error.downcast_ref::<std::io::Error>().is_some_and(|e| {
                            e.kind() == std::io::ErrorKind::PermissionDenied
                                || e.raw_os_error() == Some(libc::EPERM)
                        });
                        if matches!(policy, BlobCopy::Sample { .. }) && permission {
                            if out.exists() {
                                fs::remove_file(&out)?;
                            }
                            continue;
                        }
                        return Err(error);
                    }
                    copied_bytes += stamp.size;
                    copied_blobs.insert(relative.clone(), stamp.clone());
                }
            }
            ensure!(
                before == inventory(leveldb, false)?,
                "source changed during snapshot; retry or quit Electron"
            );
            if let Some(source) = blobs {
                ensure!(
                    blob_before == inventory(source, true)?,
                    "blob source changed during snapshot; retry or quit Electron"
                );
            }
            let mut source_roots = vec![leveldb.canonicalize()?];
            if let Some(p) = blobs {
                source_roots.push(p.canonicalize()?);
            }
            let inv = Inventory {
                version: 1,
                source_roots,
                leveldb: before,
                leveldb_hashes: hashes,
                complete: matches!(policy, BlobCopy::All)
                    && copied_blobs.len() == blob_before.len(),
                blobs: copied_blobs,
                blob_files_before: blob_before.len(),
                blob_bytes_before,
            };
            atomic_json(&target.join("snapshot.json"), &inv)?;
            File::open(&target)?.sync_all()?;
            Ok(Self {
                root: target.clone(),
                inventory: inv,
            })
        })();
        if result.is_err() {
            fs::remove_dir_all(&target)?;
        }
        result
    }
    pub fn open(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        let inventory: Inventory =
            serde_json::from_reader(open_regular(&root.join("snapshot.json"))?)?;
        ensure!(inventory.version == 1, "unsupported snapshot version");
        for p in inventory.leveldb.keys().chain(inventory.blobs.keys()) {
            validate_relative(p)?;
        }
        let result = Self { root, inventory };
        result.verify_leveldb()?;
        Ok(result)
    }
    pub(crate) fn ensure_destination(&self, destination: &Path) -> Result<()> {
        let resolved = if destination.exists() {
            destination.canonicalize()?
        } else {
            destination
                .parent()
                .context("destination needs a parent")?
                .canonicalize()?
                .join(
                    destination
                        .file_name()
                        .context("destination needs a name")?,
                )
        };
        for source in self
            .inventory
            .source_roots
            .iter()
            .chain(std::iter::once(&self.root))
        {
            ensure!(
                !resolved.starts_with(source) && !source.starts_with(&resolved),
                "destination overlaps migration source or snapshot"
            );
        }
        Ok(())
    }
    pub fn leveldb(&self) -> PathBuf {
        self.root.join("leveldb")
    }
    pub fn complete(&self) -> bool {
        self.inventory.complete
    }
    pub fn blob_counts(&self) -> (usize, u64, usize, u64) {
        (
            self.inventory.blob_files_before,
            self.inventory.blob_bytes_before,
            self.inventory.blobs.len(),
            self.inventory.blobs.values().map(|s| s.size).sum(),
        )
    }
    pub fn identity(&self) -> Result<String> {
        Ok(blake3::hash(&serde_json::to_vec(&self.inventory)?)
            .to_hex()
            .to_string())
    }
    pub(crate) fn verify_leveldb(&self) -> Result<()> {
        ensure!(
            self.inventory.leveldb.len() == self.inventory.leveldb_hashes.len(),
            "snapshot inventory mismatch"
        );
        for (path, stamp) in &self.inventory.leveldb {
            validate_relative(path)?;
            let (hash, size) = hash_file(&self.leveldb().join(path))?;
            ensure!(
                size == stamp.size && Some(&hash) == self.inventory.leveldb_hashes.get(path),
                "snapshot changed or incomplete"
            );
        }
        Ok(())
    }
    pub fn blob_path(&self, database: u64, number: u64) -> Result<Option<PathBuf>> {
        let relative = PathBuf::from(format!(
            "{database:x}/{:02x}/{number:x}",
            (number >> 8) & 255
        ));
        if let Some(stamp) = self.inventory.blobs.get(&relative) {
            let p = self.root.join("blobs").join(relative);
            ensure!(
                open_regular(&p)?.metadata()?.len() == stamp.size,
                "snapshot blob size changed"
            );
            Ok(Some(p))
        } else {
            Ok(None)
        }
    }
}
fn validate_relative(p: &Path) -> Result<()> {
    ensure!(
        p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_))),
        "invalid snapshot relative path"
    );
    Ok(())
}
fn total(files: &BTreeMap<PathBuf, Stamp>) -> Result<u64> {
    files.values().try_fold(0u64, |a, s| {
        a.checked_add(s.size).context("snapshot size overflow")
    })
}
fn inventory(root: &Path, recursive: bool) -> Result<BTreeMap<PathBuf, Stamp>> {
    fn walk(
        root: &Path,
        path: &Path,
        recurse: bool,
        out: &mut BTreeMap<PathBuf, Stamp>,
    ) -> Result<()> {
        ensure!(
            !fs::symlink_metadata(path)?.file_type().is_symlink(),
            "symlink in snapshot source"
        );
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            let m = fs::symlink_metadata(&p)?;
            ensure!(!m.file_type().is_symlink(), "symlink in snapshot source");
            if m.is_dir() {
                if recurse {
                    walk(root, &p, true, out)?;
                }
                continue;
            }
            let name = entry.file_name();
            let name = name.to_str().context("invalid source filename")?;
            if !(recurse
                || name == "CURRENT"
                || name.starts_with("MANIFEST-")
                || name.ends_with(".log")
                || name.ends_with(".ldb")
                || name.ends_with(".sst"))
            {
                continue;
            }
            ensure!(m.is_file(), "non-regular snapshot source");
            let stamp = Stamp {
                size: m.len(),
                modified: m
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos(),
                inode: m.ino(),
                device: m.dev(),
            };
            out.insert(p.strip_prefix(root)?.to_path_buf(), stamp);
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, recursive, &mut out)?;
    Ok(out)
}
pub(crate) fn check_space(root: &Path, needed: u64) -> Result<()> {
    let free = fs2::available_space(root)?;
    ensure!(
        free >= MIN_FREE && free.saturating_sub(MIN_FREE) >= needed,
        "insufficient free space (4 GiB reserve required)"
    );
    Ok(())
}
pub(crate) fn open_regular(path: &Path) -> Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure!(f.metadata()?.is_file(), "not a regular file");
    Ok(f)
}
pub(crate) fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = open_regular(path)?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut hash = blake3::Hasher::new();
    let mut size = 0;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((hash.finalize().to_hex().to_string(), size))
}
pub(crate) fn copy_verified_length(source: &Path, destination: &Path, size: u64) -> Result<()> {
    let mut input = open_regular(source)?;
    ensure!(input.metadata()?.len() == size, "source length changed");
    if !try_clone(source, destination)? {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(destination)?;
        let copied = std::io::copy(&mut input, &mut out)?;
        ensure!(copied == size, "source length changed during copy");
        out.sync_all()?;
    } else {
        File::open(destination)?.sync_all()?;
    }
    ensure!(
        fs::metadata(destination)?.len() == size,
        "copy length mismatch"
    );
    Ok(())
}
#[cfg(target_os = "macos")]
fn try_clone(source: &Path, target: &Path) -> Result<bool> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32)
        -> libc::c_int;
    }
    let src = CString::new(source.as_os_str().as_bytes())?;
    let dst = CString::new(target.as_os_str().as_bytes())?;
    // CLONE_NOFOLLOW: never follow a source symlink introduced during capture.
    if unsafe { clonefile(src.as_ptr(), dst.as_ptr(), 1) } == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(
            libc::EXDEV | libc::ENOTSUP | libc::EINVAL | libc::ENOSYS | libc::EPERM | libc::EACCES,
        ) => Ok(false),
        _ => Err(err.into()),
    }
}
#[cfg(not(target_os = "macos"))]
fn try_clone(_: &Path, _: &Path) -> Result<bool> {
    Ok(false)
}

pub(crate) fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension("json.part");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)?;
    File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}
