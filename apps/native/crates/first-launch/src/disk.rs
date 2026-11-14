//! Private, durable migration files. Never opens a source for writing.
use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub const RESERVE: u64 = 6 * 1024 * 1024 * 1024;
/// Typed so the gate can show `Not Enough Space` with real byte counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LowDisk {
    pub needed: u64,
    pub available: u64,
}
impl std::fmt::Display for LowDisk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("migration paused: 6 GiB free space reserve required")
    }
}
impl std::error::Error for LowDisk {}
pub fn space(root: &Path, needed: u64) -> Result<()> {
    let available = fs2::available_space(root)?;
    if available.saturating_sub(needed) < RESERVE {
        return Err(LowDisk {
            needed: needed.saturating_add(RESERVE),
            available,
        }
        .into());
    }
    Ok(())
}
/// Up-front bytes a copy of `bytes` from `source` into `destination` will
/// really take. Same-volume APFS copies are clones that share blocks with the
/// source, so they cost nothing up front; each file that falls back to a
/// streamed copy still checks space before writing (see [`copy`]).
pub fn copy_cost(source: &Path, destination: &Path, bytes: u64) -> u64 {
    if pm_native_migrate::snapshot::clone_volume(source, destination) {
        0
    } else {
        bytes
    }
}
pub fn private_dir(path: &Path) -> Result<()> {
    // Check ancestors BEFORE mkdir: a beta folder replaced by a symlink must
    // never cause mkdir/chmod in the Electron profile or an external project.
    let mut ancestor = PathBuf::new();
    for component in path.components() {
        ancestor.push(component);
        if let Ok(meta) = fs::symlink_metadata(&ancestor) {
            ensure!(
                !meta.file_type().is_symlink(),
                "symlink destination ancestor"
            );
        }
    }
    fs::create_dir_all(path)?;
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "symlink destination"
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
pub fn regular(path: &Path) -> Result<File> {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        // Darwin SF_DATALESS: reading would materialize a cloud placeholder.
        // Preserve it as an unavailable input; never cause a download here.
        ensure!(
            !dataless(fs::symlink_metadata(path)?.st_flags()),
            "source is a dataless file; explicit source availability is required"
        );
    }
    let f = pm_native_migrate::source_access::open(
        path,
        OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW),
    )?;
    ensure!(f.metadata()?.is_file(), "expected a regular file");
    Ok(f)
}
pub fn hash(path: &Path) -> Result<(String, u64)> {
    hash_with(path, |_| {})
}
/// The pass whose bytes the running step counts: one per file it moves.
pub fn hash_counted(path: &Path) -> Result<(String, u64)> {
    hash_with(path, pm_native_migrate::source_access::add_bytes)
}
fn hash_with(path: &Path, mut read: impl FnMut(u64)) -> Result<(String, u64)> {
    let mut f = regular(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0; 1024 * 1024];
    let mut size = 0;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        read(n as u64);
        size += n as u64;
    }
    Ok((hasher.finalize().to_hex().to_string(), size))
}
pub fn json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_reader(regular(path)?)?)
}
pub fn atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    bytes(path, &serde_json::to_vec(value)?)
}
pub fn bytes(path: &Path, value: &[u8]) -> Result<()> {
    let tmp = path.with_extension("first-launch-part");
    if fs::symlink_metadata(&tmp).is_ok() {
        archive(&tmp)?;
    }
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    f.write_all(value)?;
    f.sync_all()?;
    drop(f);
    fs::rename(tmp, path)?;
    File::open(path.parent().context("missing parent")?)?.sync_all()?;
    Ok(())
}
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub hash: String,
    pub bytes: u64,
    pub mode: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub(crate) size: u64,
    modified: std::time::SystemTime,
    inode: u64,
    device: u64,
}
pub fn walk(root: &Path) -> Result<std::collections::BTreeMap<PathBuf, Stamp>> {
    walk_with(root, &[], Links::Refuse)
}
/// Walk an Electron source family. Links and other special files inside it
/// are left behind, never followed: agent work folders routinely hold Python
/// venvs, package caches and `node_modules/.bin` links, and none of those
/// can be part of a user's authored state. Copying a link could later let a
/// native write escape the beta profile, so a skipped link is the safe copy.
/// A family root that is itself a link (e.g. a folder the user moved to
/// another disk) is followed: reading through it never writes to the source.
pub(crate) fn walk_source(
    root: &Path,
    ignored: &[&str],
) -> Result<std::collections::BTreeMap<PathBuf, Stamp>> {
    walk_with(root, ignored, Links::Skip)
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Links {
    Refuse,
    Skip,
}
fn walk_with(
    root: &Path,
    ignored: &[&str],
    links: Links,
) -> Result<std::collections::BTreeMap<PathBuf, Stamp>> {
    fn visit(
        base: &Path,
        p: &Path,
        out: &mut std::collections::BTreeMap<PathBuf, Stamp>,
        ignored: &[&str],
        links: Links,
    ) -> Result<()> {
        let m = if links == Links::Skip && p == base {
            fs::metadata(p)?
        } else {
            fs::symlink_metadata(p)?
        };
        if m.file_type().is_symlink() {
            ensure!(
                links == Links::Skip,
                "symlink in checkpoint source; coordinated export required"
            );
            return Ok(());
        }
        if m.is_dir() {
            for e in fs::read_dir(p)? {
                let path = e?.path();
                if !ignored
                    .iter()
                    .any(|name| path.file_name().is_some_and(|n| n == *name))
                {
                    visit(base, &path, out, ignored, links)?;
                }
            }
        } else if m.is_file() {
            out.insert(
                p.strip_prefix(base)?.into(),
                Stamp {
                    size: m.len(),
                    modified: m.modified()?,
                    inode: m.ino(),
                    device: m.dev(),
                },
            );
        } else {
            // Sockets, pipes and devices hold no bytes worth carrying over.
            ensure!(links == Links::Skip, "nonregular checkpoint source");
        }
        Ok(())
    }
    let mut out = std::collections::BTreeMap::new();
    if root.exists() {
        visit(root, root, &mut out, ignored, links)?;
    }
    Ok(out)
}
pub fn tree_bytes(root: &Path) -> Result<u64> {
    Ok(walk(root)?.values().map(|s| s.size).sum())
}
pub fn copy(source: &Path, target: &Path) -> Result<FileEntry> {
    copy_with_sync(source, target, true)
}
/// Flush each staged file to the filesystem. The caller must issue a full
/// storage barrier before publishing the enclosing checkpoint/receipt.
pub(crate) fn copy_staged(source: &Path, target: &Path) -> Result<FileEntry> {
    copy_with_sync(source, target, false)
}
pub(crate) fn sync_staged(file: &File) -> Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: the descriptor is live and borrowed for this filesystem flush.
    if unsafe { libc::fsync(file.as_raw_fd()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
fn copy_with_sync(source: &Path, target: &Path, full: bool) -> Result<FileEntry> {
    let sync = |file: &File| -> Result<()> {
        if full {
            file.sync_all()?;
            Ok(())
        } else {
            sync_staged(file)
        }
    };
    let (expected, size) = hash_counted(source)?;
    // A clone shares the source's blocks, so free space is checked only
    // before bytes are actually streamed.
    // clonefile keeps APFS snapshots cheap. Stream on other filesystems.
    #[cfg(target_os = "macos")]
    let cloned = {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        unsafe extern "C" {
            fn clonefile(a: *const libc::c_char, b: *const libc::c_char, flags: u32)
            -> libc::c_int;
        }
        let a = CString::new(source.as_os_str().as_bytes())?;
        let b = CString::new(target.as_os_str().as_bytes())?;
        // SAFETY: valid NUL-terminated paths; CLONE_NOFOLLOW forbids source symlinks.
        unsafe { clonefile(a.as_ptr(), b.as_ptr(), 1) == 0 }
    };
    #[cfg(not(target_os = "macos"))]
    let cloned = false;
    if !cloned {
        space(target.parent().context("missing parent")?, size)?;
        let mut input = regular(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(target)?;
        ensure!(
            std::io::copy(&mut input, &mut output)? == size,
            "checkpoint source changed"
        );
        sync(&output)?;
    }
    let mode = fs::metadata(source)?.permissions().mode() & 0o777;
    fs::set_permissions(target, fs::Permissions::from_mode(mode & 0o700))?;
    sync(&File::open(target)?)?;
    ensure!(
        hash(target)? == (expected.clone(), size),
        "checkpoint content mismatch"
    );
    sync(&File::open(target.parent().unwrap())?)?;
    Ok(FileEntry {
        hash: expected,
        bytes: size,
        mode,
    })
}
/// Resolve existing ancestors before creating anything. Symlink ancestors cannot
/// smuggle a destination into the source profile.
pub fn resolved(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "profile paths must be absolute");
    ensure!(
        path.components()
            .all(|c| !matches!(c, std::path::Component::ParentDir)),
        "parent traversal in profile path"
    );
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = resolved(path.parent().context("profile has no parent")?)?;
    Ok(parent.join(path.file_name().context("profile has no name")?))
}

/// Retain an owned attempt under a fresh sibling name. Caller holds the migration
/// lease. Never follows links or removes a prior archive, even after a no-op delta.
pub fn archive(path: &Path) -> Result<PathBuf> {
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "linked attempt"
    );
    let parent = path.parent().context("attempt has no parent")?;
    let name = path
        .file_name()
        .context("attempt has no name")?
        .to_string_lossy();
    for sequence in 0u64.. {
        let archived = parent.join(format!("{name}.archive-{sequence:016x}"));
        if fs::symlink_metadata(&archived).is_ok() {
            continue;
        }
        fs::rename(path, &archived)?;
        File::open(parent)?.sync_all()?;
        return Ok(archived);
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_state_append_is_archived_before_atomic_retry() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let state = root.join("migration.json");
        let partial = state.with_extension("first-launch-part");
        fs::write(&partial, b"synthetic interrupted state").unwrap();
        atomic(&state, &serde_json::json!({"version":1})).unwrap();
        assert_eq!(
            fs::read(root.join("migration.first-launch-part.archive-0000000000000000")).unwrap(),
            b"synthetic interrupted state"
        );
        assert_eq!(json::<serde_json::Value>(&state).unwrap()["version"], 1);
    }
}

/// Darwin stat.h SF_DATALESS. Pure flag policy is exercised with Linux doubles;
/// the actual macOS metadata/provider behavior remains a signed Mac check.
#[cfg(any(test, target_os = "macos"))]
fn dataless(flags: u32) -> bool {
    flags & 0x4000_0000 != 0
}
#[cfg(test)]
mod dataless_tests {
    #[test]
    fn cloud_placeholders_are_not_read_as_local_bytes() {
        assert!(super::dataless(0x4000_0000));
        assert!(super::dataless(0x4000_0001));
        assert!(!super::dataless(0));
        assert!(!super::dataless(1));
    }
}
