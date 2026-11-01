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
pub fn space(root: &Path, needed: u64) -> Result<()> {
    ensure!(
        fs2::available_space(root)?.saturating_sub(needed) >= RESERVE,
        "migration paused: 6 GiB free space reserve required"
    );
    Ok(())
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
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure!(f.metadata()?.is_file(), "expected a regular file");
    Ok(f)
}
pub fn hash(path: &Path) -> Result<(String, u64)> {
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
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
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
    size: u64,
    modified: std::time::SystemTime,
    inode: u64,
    device: u64,
}
pub fn walk(root: &Path) -> Result<std::collections::BTreeMap<PathBuf, Stamp>> {
    fn visit(
        base: &Path,
        p: &Path,
        out: &mut std::collections::BTreeMap<PathBuf, Stamp>,
    ) -> Result<()> {
        let m = fs::symlink_metadata(p)?;
        ensure!(
            !m.file_type().is_symlink(),
            "symlink in checkpoint source; coordinated export required"
        );
        if m.is_dir() {
            for e in fs::read_dir(p)? {
                visit(base, &e?.path(), out)?;
            }
        } else {
            ensure!(m.is_file(), "nonregular checkpoint source");
            out.insert(
                p.strip_prefix(base)?.into(),
                Stamp {
                    size: m.len(),
                    modified: m.modified()?,
                    inode: m.ino(),
                    device: m.dev(),
                },
            );
        }
        Ok(())
    }
    let mut out = std::collections::BTreeMap::new();
    if root.exists() {
        visit(root, root, &mut out)?;
    }
    Ok(out)
}
pub fn tree_bytes(root: &Path) -> Result<u64> {
    Ok(walk(root)?.values().map(|s| s.size).sum())
}
pub fn copy(source: &Path, target: &Path) -> Result<FileEntry> {
    let (expected, size) = hash(source)?;
    space(target.parent().context("missing parent")?, size)?;
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
        output.sync_all()?;
    }
    let mode = fs::metadata(source)?.permissions().mode() & 0o777;
    fs::set_permissions(target, fs::Permissions::from_mode(mode & 0o700))?;
    File::open(target)?.sync_all()?;
    ensure!(
        hash(target)? == (expected.clone(), size),
        "checkpoint content mismatch"
    );
    File::open(target.parent().unwrap())?.sync_all()?;
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
