//! Durable file helpers: file identity, conflict checks, sibling temp files,
//! atomic publication, and version backups.
//!
//! Ported from `main:apps/desktop/src/main/durable-file.ts` (`fileVersion`,
//! `checkVersion`, `temporarySibling`, `syncFile`, `syncDirectory`,
//! `copySnapshot`, `publishFile`) and the backup half of `project-files.ts`
//! (`backup`, `retain`).
//!
//! The rules, as in the old app:
//!
//! * A file is "the same" only with the same device, inode, size, mode,
//!   owner, and timestamps. Changed timestamps on the same inode are excused
//!   only by proof of identical bytes (a full re-hash); a timestamp tolerance
//!   or an equal hash on a *different* inode never excuses anything.
//! * Replacing a file uses the macOS atomic exchange (`renamex_np` with
//!   `RENAME_SWAP`) and then judges the *displaced* file: the version we
//!   expected is dropped, anything else is kept beside the project. Where
//!   the volume has no exchange the file is moved aside under a private name
//!   and the new one installed at the empty path ([`replace_aside`]). No
//!   route removes a file before it was read and found to be that version.
//! * Every save writes its record before it moves anything
//!   ([`super::transaction`]); a save that was cut short is settled from it.
//! * No file is ever renamed onto a public name (the project's, or the name
//!   a kept file gets): a name is taken by the exclusive rename
//!   (`RENAME_EXCL`), by a hard link, or by exclusive creation and a verified
//!   copy, all of which fail if anything appeared there ([`place_new`]).

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::transaction::{self, Route, Transaction};
use crate::error::{ProjectIoError, io_err};

/// What identifies a file on disk (`fs.Stats` identity in the old app).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub mtime_ns: i128,
    pub ctime_ns: i128,
}

impl FileStat {
    /// `sameIdentity`: the same file object.
    pub fn same_identity(&self, other: &FileStat) -> bool {
        self.dev == other.dev && self.ino == other.ino
    }

    /// `samePermissions`.
    pub fn same_permissions(&self, other: &FileStat) -> bool {
        self.mode == other.mode && self.uid == other.uid && self.gid == other.gid
    }

    /// `sameFile`: identity, size and both timestamps.
    pub fn same_file(&self, other: &FileStat) -> bool {
        self.same_identity(other)
            && self.size == other.size
            && self.mtime_ns == other.mtime_ns
            && self.ctime_ns == other.ctime_ns
    }
}

#[cfg(unix)]
pub(crate) fn stat_of(meta: &fs::Metadata) -> FileStat {
    use std::os::unix::fs::MetadataExt;
    FileStat {
        dev: meta.dev(),
        ino: meta.ino(),
        size: meta.len(),
        mode: meta.mode(),
        uid: meta.uid(),
        gid: meta.gid(),
        mtime_ns: meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128,
        ctime_ns: meta.ctime() as i128 * 1_000_000_000 + meta.ctime_nsec() as i128,
    }
}

#[cfg(not(unix))]
pub(crate) fn stat_of(meta: &fs::Metadata) -> FileStat {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i128);
    FileStat {
        dev: 0,
        ino: 0,
        size: meta.len(),
        mode: 0,
        uid: 0,
        gid: 0,
        mtime_ns: mtime,
        ctime_ns: mtime,
    }
}

/// A file exactly as we last read or wrote it: its identity and the SHA-256
/// of its whole content (`FileVersion` in the old app).
#[derive(Clone, Debug)]
pub struct FileVersion {
    pub path: PathBuf,
    pub stat: FileStat,
    /// Lowercase hex SHA-256 of the whole file.
    pub hash: String,
    /// The descriptor the file was read (or written) through. It stays the
    /// same file when the path stops naming it (renamed, moved, deleted
    /// while open), so the session can follow it and its embedded media can
    /// still be copied out. Not part of the version's identity.
    pub held: Option<Arc<HeldFile>>,
}
impl PartialEq for FileVersion {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.stat == other.stat && self.hash == other.hash
    }
}
impl Eq for FileVersion {}

/// An open descriptor of a file version and the last state its bytes were
/// proven at.
#[derive(Debug)]
pub struct HeldFile {
    file: File,
    proven: std::sync::Mutex<Option<FileStat>>,
}
impl HeldFile {
    pub(crate) fn new(file: File) -> Arc<Self> {
        Arc::new(Self {
            file,
            proven: std::sync::Mutex::new(None),
        })
    }
    pub(crate) fn file(&self) -> &File {
        &self.file
    }
    fn stat(&self) -> io::Result<FileStat> {
        Ok(stat_of(&self.file.metadata()?))
    }
    /// Where the system says this file is now (it follows a rename or a
    /// move on its volume). `None` when it has no name any more.
    #[cfg(target_os = "macos")]
    pub(crate) fn current_path(&self) -> Option<PathBuf> {
        use std::os::{fd::AsRawFd, unix::ffi::OsStringExt};
        let mut buf = vec![0u8; libc::PATH_MAX as usize];
        // SAFETY: a live descriptor and a PATH_MAX buffer, as F_GETPATH asks.
        if unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } != 0 {
            return None;
        }
        let len = buf.iter().position(|b| *b == 0)?;
        buf.truncate(len);
        let linked = self.file.metadata().ok().is_some_and(|m| {
            use std::os::unix::fs::MetadataExt;
            m.nlink() > 0
        });
        linked.then(|| PathBuf::from(std::ffi::OsString::from_vec(buf)))
    }
    #[cfg(not(target_os = "macos"))]
    pub(crate) fn current_path(&self) -> Option<PathBuf> {
        None
    }
}

/// Whether `path` names a file the editor may treat as a project file:
/// `Ok(None)` when nothing is there, an error for a symlink or directory
/// (the old app never wrote through a symlink supplied as a destination).
pub(crate) fn lstat_file(path: &Path) -> Result<Option<FileStat>, ProjectIoError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(Some(stat_of(&meta))),
        Ok(_) => Err(ProjectIoError::Invalid(
            "The destination cannot be safely backed up. Choose another file with Save As.".into(),
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err(path, e)),
    }
}

impl FileVersion {
    /// `fileVersion`: stats and hashes `path`, refusing a file that changes
    /// while it is being hashed. `Ok(None)` when nothing is there.
    pub fn read(path: &Path) -> Result<Option<FileVersion>, ProjectIoError> {
        let Some(info) = lstat_file(path)? else {
            return Ok(None);
        };
        let file = File::open(path).map_err(|e| io_err(path, e))?;
        let opened = stat_of(&file.metadata().map_err(|e| io_err(path, e))?);
        if !info.same_file(&opened) {
            return Err(ProjectIoError::ChangedOnDisk);
        }
        let hash = hex(&hash_reader(&file, path)?.finalize());
        let after = stat_of(&file.metadata().map_err(|e| io_err(path, e))?);
        let relinked = lstat_file(path)?;
        if !info.same_file(&after) || relinked.as_ref().is_none_or(|s| !info.same_file(s)) {
            return Err(ProjectIoError::ChangedOnDisk);
        }
        Ok(Some(FileVersion {
            path: path.to_owned(),
            stat: info,
            hash,
            held: Some(HeldFile::new(file)),
        }))
    }

    /// The descriptor held since this version was read, if the file behind
    /// it still has exactly this version's bytes: the same inode and size,
    /// and either untouched timestamps or (a rename or move touches them) a
    /// whole-file hash equal to the version's, proven once per state.
    pub(crate) fn held_unchanged(&self) -> Result<Option<&File>, ProjectIoError> {
        let Some(held) = &self.held else {
            return Ok(None);
        };
        let Ok(now) = held.stat() else {
            return Ok(None);
        };
        if !now.same_identity(&self.stat) || now.size != self.stat.size {
            return Ok(None);
        }
        let mut proven = held.proven.lock().unwrap();
        if now.same_file(&self.stat) || proven.as_ref() == Some(&now) {
            return Ok(Some(held.file()));
        }
        // Proven for the state it was read in: the descriptor's stat must be
        // the same after the read as before it.
        if hex(&hash_reader(held.file(), &self.path)?.finalize()) != self.hash
            || held.stat().ok().as_ref() != Some(&now)
        {
            return Ok(None);
        }
        *proven = Some(now);
        Ok(Some(held.file()))
    }

    /// This version at the path the file has now (it was renamed or moved
    /// while open): the same inode and bytes, found through the descriptor
    /// held since it was read. `None` if the path still names it, it has no
    /// name any more, its bytes changed, or it now sits somewhere nobody
    /// keeps documents ([`is_document_place`]: a Trash, a bundle, a hidden or
    /// system folder), where a save must be the person's choice (Save As),
    /// never something that follows the file there on its own.
    pub fn moved(&self) -> Result<Option<FileVersion>, ProjectIoError> {
        if lstat_file(&self.path)
            .ok()
            .flatten()
            .is_some_and(|at| at.same_identity(&self.stat))
        {
            return Ok(None);
        }
        let Some(path) = self.held.as_ref().and_then(|held| held.current_path()) else {
            return Ok(None);
        };
        let Some(at) = lstat_file(&path).ok().flatten() else {
            return Ok(None);
        };
        if !is_document_place(&path)
            || transaction::is_private_name(&path)
            || !at.same_identity(&self.stat)
            || self.held_unchanged()?.is_none()
        {
            return Ok(None);
        }
        Ok(Some(FileVersion {
            path,
            stat: at,
            hash: self.hash.clone(),
            held: self.held.clone(),
        }))
    }

    /// `checkVersion(path, this, verify_content)`: errors with
    /// [`ProjectIoError::ChangedOnDisk`] unless `path` is still this exact
    /// file. Changed timestamps (or `verify_content`) force a re-hash on the
    /// same inode.
    pub fn check(&self, verify_content: bool) -> Result<(), ProjectIoError> {
        check_version(&self.path, Some(self), verify_content)
    }

    /// [`FileVersion::check`] as a boolean (I/O errors are still errors).
    pub fn is_unchanged(&self) -> Result<bool, ProjectIoError> {
        match self.check(false) {
            Ok(()) => Ok(true),
            Err(ProjectIoError::ChangedOnDisk) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// Whether `path` is somewhere a person keeps documents: not inside a
/// hidden folder (a Trash is one), not inside a bundle or another package
/// (known by the folder's extension), not in a system folder, and not in the
/// user's Library other than the folders sync clients keep documents in.
pub(crate) fn is_document_place(path: &Path) -> bool {
    const PACKAGES: &[&str] = &[
        "app",
        "appex",
        "bundle",
        "framework",
        "plugin",
        "kext",
        "xpc",
        "pkg",
        "mpkg",
        "prefpane",
        "saver",
        "qlgenerator",
        "mdimporter",
        "component",
        "wdgt",
        "dsym",
        "lproj",
        "xcodeproj",
        "xcworkspace",
        "xcassets",
        "playground",
        "photoslibrary",
        "aplibrary",
        "imovielibrary",
        "tvlibrary",
        "musiclibrary",
        "fcpbundle",
        "logicx",
        "band",
        "rtfd",
        "key",
        "pages",
        "numbers",
        "sparsebundle",
    ];
    const SYSTEM: &[&str] = &[
        "/System",
        "/Library",
        "/Applications",
        "/usr",
        "/bin",
        "/sbin",
        "/opt",
        "/etc",
        "/private/etc",
    ];
    if SYSTEM.iter().any(|folder| path.starts_with(folder)) {
        return false;
    }
    if let Some(library) = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library"))
        && path.starts_with(&library)
        && !path.starts_with(library.join("Mobile Documents"))
        && !path.starts_with(library.join("CloudStorage"))
    {
        return false;
    }
    let Some(folder) = path.parent() else {
        return true;
    };
    !folder.components().any(|c| {
        let std::path::Component::Normal(name) = c else {
            return false;
        };
        let name = name.to_string_lossy();
        name.starts_with('.')
            || Path::new(&*name)
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|e| PACKAGES.iter().any(|p| p.eq_ignore_ascii_case(e)))
    })
}

/// `checkVersion(destination, expected, verifyContent)`. With no expected
/// version the destination must not exist.
pub fn check_version(
    destination: &Path,
    expected: Option<&FileVersion>,
    verify_content: bool,
) -> Result<(), ProjectIoError> {
    let current = lstat_file(destination).map_err(|e| match e {
        // A directory or symlink where a file was is a change too.
        ProjectIoError::Invalid(_) => ProjectIoError::ChangedOnDisk,
        other => other,
    })?;
    let Some(expected) = expected else {
        return match current {
            Some(_) => Err(ProjectIoError::ChangedOnDisk),
            None => Ok(()),
        };
    };
    let Some(current) = current else {
        return Err(ProjectIoError::ChangedOnDisk);
    };
    if !expected.stat.same_identity(&current)
        || !expected.stat.same_permissions(&current)
        || expected.stat.size != current.size
    {
        return Err(ProjectIoError::ChangedOnDisk);
    }
    if verify_content || !expected.stat.same_file(&current) {
        let verified = FileVersion::read(destination)?.ok_or(ProjectIoError::ChangedOnDisk)?;
        if !expected.stat.same_identity(&verified.stat)
            || !expected.stat.same_permissions(&verified.stat)
            || verified.hash != expected.hash
        {
            return Err(ProjectIoError::ChangedOnDisk);
        }
    }
    Ok(())
}

/// SHA-256 of everything behind `file`, read by position from its start. A
/// descriptor's cursor belongs to whatever read through it last (a held
/// descriptor is hashed more than once, and its duplicates share the
/// cursor), so it says nothing about where the file begins.
pub(crate) fn hash_reader(file: &File, path: &Path) -> Result<Sha256, ProjectIoError> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut offset = 0u64;
    loop {
        #[cfg(unix)]
        let n = {
            use std::os::unix::fs::FileExt;
            read_retry(|| file.read_at(&mut buf, offset))
        };
        #[cfg(not(unix))]
        let n = {
            use std::io::{Seek, SeekFrom};
            let mut reader = file;
            reader
                .seek(SeekFrom::Start(offset))
                .and_then(|_| read_retry(|| reader.read(&mut buf)))
        };
        let n = n.map_err(|e| io_err(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        offset += n as u64;
    }
    Ok(hasher)
}

/// SHA-256 of a whole file, lowercase hex.
pub fn hash_file(path: &Path) -> Result<String, ProjectIoError> {
    let file = File::open(path).map_err(|e| io_err(path, e))?;
    Ok(hex(&hash_reader(&file, path)?.finalize()))
}

/// `readWithRetry`: macOS can transiently fail a read with `EDEADLK`;
/// retrying the same offset is safe because a failed read returned nothing.
pub(crate) fn read_retry<T>(mut read: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut attempt = 0;
    loop {
        match read() {
            Err(e) if e.raw_os_error() == Some(11) && attempt < 8 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(5 << attempt));
            }
            other => return other,
        }
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

/// 16 random bytes from the system generator, formatted as a UUID v4 (the old
/// app used `crypto.randomUUID()` for temp names and media revisions).
pub(crate) fn random_uuid() -> String {
    let mut bytes = [0u8; 16];
    let filled = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if filled.is_err() {
        // Not cryptographic; uniqueness only (temp names are also created
        // exclusively, and revisions only need to differ).
        use std::hash::{BuildHasher, Hasher};
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for (i, chunk) in bytes.chunks_mut(8).enumerate() {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(nanos);
            h.write_usize(i);
            chunk.copy_from_slice(&h.finish().to_le_bytes());
        }
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// `temporarySibling`: `.<name>.<uuid>.tmp` next to `destination` (the same
/// volume, so publication is a rename, never a copy).
pub fn temporary_sibling(destination: &Path) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(destination.file_name().unwrap_or("project".as_ref()));
    name.push(format!(".{}.tmp", random_uuid()));
    destination
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(name)
}

/// A staged file that is removed when dropped unless [`TempFile::keep`]
/// was called (or it was renamed away).
pub(crate) struct TempFile {
    pub path: PathBuf,
    keep: bool,
}

impl TempFile {
    /// Creates `path`'s sibling exclusively with mode 0600.
    pub fn create(destination: &Path) -> Result<(TempFile, File), ProjectIoError> {
        let path = temporary_sibling(destination);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(|e| io_err(&path, e))?;
        Ok((TempFile { path, keep: false }, file))
    }

    /// `copySnapshot(source, sibling)`: an APFS clone where possible (Rust's
    /// `fs::copy` uses `fclonefileat` on macOS), a byte copy otherwise.
    pub fn clone_of(source: &Path, destination: &Path) -> Result<TempFile, ProjectIoError> {
        let path = temporary_sibling(destination);
        let temp = TempFile { path, keep: false };
        fs::copy(source, &temp.path).map_err(|e| io_err(&temp.path, e))?;
        Ok(temp)
    }

    /// Leaves the file on disk (it was published or is a recovery copy).
    pub fn keep(mut self) -> PathBuf {
        self.keep = true;
        std::mem::take(&mut self.path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.keep && !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// `syncFile`: Rust's `File::sync_all` is `fcntl(F_FULLFSYNC)` on Apple
/// platforms, the old native `fullSync`.
pub fn sync_file(file: &File) -> std::io::Result<()> {
    file.sync_all()
}

/// How far a folder's entries (a rename, a new name) can be made to
/// survive a power cut, weakest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FolderSync {
    /// The volume has no way to ask (neither sync exists for a folder).
    Unproven,
    /// Only the plain sync, which hands the entries to the volume or the
    /// server without proof they are on stable storage (a network share).
    Plain,
    /// The full sync (`F_FULLFSYNC`).
    Full,
}

/// What a folder answered when it was synced: how far, and the error
/// numbers by which it said a kind of sync does not exist there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Folder {
    pub sync: FolderSync,
    unsupported: (Option<i32>, Option<i32>),
}

/// `syncDirectory`: makes a rename/link durable ([`folder_sync`]), wherever
/// the volume allows.
pub fn sync_directory(dir: &Path) -> Result<(), ProjectIoError> {
    folder_sync(dir, &PublishHooks::default()).map(drop)
}

/// Syncs a folder so that its entries survive a power cut. The full sync
/// first; a volume that has none for folders (a network share answers
/// "Operation not supported") gets the plain sync, and where even that does
/// not exist nothing can be asked. Only those two answers mean "does not
/// exist"; every other failure of either sync (an I/O error, for one) is a
/// real failure and is returned.
pub(crate) fn folder_sync(dir: &Path, hooks: &PublishHooks) -> Result<Folder, ProjectIoError> {
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let handle = File::open(dir).map_err(|e| io_err(dir, e))?;
    let (full, plain) = hooks.folder_sync.as_ref().map_or((None, None), |f| f());
    let injected = |errno: i32| Err(io::Error::from_raw_os_error(errno));
    let full = match full.map_or_else(|| handle.sync_all(), injected) {
        Ok(()) => {
            return Ok(Folder {
                sync: FolderSync::Full,
                unsupported: (None, None),
            });
        }
        Err(e) if folder_sync_unsupported(&e) => e.raw_os_error(),
        Err(e) => return Err(io_err(dir, e)),
    };
    #[cfg(unix)]
    let synced = {
        use std::os::fd::AsRawFd;
        // SAFETY: a live descriptor.
        match unsafe { libc::fsync(handle.as_raw_fd()) } {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    };
    #[cfg(not(unix))]
    let synced: io::Result<()> = Ok(());
    match plain.map_or(synced, injected) {
        Ok(()) => Ok(Folder {
            sync: FolderSync::Plain,
            unsupported: (full, None),
        }),
        Err(e) if folder_sync_unsupported(&e) => Ok(Folder {
            sync: FolderSync::Unproven,
            unsupported: (full, e.raw_os_error()),
        }),
        Err(e) => Err(io_err(dir, e)),
    }
}

/// Syncs `dir` as [`folder_sync`] does; an error only for a real failure.
pub(crate) fn folder_sync_now(dir: &Path, hooks: &PublishHooks) -> Result<(), ProjectIoError> {
    folder_sync(dir, hooks).map(drop)
}

/// [`folder_sync`] after a save installed its file, held to what the folder
/// answered when the save began (`probed`): "does not exist" is believed
/// only from a folder that said so then, with the same error. A folder
/// that could be synced then and cannot now has failed.
fn folder_sync_as(dir: &Path, probed: Folder, hooks: &PublishHooks) -> Result<(), ProjectIoError> {
    let now = folder_sync(dir, hooks)?;
    if now == probed {
        Ok(())
    } else {
        Err(ProjectIoError::Invalid(format!(
            "{}: the folder could not be synced as it was when the save began",
            dir.display()
        )))
    }
}

/// ENOTSUP, or EINVAL from a descriptor that cannot be synced.
fn folder_sync_unsupported(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(45) | Some(22))
}

#[cfg(test)]
mod sync_tests {
    #[test]
    fn a_folder_that_cannot_be_synced_is_not_an_error_and_a_real_failure_is() {
        use std::io::Error;
        assert!(super::folder_sync_unsupported(&Error::from_raw_os_error(
            45
        )));
        assert!(!super::folder_sync_unsupported(&Error::from_raw_os_error(
            5
        )));
        assert!(!super::folder_sync_unsupported(&Error::from_raw_os_error(
            28
        )));
    }
}

#[cfg(target_os = "macos")]
mod sys {
    //! `renamex_np` (macOS 10.12+): `RENAME_SWAP` exchanges two names
    //! atomically, `RENAME_EXCL` renames only when the target does not exist.
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int, c_uint};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub const RENAME_SWAP: c_uint = 0x0000_0002;
    pub const RENAME_EXCL: c_uint = 0x0000_0004;

    unsafe extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
    }

    pub fn renamex(from: &Path, to: &Path, flags: c_uint) -> std::io::Result<()> {
        let from = CString::new(from.as_os_str().as_bytes())?;
        let to = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both pointers are valid NUL-terminated strings for the
        // duration of the call; the function does not retain them.
        let rc = unsafe { renamex_np(from.as_ptr(), to.as_ptr(), flags) };
        if rc == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

/// Something a test does to the destination at a point of a publication,
/// as a sync client or another machine would.
#[doc(hidden)]
pub type PublishHook = Arc<dyn Fn(&Path) + Send + Sync>;

/// Something a test does when a save's record reaches a step.
#[doc(hidden)]
pub type StepHook = Arc<dyn Fn(&str, &Path) + Send + Sync>;

/// Moves a file to the person's Trash (on macOS, `NSFileManager`'s
/// `trashItemAtURL`) and says where it went. An error where the volume has
/// no Trash (a network share) or the move is refused.
#[derive(Clone)]
pub struct TrashHook(pub Arc<TrashFn>);
/// What a [`TrashHook`] calls.
pub type TrashFn = dyn Fn(&Path) -> io::Result<PathBuf> + Send + Sync;
impl std::fmt::Debug for TrashHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Trash")
    }
}

/// A file found changed when it was to go that could not be kept beside its
/// project: where it is now, for the person to be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stray {
    /// The project file it was a version of.
    pub project: PathBuf,
    /// Where it is: in the Trash (`trashed`), or still under the private
    /// name it had beside the project.
    pub path: PathBuf,
    pub trashed: bool,
    /// The save's record tells the person (it could be written).
    pub recorded: bool,
}

/// Where saves and recoveries report their [`Stray`]s; the kernel collects
/// them once the call that ran the save or the recovery returns.
pub type Strays = Arc<std::sync::Mutex<Vec<Stray>>>;

/// What a test makes a folder's syncs answer.
#[doc(hidden)]
pub type FolderSyncHook = Arc<dyn Fn() -> (Option<i32>, Option<i32>) + Send + Sync>;

/// Test hooks on publication.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct PublishHooks {
    /// The volume has no atomic exchange (exFAT, HFS+, most network shares).
    pub no_exchange: bool,
    /// The volume answers the exchange with success but only renames the
    /// first name over the second (a FAT32 volume on macOS 27).
    pub exchange_renames: bool,
    /// The volume has neither an exclusive rename nor hard links (exFAT, a
    /// network share).
    pub no_exclusive: bool,
    /// The volume has hard links but no exclusive rename.
    pub links_only: bool,
    /// Called with the destination after its last check, right before the
    /// file there is taken (exchanged with the staged file, or moved aside).
    pub before_exchange: Option<PublishHook>,
    /// Called with the destination while it holds nothing: the file was
    /// moved aside and the staged one is not installed yet.
    pub between_renames: Option<PublishHook>,
    /// Called with the destination right after the staged file is there.
    pub after_exchange: Option<PublishHook>,
    /// Called with a name right before a file is placed under it (the
    /// project's own name, or the name a kept file gets): after every look
    /// at that name, immediately before the call that takes it.
    pub before_install: Option<PublishHook>,
    /// Called with the private name of the file a save replaced, right
    /// after that file was read and found to be the version the save could
    /// replace.
    pub after_judged: Option<PublishHook>,
    /// The save's record says it was written on this machine (its name and
    /// identifier) instead of the real one.
    pub owner_host: Option<String>,
    /// While set, the volume answers the exchange with success and only
    /// renames, as [`Self::exchange_renames`]: a volume whose behaviour
    /// changed while the app ran (unmounted, and another mounted in its
    /// place).
    pub weak_exchange: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Called with a step's name and the destination when a save's record
    /// reaches that step (`begin`, `aside`, `install`, `exchanged`,
    /// `restore`, `keep`, `closed`, ...): the step is recorded, and not yet
    /// taken.
    pub step: Option<StepHook>,
    /// What the next sync of a folder answers, as error numbers: the full
    /// sync's and the plain sync's (`None`: the real call).
    pub folder_sync: Option<FolderSyncHook>,
    /// Called with the destination half way through a copy into an
    /// exclusively created name.
    pub mid_copy: Option<PublishHook>,
    /// Called with the destination of a copy into an exclusively created
    /// name at the moment exactly this many bytes of it were written.
    pub copy_stops_at: Option<(u64, PublishHook)>,
    /// While set, the next copy into an exclusively created name stops
    /// half way with "No space left on device" (and clears it).
    pub copy_fails: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Called with a file right before it is disposed of ([`dispose`]):
    /// after it was last read, immediately before it goes to the Trash.
    pub before_disposal: Option<PublishHook>,
    /// Called with the name a file had when it was read to be disposed of,
    /// after that read and right before the file is unlinked.
    pub before_unlink: Option<PublishHook>,
    /// Called with a public name right after the file under it was moved
    /// to a private name of the save's record, before that file is judged.
    pub after_withdrawn: Option<PublishHook>,
    /// While it answers true for a step's name, that step's line cannot be
    /// written to the save's record (a full or failing disk).
    pub record_fails: Option<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
    /// The person's Trash, where a version of the project goes once a save
    /// no longer needs it (the kernel's host sets it; `None`: such files
    /// stay where they are).
    pub trash: Option<TrashHook>,
    /// Not a test hook: where files that could not be kept beside their
    /// project are reported ([`Stray`]).
    pub strays: Strays,
}
impl std::fmt::Debug for PublishHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublishHooks")
            .field("no_exchange", &self.no_exchange)
            .field("exchange_renames", &self.exchange_renames)
            .field("no_exclusive", &self.no_exclusive)
            .field("links_only", &self.links_only)
            .field("before_exchange", &self.before_exchange.is_some())
            .field("between_renames", &self.between_renames.is_some())
            .field("after_exchange", &self.after_exchange.is_some())
            .field("before_install", &self.before_install.is_some())
            .field("after_judged", &self.after_judged.is_some())
            .field("owner_host", &self.owner_host)
            .field("weak_exchange", &self.weak_exchange.is_some())
            .field("step", &self.step.is_some())
            .field("folder_sync", &self.folder_sync.is_some())
            .field("mid_copy", &self.mid_copy.is_some())
            .field("copy_stops_at", &self.copy_stops_at.is_some())
            .field("copy_fails", &self.copy_fails.is_some())
            .field("before_disposal", &self.before_disposal.is_some())
            .field("before_unlink", &self.before_unlink.is_some())
            .field("after_withdrawn", &self.after_withdrawn.is_some())
            .field("trash", &self.trash.is_some())
            .finish()
    }
}

/// `swapFiles`: `Ok(true)` when exchanged, `Ok(false)` when the volume or
/// platform has no atomic exchange (nothing was moved).
fn swap_files(a: &Path, b: &Path, hooks: &PublishHooks) -> Result<bool, ProjectIoError> {
    if hooks.no_exchange {
        return Ok(false);
    }
    if hooks.exchange_renames
        || hooks
            .weak_exchange
            .as_ref()
            .is_some_and(|weak| weak.load(std::sync::atomic::Ordering::SeqCst))
    {
        fs::rename(a, b).map_err(|e| io_err(b, e))?;
        return Ok(true);
    }
    #[cfg(target_os = "macos")]
    {
        match sys::renamex(a, b, sys::RENAME_SWAP) {
            Ok(()) => Ok(true),
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(45 /* ENOTSUP */) | Some(22 /* EINVAL */)
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(io_err(b, e)),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (a, b);
        Ok(false)
    }
}

/// What folders were proven to do: (which call, the folder) and the answer.
type Probes = Vec<((&'static str, PathBuf), bool)>;

thread_local! {
    /// What the folders probed by the publication or recovery this thread
    /// is in the middle of were proven to do ([`probing`]).
    static PROBED: std::cell::RefCell<Option<Probes>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `work`, one publication or one recovery, remembering what its
/// volume is proven to do ([`exchanges`], [`excludes`]) for that long and no
/// longer. Nothing is remembered from one save to the next, or from an open
/// to a save: a volume can be unmounted and another mounted in its place
/// under the same name, device number and type while the app runs, and a
/// proof that the first one exchanges files says nothing about the second.
/// An answer relied on wrongly destroys a file, so each save proves it
/// afresh (two one-byte files beside the project, a few calls).
pub(super) fn probing<T>(work: impl FnOnce() -> T) -> T {
    struct Forget(Option<Probes>);
    impl Drop for Forget {
        fn drop(&mut self) {
            PROBED.with(|probed| *probed.borrow_mut() = self.0.take());
        }
    }
    let _forget = Forget(PROBED.with(|probed| probed.borrow_mut().replace(Vec::new())));
    work()
}

/// A call the volume was proven to have just answered that it has not: what
/// was proven no longer holds, and is proven again before it is relied on.
fn forget_probes() {
    PROBED.with(|probed| {
        if let Some(probed) = probed.borrow_mut().as_mut() {
            probed.clear();
        }
    });
}

/// Runs `probe` on two one-byte staging files beside `destination`, unless
/// the publication or recovery in progress already did ([`probing`]). An
/// error is the staging files' own (a folder that cannot be written) and is
/// not remembered.
fn proven(
    what: &'static str,
    destination: &Path,
    probe: impl FnOnce(&TempFile, &TempFile) -> Result<bool, ProjectIoError>,
) -> Result<bool, ProjectIoError> {
    let key = (
        what,
        destination.parent().unwrap_or(Path::new(".")).to_owned(),
    );
    let known = PROBED.with(|probed| {
        let probed = probed.borrow();
        let (_, does) = probed.as_ref()?.iter().find(|(k, _)| *k == key)?;
        Some(*does)
    });
    if let Some(does) = known {
        return Ok(does);
    }
    // Told apart by a byte of content each: an empty file has no stable
    // inode number on FAT-family volumes.
    let stage = |text: &[u8]| -> Result<TempFile, ProjectIoError> {
        use std::io::Write;
        let (temp, mut file) = TempFile::create(destination)?;
        file.write_all(text).map_err(|e| io_err(&temp.path, e))?;
        Ok(temp)
    };
    let does = probe(&stage(b"a")?, &stage(b"b")?)?;
    PROBED.with(|probed| {
        if let Some(probed) = probed.borrow_mut().as_mut() {
            probed.push((key, does));
        }
    });
    Ok(does)
}

fn holds(path: &Path, text: &[u8]) -> bool {
    fs::read(path).is_ok_and(|read| read == text)
}

/// Whether the volume `destination` is on really exchanges two names
/// atomically, proven by doing it: two one-byte staging files are exchanged
/// and each must come back under the other's name. The call's own answer is not
/// proof. One filesystem driver answers it with success and renames instead,
/// which destroys whatever was at the destination (measured on a FAT32
/// volume).
pub(crate) fn exchanges(destination: &Path, hooks: &PublishHooks) -> Result<bool, ProjectIoError> {
    proven("exchange", destination, |a, b| {
        Ok(swap_files(&a.path, &b.path, hooks)? && holds(&a.path, b"b") && holds(&b.path, b"a"))
    })
}

/// Whether the volume `destination` is on has a rename that refuses a name
/// something holds, proven by doing it: onto a taken name it must fail and
/// leave both files, and onto a free name it must rename. (One volume type
/// refuses the taken name and then cannot do the rename at all: exFAT.)
fn excludes(destination: &Path, hooks: &PublishHooks) -> Result<bool, ProjectIoError> {
    if hooks.no_exclusive || hooks.links_only {
        return Ok(false);
    }
    #[cfg(target_os = "macos")]
    {
        proven("exclusive rename", destination, |a, b| {
            Ok(exclusive_rename_probe(
                &a.path,
                &b.path,
                temporary_sibling(destination),
            ))
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = destination;
        Ok(false)
    }
}

/// Why [`place_new`] did not place a file, and what it left.
#[derive(Debug)]
pub(super) struct Unplaced {
    pub error: ProjectIoError,
    /// The copy it was making is under the name, incomplete: the save's
    /// record takes it back ([`Transaction::place`]).
    pub partial: bool,
    /// That incomplete file, as this call created it.
    pub created: Option<FileStat>,
}
impl From<ProjectIoError> for Box<Unplaced> {
    fn from(error: ProjectIoError) -> Self {
        Box::new(Unplaced {
            error,
            partial: false,
            created: None,
        })
    }
}

/// [`excludes`]' probe on the two staging files `a` (holding "a") and `b`
/// (holding "b") and the name `free`, which should hold nothing.
#[cfg(target_os = "macos")]
fn exclusive_rename_probe(a: &Path, b: &Path, free: PathBuf) -> bool {
    let refused = sys::renamex(a, b, sys::RENAME_EXCL)
        .is_err_and(|e| e.kind() == io::ErrorKind::AlreadyExists)
        && holds(a, b"a")
        && holds(b, b"b");
    if !refused || sys::renamex(a, &free, sys::RENAME_EXCL).is_err() {
        return false;
    }
    // Only now does the name hold the probe's own file: it is the probe's
    // to remove from here on, and never a file that was already there.
    let free = TempFile {
        path: free,
        keep: false,
    };
    holds(&free.path, b"a")
}

#[cfg(all(test, target_os = "macos"))]
mod probe_tests {
    use super::*;

    /// The probe moves a staging file to a name that should be free. A
    /// file that is under that name all the same is not the probe's: the
    /// rename refuses it, and the probe must not remove it either.
    #[test]
    fn the_probe_never_removes_a_file_it_found_under_its_free_name() {
        let dir = std::env::temp_dir().join(format!("pm-probe-{}", random_uuid()));
        fs::create_dir_all(&dir).unwrap();
        let (a, b, taken) = (dir.join("a"), dir.join("b"), dir.join("taken"));
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        fs::write(&taken, b"someone's file").unwrap();
        assert!(!exclusive_rename_probe(&a, &b, taken.clone()));
        assert_eq!(fs::read(&taken).unwrap(), b"someone's file");
        // A name that is free: the probe's file lands there and goes again.
        let free = dir.join("free");
        assert!(exclusive_rename_probe(&a, &b, free.clone()));
        assert!(!free.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}

/// How a file came to be under a name nothing held ([`place_new`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Placed {
    /// The file itself is under the new name and its old name is gone.
    Moved,
    /// The new name holds a verified copy of the file, which is still under
    /// its old name too.
    Copied,
}

/// Puts the file `from` under the name `to`, which nothing may hold. No
/// route looks at the name and then renames onto it: a file that lands under
/// `to` at any instant is never replaced, on any volume, and the answer is
/// then [`ProjectIoError::DestinationExists`]. In order:
///
/// 1. the exclusive rename, where the volume has one ([`excludes`]);
/// 2. a hard link, which fails on a taken name, and then the old name is
///    removed (the file keeps the new one);
/// 3. the name is created exclusively and the bytes copied into it, synced
///    and read back ([`create_exclusive`]): a volume with neither of the
///    above (exFAT, a network share) has no other call that refuses a taken
///    name.
///
/// `hash` is what `from` holds, when the caller knows.
pub(super) fn place_new(
    from: &Path,
    to: &Path,
    hash: Option<&str>,
    hooks: &PublishHooks,
) -> Result<Placed, Box<Unplaced>> {
    let taken =
        || -> Box<Unplaced> { ProjectIoError::DestinationExists(to.display().to_string()).into() };
    let about_to = || {
        if let Some(hook) = &hooks.before_install {
            hook(to);
        }
    };
    #[cfg(target_os = "macos")]
    if excludes(to, hooks)? {
        about_to();
        match sys::renamex(from, to, sys::RENAME_EXCL) {
            Ok(()) => return Ok(Placed::Moved),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(taken()),
            // Proven a moment ago and refused now: not relied on again.
            Err(e) if matches!(e.raw_os_error(), Some(45) | Some(22)) => forget_probes(),
            Err(e) => return Err(io_err(from, e).into()),
        }
    }
    about_to();
    let linked = if hooks.no_exclusive {
        Err(io::Error::from_raw_os_error(45))
    } else {
        fs::hard_link(from, to)
    };
    match linked {
        // Both names lead to the one file: removing the old name removes no
        // bytes. If it cannot be removed the file simply has both.
        Ok(()) => {
            return Ok(match fs::remove_file(from) {
                Ok(()) => Placed::Moved,
                Err(_) => Placed::Copied,
            });
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(taken()),
        // No links (or none allowed): exclusive creation replaces nothing
        // whatever the reason was, and reports its own error if the folder
        // cannot be written at all.
        Err(_) => {}
    }
    about_to();
    create_exclusive(from, to, hash, hooks)?;
    Ok(Placed::Copied)
}

/// Renames `from` to `to`, a private name of this save's own making: it
/// carries a UUID drawn for this save, so no other writer can have chosen
/// it and nothing lands under it. (The exclusive rename is used all the
/// same where the volume has one.) Taking a file away from a public name
/// this way destroys nothing: whatever is there at that instant is what
/// gets the private name.
pub(super) fn rename_private(from: &Path, to: &Path, hooks: &PublishHooks) -> io::Result<()> {
    debug_assert!(
        to.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    );
    #[cfg(target_os = "macos")]
    if excludes(to, hooks).unwrap_or(false) {
        match sys::renamex(from, to, sys::RENAME_EXCL) {
            Err(e) if matches!(e.raw_os_error(), Some(45) | Some(22)) => {}
            other => return other,
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = hooks;
    fs::rename(from, to)
}

/// [`place_new`]'s last route: `destination` is created exclusively (never
/// over a file that is there), `source`'s bytes are copied into it and
/// synced, and what the name then holds is read back and must be those
/// bytes (and `hash`, when the caller knows what the source holds). A copy
/// that fails is left under the name, incomplete, and said to be
/// ([`Unplaced::partial`]): the save's record takes it back. A crash
/// mid-copy leaves an incomplete file under a name that held nothing
/// before.
///
/// The file's first bytes ([`HELD_BACK`] of them) are written last, after
/// everything else is synced; until then they are zeros. So whatever the
/// name holds while the copy runs is no project file to any reader, on
/// this machine or on one a sync client replicates it to: a save that only
/// appended to the previous file would otherwise pass through a moment
/// where the name holds exactly that previous file.
fn create_exclusive(
    source: &Path,
    destination: &Path,
    hash: Option<&str>,
    hooks: &PublishHooks,
) -> Result<(), Box<Unplaced>> {
    let from = File::open(source).map_err(|e| io_err(source, e))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mode = from
            .metadata()
            .map_err(|e| io_err(source, e))?
            .permissions()
            .mode();
        options.mode(mode & 0o777);
    }
    let mut out = match options.open(destination) {
        Ok(out) => out,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(
                ProjectIoError::DestinationExists(destination.display().to_string()).into(),
            );
        }
        Err(e) => return Err(io_err(destination, e).into()),
    };
    // Marked as this save's copy before it holds a byte (a volume without
    // extended attributes leaves it unmarked: recovery then never takes it
    // back on its own, and offers the versions instead).
    mark_copy(&out, source);
    let copied = (|| -> Result<(), ProjectIoError> {
        use std::io::Write;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut reader = &from;
        let mut written = 0u64;
        let mut head = Vec::new();
        loop {
            let n = read_retry(|| reader.read(&mut buf)).map_err(|e| io_err(source, e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            // The first bytes wait until the rest is there.
            if written == 0 {
                let held = n.min(HELD_BACK);
                head.extend_from_slice(&buf[..held]);
                buf[..held].fill(0);
            }
            if hooks
                .copy_fails
                .as_ref()
                .is_some_and(|armed| armed.swap(false, std::sync::atomic::Ordering::SeqCst))
            {
                let _ = out.write_all(&buf[..n / 2]);
                return Err(io_err(destination, io::Error::from_raw_os_error(28)));
            }
            if let Some(hook) = &hooks.mid_copy {
                out.write_all(&buf[..n / 2])
                    .map_err(|e| io_err(destination, e))?;
                hook(destination);
                out.write_all(&buf[n / 2..n])
                    .map_err(|e| io_err(destination, e))?;
                written += n as u64;
                continue;
            }
            if let Some((at, hook)) = &hooks.copy_stops_at
                && (written..written + n as u64).contains(at)
                && *at > written
            {
                let first = (*at - written) as usize;
                out.write_all(&buf[..first])
                    .map_err(|e| io_err(destination, e))?;
                hook(destination);
                out.write_all(&buf[first..n])
                    .map_err(|e| io_err(destination, e))?;
                written += n as u64;
                continue;
            }
            out.write_all(&buf[..n])
                .map_err(|e| io_err(destination, e))?;
            written += n as u64;
        }
        sync_file(&out).map_err(|e| io_err(destination, e))?;
        // Complete but for its first bytes: they make it the file it is.
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            out.write_all_at(&head, 0)
                .map_err(|e| io_err(destination, e))?;
        }
        #[cfg(not(unix))]
        {
            use std::io::{Seek, SeekFrom};
            out.seek(SeekFrom::Start(0))
                .and_then(|_| out.write_all(&head))
                .map_err(|e| io_err(destination, e))?;
        }
        sync_file(&out).map_err(|e| io_err(destination, e))?;
        let read = hex(&hasher.finalize());
        // Its identity is read once it has bytes: on FAT-family volumes an
        // empty file has no stable inode number yet.
        let ours = stat_of(&out.metadata().map_err(|e| io_err(destination, e))?);
        let back = File::open(destination).map_err(|e| io_err(destination, e))?;
        let same = back
            .metadata()
            .is_ok_and(|now| stat_of(&now).same_identity(&ours));
        let holds = hex(&hash_reader(&back, destination)?.finalize());
        if same && holds == read && hash.is_none_or(|hash| hash == read) {
            unmark_copy(&out);
            sync_file(&out).map_err(|e| io_err(destination, e))?;
            Ok(())
        } else {
            Err(ProjectIoError::Invalid(
                "The file could not be verified after writing it.".into(),
            ))
        }
    })();
    copied.map_err(|error| {
        Box::new(Unplaced {
            error,
            partial: true,
            created: out.metadata().ok().map(|m| stat_of(&m)),
        })
    })
}

/// How many of a file's first bytes [`create_exclusive`] writes last (a
/// project file's magic number is that long).
const HELD_BACK: usize = 5;

/// Whether `part` is a copy of `whole` in the making ([`create_exclusive`]):
/// its bytes are the beginning (or all) of `whole`'s, but for its first
/// [`HELD_BACK`] bytes, which may still be zeros.
pub(super) fn begins(whole: &Path, part: &Path) -> bool {
    File::open(part).is_ok_and(|part| begins_file(whole, &part))
}

/// Whether `path`'s first bytes are still the zeros a copy in the making
/// has there ([`HELD_BACK`]).
pub(super) fn head_held_back(path: &Path) -> bool {
    File::open(path).is_ok_and(|file| file_head_held_back(&file))
}

/// [`head_held_back`] for an open file. No project file of any format
/// begins with a zero byte.
pub(super) fn file_head_held_back(file: &File) -> bool {
    use std::os::unix::fs::FileExt;
    let mut head = [0u8; HELD_BACK];
    read_retry(|| file.read_at(&mut head, 0))
        .is_ok_and(|n| n > 0 && head[..n].iter().all(|b| *b == 0))
}

/// [`begins`] for the bytes behind `part`.
pub(super) fn begins_file(whole: &Path, part: &File) -> bool {
    let Ok(whole) = File::open(whole) else {
        return false;
    };
    let (mut a, mut b) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    let (mut whole, mut part) = (io::BufReader::new(whole), io::BufReader::new(part));
    let mut first = true;
    loop {
        let Ok(n) = read_retry(|| part.read(&mut b)) else {
            return false;
        };
        if n == 0 {
            return true;
        }
        if whole.read_exact(&mut a[..n]).is_err() {
            return false;
        }
        // A buffered reader's first read is at least the held-back bytes
        // unless the file is shorter than them.
        let held = if first { n.min(HELD_BACK) } else { 0 };
        first = false;
        let head = a[..held] == b[..held] || b[..held].iter().all(|b| *b == 0);
        if !head || a[held..n] != b[held..n] {
            return false;
        }
    }
}

/// Disposes of `path`, a file that was a version of the project (a
/// replaced version, a staged file recovery found duplicated) under a
/// private name, if it is still exactly what its record says.
///
/// The file is first moved to `fresh`, a private name that holds nothing
/// and that nothing else can know (taken exclusively where the volume can):
/// that captures whatever the name holds at that instant. The captured file
/// is then read through a newly opened descriptor (`unchanged` judges what
/// it holds), the descriptor's stat must be the same before and after that
/// read and the same as the fresh name's, and then the captured file is
/// unlinked at once, in this call. A file another writer puts under the old
/// name at any moment is therefore never what is unlinked.
///
/// Returns the files that were not removed and are the caller's to keep: the
/// captured file when it is changed or unreadable (under `fresh`), the file
/// itself when it could not be captured (under `path`), and a file that
/// landed under `path` since. Empty when the version is gone.
///
/// What remains (docs/native/p0-decisions.md): a program still holding a
/// descriptor to the file, which writes into it in the instant between
/// this check and the unlink, loses that write.
pub(super) fn dispose(
    path: &Path,
    fresh: &Path,
    unchanged: &dyn Fn(&File, &FileStat) -> bool,
    hooks: &PublishHooks,
) -> Vec<PathBuf> {
    if let Some(hook) = &hooks.before_disposal {
        hook(path);
    }
    if !matches!(lstat_file(fresh), Ok(None)) || rename_private(path, fresh, hooks).is_err() {
        return match lstat_file(path) {
            Ok(None) => Vec::new(),
            _ => vec![path.to_owned()],
        };
    }
    let mut left = Vec::new();
    if !dispose_captured(fresh, path, unchanged, hooks) {
        left.push(fresh.to_owned());
    }
    if !matches!(lstat_file(path), Ok(None)) {
        left.push(path.to_owned());
    }
    left
}

/// [`dispose`] for a file already captured under the fresh private name
/// `captured` (`name` is the name it was known by): read now, and unlinked
/// in this call if `unchanged` says it holds what it should and its stat
/// did not move across the read. `false`: it is still there.
pub(super) fn dispose_captured(
    captured: &Path,
    name: &Path,
    unchanged: &dyn Fn(&File, &FileStat) -> bool,
    hooks: &PublishHooks,
) -> bool {
    let Ok(Some(named)) = lstat_file(captured) else {
        return false;
    };
    let Ok(file) = File::open(captured) else {
        return false;
    };
    let Ok(before) = file.metadata().map(|m| stat_of(&m)) else {
        return false;
    };
    if !before.same_file(&named) || !unchanged(&file, &before) {
        return false;
    }
    let after = file.metadata().map(|m| stat_of(&m)).ok();
    if let Some(hook) = &hooks.before_unlink {
        hook(name);
    }
    after.as_ref() == Some(&before) && fs::remove_file(captured).is_ok()
}

/// For [`dispose`]: the file holds exactly the bytes with `hash` (and is
/// `size` long, where the record says).
pub(super) fn holds_hash<'a>(
    hash: &'a str,
    size: Option<u64>,
) -> impl Fn(&File, &FileStat) -> bool + 'a {
    move |file, stat| {
        size.is_none_or(|size| stat.size == size)
            && hash_reader(file, Path::new("")).is_ok_and(|h| hex(&h.finalize()) == hash)
    }
}

/// A file [`dispose`] found changed, that could not be kept beside the
/// project: it goes to the person's Trash, or stays where it is where there
/// is none. Returns where it is now, for the person to be told.
pub(super) fn trash_or_leave(path: PathBuf, hooks: &PublishHooks) -> PathBuf {
    match &hooks.trash {
        Some(trash) => (trash.0)(&path).unwrap_or(path),
        None => path,
    }
}

/// How the destination may be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Publish {
    /// The destination must not exist (first Save, Save As to a new name). A
    /// file that appears meanwhile is never overwritten.
    CreateNew,
    /// Replace exactly this version of the file; anything else on disk is a
    /// conflict.
    Replace(FileVersion),
}

/// `2026-11-12 14.03.22`, local time (a file name the person reads).
pub(super) fn local_stamp() -> String {
    local_time(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
}

/// `now` (seconds since 1970) as `2026-11-12 14.03.22`, local time.
pub fn local_time(now: u64) -> String {
    #[cfg(unix)]
    {
        let seconds = now as libc::time_t;
        let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
        // SAFETY: a valid time and output storage; zeroed is a valid `tm`
        // should the call fail.
        let tm = unsafe {
            libc::localtime_r(&seconds, tm.as_mut_ptr());
            tm.assume_init()
        };
        format!(
            "{:04}-{:02}-{:02} {:02}.{:02}.{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
    #[cfg(not(unix))]
    backup_stamp(now as i64 * 1000)
}

/// Whether `entry` is the private name a save gives a file beside the file
/// named `name`: `.<name>.<uuid v4><suffix>`, exactly.
fn private_sibling(entry: &str, name: &str, suffix: &str) -> bool {
    entry
        .strip_prefix('.')
        .and_then(|e| e.strip_prefix(name))
        .and_then(|e| e.strip_prefix('.'))
        .and_then(|e| e.strip_suffix(suffix))
        .is_some_and(is_uuid_v4)
}

/// The file a save installed, held open from the moment it was confirmed
/// under the project's name: what the receipt adopts, and what the save
/// compares the name with once everything after the install is done.
#[derive(Debug)]
pub(crate) struct Installed {
    pub file: File,
    pub stat: FileStat,
}

/// The file `destination` names now, opened, if it is the file a save just
/// installed: `ours` says it is that file by its stat (where file numbers
/// are the file's own); otherwise it must hold exactly the new bytes, read
/// through the very descriptor that is then held. On a volume whose numbers
/// follow names (a network share) another file put under the name has the
/// same number, so a number alone never says it: `ours` asks for the size
/// and times too. A file that is neither is never taken for the save.
fn installed_at(
    destination: &Path,
    ours: impl Fn(&FileStat) -> bool,
    size: u64,
    new_hash: &str,
) -> Option<Installed> {
    let named = lstat_file(destination).ok().flatten()?;
    let file = File::open(destination).ok()?;
    let stat = stat_of(&file.metadata().ok()?);
    if !stat.same_identity(&named) {
        return None;
    }
    // Read through the descriptor that is then held, and only believed for
    // the state it was read in: the same stat after the read as before it.
    let ours = ours(&stat)
        || (stat.size == size
            && hash_reader(&file, destination).is_ok_and(|h| hex(&h.finalize()) == new_hash)
            && file.metadata().is_ok_and(|after| stat_of(&after) == stat));
    ours.then_some(Installed { file, stat })
}

/// After the install: the path must hold the file just installed, and the
/// folder must sync as it did when the save began. Only then is the step
/// recorded, and only after that is anything done with the version that was
/// replaced.
fn confirm_installed(
    tx: &mut Transaction,
    destination: &Path,
    incoming: &FileStat,
    new_hash: &str,
    hooks: &PublishHooks,
) -> Result<Installed, ProjectIoError> {
    // The staged file itself, renamed into place: the same number, size and
    // modification time (a rename keeps them; a copy does not, and is read).
    let moved = |now: &FileStat| {
        now.same_identity(incoming)
            && now.size == incoming.size
            && now.mtime_ns == incoming.mtime_ns
    };
    let installed = installed_at(destination, moved, incoming.size, new_hash)
        .ok_or(ProjectIoError::ChangedOnDisk)?;
    let dir = destination.parent().unwrap_or(Path::new("."));
    match folder_sync_as(dir, tx.folder(), hooks) {
        Ok(()) => tx.step("installed")?,
        // Installed but not proven durable: nothing more is done with the
        // replaced version, and the save reports it ([`Published::unfinalized`]).
        Err(e) => tx.unsynced = Some(e.to_string()),
    }
    Ok(installed)
}

/// The last look, after everything the save did once its file was installed
/// (judging, retaining and pruning earlier versions): the name must still
/// hold the installed file, as it was when it was installed. Another writer
/// may have put a version there in the meantime, or written its own into
/// that very file; neither is ever adopted with this save's hash: the name
/// must hold a file with the stat captured at the install, or one that is
/// read now and holds exactly this save's bytes. Otherwise this save's
/// bytes, read from the descriptor held since the install (which keeps them
/// where the name no longer does, and is not copied when it was itself
/// written into), are kept beside the project under a plain name, the
/// record says so and knows the version now under the name, and the save
/// fails ([`ProjectIoError::KeptBeside`], or
/// [`ProjectIoError::ChangedOnDisk`] when there is nothing of this save
/// left to keep): the session keeps its work, its journal and its unsaved
/// state.
fn still_installed(
    tx: &mut Transaction,
    destination: &Path,
    installed: Installed,
    new_hash: &str,
) -> Result<Installed, ProjectIoError> {
    // Untouched since the install: the very stat captured then. A stat
    // that moved since (a write into the file, or only its times) is
    // believed by its bytes alone.
    let then = &installed.stat;
    if let Some(again) = installed_at(destination, |at| at.same_file(then), then.size, new_hash) {
        return Ok(again);
    }
    let kept = (|| {
        // Under one of the record's own private names that is free now (the
        // staged file was installed or exchanged away, and what was moved
        // aside went to the backups or was kept), so the record names it.
        let private = [tx.staged(), tx.aside()]
            .into_iter()
            .find(|p| matches!(lstat_file(p), Ok(None)))?;
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&private)
            .ok()?;
        let temp = TempFile {
            path: private,
            keep: false,
        };
        let mut copied = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut offset = 0u64;
        loop {
            #[cfg(unix)]
            let n = {
                use std::os::unix::fs::FileExt;
                read_retry(|| installed.file.read_at(&mut buf, offset)).ok()?
            };
            #[cfg(not(unix))]
            let n = 0;
            if n == 0 {
                break;
            }
            use std::io::Write;
            out.write_all(&buf[..n]).ok()?;
            copied.update(&buf[..n]);
            offset += n as u64;
        }
        sync_file(&out).ok()?;
        (hex(&copied.finalize()) == new_hash).then_some(())?;
        tx.keep(temp.keep(), "recovered").ok()
    })();
    let _ = sync_directory(destination.parent().unwrap_or(Path::new(".")));
    // The record knows the version under the name now, so its notice is
    // this project's at the next open.
    let there = FileVersion::read(destination).ok().flatten();
    tx.settled(there.as_ref().map(|v| v.hash.as_str()));
    Err(match kept {
        Some(kept) => ProjectIoError::KeptBeside { kept },
        // Not even a copy could be written: the work is still the session's
        // (unsaved, journalled), and the name holds the other version.
        None => ProjectIoError::ChangedOnDisk,
    })
}

/// Replaces the file at `destination` with the staged `temp` on a volume
/// without an atomic exchange, in two steps, each recorded in the save's
/// record `tx` before it is taken:
///
/// 1. The file at the path is moved aside under a private name in the same
///    folder. That takes whatever is there at that instant and destroys
///    nothing.
/// 2. The staged file is installed at the now empty path, which it takes
///    only if nothing holds it ([`place_new`]). A file that landed there
///    since stays; this save's file is kept beside it and the save fails
///    naming it ([`ProjectIoError::KeptBeside`]).
///
/// The install is then confirmed ([`confirm_installed`]), and only after
/// that is the file that was moved aside read: with `expected`'s bytes it
/// is retained ([`retire`]); with others (another version landed after the
/// last check) it is kept beside the project under a plain name and
/// reported ([`Published::displaced`]). It is never removed here. A failure
/// to install puts it back; a failure to confirm leaves it where the record
/// names it. A process killed on the way leaves the record and the files it
/// names for the next open ([`super::transaction::settle`]).
fn replace_aside(
    temp: TempFile,
    destination: &Path,
    expected: &FileVersion,
    incoming: &FileStat,
    new_hash: &str,
    tx: &mut Transaction,
    host: &Host,
) -> Result<Published, ProjectIoError> {
    let hooks = host.hooks;
    let dir = destination.parent().unwrap_or(Path::new("."));
    let aside = temp.path.with_extension(transaction::ASIDE);
    tx.step("aside")?;
    if let Some(hook) = &hooks.before_exchange {
        hook(destination);
    }
    rename_private(destination, &aside, hooks).map_err(|e| match e.kind() {
        // Gone since the last check: nothing was moved.
        io::ErrorKind::NotFound => ProjectIoError::ChangedOnDisk,
        _ => io_err(destination, e),
    })?;
    if let Some(hook) = &hooks.between_renames {
        hook(destination);
    }
    let installed = match tx
        .step("install")
        .and_then(|()| tx.place(&temp.path, destination, Some(new_hash)))
    {
        Ok(placed) => Some(placed),
        // Something landed at the path since it was emptied: it stays.
        Err(ProjectIoError::DestinationExists(_)) => None,
        // Not installed and nothing arrived: what was at the path goes back
        // (never over a file that landed there since).
        Err(e) => {
            return Err(match tx.place(&aside, destination, None) {
                // Back as a copy, the same bytes in another file: the file
                // that was moved aside stays, for a later save to prune.
                Ok(Placed::Copied) => {
                    tx.retain(&aside, None);
                    ProjectIoError::PutBack(Box::new(e))
                }
                Ok(Placed::Moved) => e,
                Err(_) => {
                    tx.strand();
                    ProjectIoError::Conflict {
                        message: format!(
                            "The project file could not be replaced ({e}), and the file that \
                             was there could not be put back under its name."
                        ),
                        recovery_path: aside,
                    }
                }
            });
        }
    };
    if let Some(hook) = &hooks.after_exchange {
        hook(destination);
    }
    match installed {
        // The staged file is the project file now.
        Some(Placed::Moved) => drop(temp.keep()),
        // The project file is a verified copy of it.
        Some(Placed::Copied) => drop(temp),
        None => {
            // The version that landed stays under the name. This save's
            // file and the one it moved aside are kept, each as what it is.
            if judged(&aside, expected, false) {
                retire(aside, &expected.hash, tx, host);
            } else {
                let _ = tx.keep(aside, "other version");
            }
            let kept = tx
                .keep(temp.keep(), "recovered")
                .unwrap_or_else(|private| private);
            let _ = sync_directory(dir);
            return Err(ProjectIoError::KeptBeside { kept });
        }
    }
    // Not confirmed: the file moved aside stays where the record names it.
    let installed = confirm_installed(tx, destination, incoming, new_hash, hooks)
        .inspect_err(|_| tx.strand())?;
    // Not synced: the same, and the save says so.
    if tx.unsynced.is_some() {
        tx.strand();
        return Ok(Published {
            installed: Some(installed),
            ..Default::default()
        });
    }
    // What was moved aside is read only now: the name was empty for no
    // longer than the two renames, and the new file is confirmed in place.
    let mut published = Published {
        installed: Some(installed),
        ..Default::default()
    };
    if judged(&aside, expected, false) {
        published.backup = retire(aside, &expected.hash, tx, host);
    } else {
        published.displaced = Some(
            tx.keep(aside, "other version")
                .unwrap_or_else(|private| private),
        );
    }
    Ok(published)
}

/// Whether `displaced`, the file a save took from the project's path, is
/// the version it could replace: read now, with exactly `expected`'s bytes.
/// By its bytes, not its file number, where the file was renamed (a
/// network share numbers files by name); `same_file` asks for the same
/// file and permissions too, where an exchange handed it back untouched. A
/// file with exactly the bytes the session read holds nothing it did not
/// read.
fn judged(displaced: &Path, expected: &FileVersion, same_file: bool) -> bool {
    FileVersion::read(displaced)
        .ok()
        .flatten()
        .is_some_and(|d| {
            d.hash == expected.hash
                && (!same_file
                    || (d.stat.same_identity(&expected.stat)
                        && d.stat.same_permissions(&expected.stat)))
        })
}

/// What a publication left on disk besides the published file.
#[derive(Debug, Default)]
pub(crate) struct Published {
    /// The exchange found another version at the destination (it arrived
    /// after the last check, or cannot be read). Ours is published; that
    /// version is kept here, beside it.
    pub displaced: Option<PathBuf>,
    /// Versions an earlier save had replaced and retained, found changed
    /// since (something wrote into them): kept beside the project here.
    pub changed: Vec<PathBuf>,
    /// The replaced version itself, where it is the backup of record now.
    pub backup: Option<PathBuf>,
    /// The volume cannot prove the new file's name survives a power cut (a
    /// network share: the server decides). The save stands as far as can
    /// be told; `local_copy` is the copy of the new version kept on this
    /// machine until a later save's copy takes its place.
    pub unproven: bool,
    pub local_copy: Option<PathBuf>,
    /// The published file, held open since it was confirmed under the
    /// name and confirmed again after the save's last step.
    pub installed: Option<Installed>,
    /// The file is installed but the folder did not sync after it: why.
    /// Nothing else was done (the replaced version stays named by the
    /// record); the save is reported failed, and adopted by the next save
    /// once the folder syncs ([`ProjectIoError::Finalize`]'s receipt).
    pub unfinalized: Option<String>,
}

/// The system's way for programs to take turns at a file (on macOS,
/// `NSFileCoordinator`). A save asks for a coordinated write of the project
/// file around the steps that move files, and so does recovery.
///
/// What that covers: every program that takes part in file coordination.
/// A file provider (iCloud Drive, a sync client built on the system's
/// provider interface) and any app that presents the file wait for the
/// write to finish, and an app holding unsaved changes to the file is asked
/// to write them first, so the save's own check then finds them. What it
/// does not cover: a program that writes the file with plain system calls
/// (a script, a sync client that does not use the provider interface), and
/// anything on another machine, such as a second computer on a shared
/// volume. Those are what the rest of the save is built for: no step
/// replaces or removes a file it has not read.
pub trait WriteCoordinator: Send + Sync {
    /// Runs `work` exactly once: inside a coordinated write of `path` when
    /// one is granted within a bounded wait (`work(true)`), and without one
    /// otherwise (`work(false)`). A save is never refused for want of it.
    fn coordinate(&self, path: &Path, work: &mut dyn FnMut(bool));
}

/// A [`WriteCoordinator`] a kernel and its saves share.
#[derive(Clone)]
pub struct SharedWriteCoordinator(pub Arc<dyn WriteCoordinator>);
impl std::fmt::Debug for SharedWriteCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WriteCoordinator")
    }
}

/// What a publication needs from its host besides the files.
#[derive(Clone, Copy)]
pub(crate) struct Host<'a> {
    pub hooks: &'a PublishHooks,
    /// Asked for a coordinated write around the steps that move files.
    pub coordinator: Option<&'a SharedWriteCoordinator>,
    /// The profile folder of the instance that saves (a record names it).
    pub profile: Option<&'a Path>,
    /// Where replaced versions are kept, and the copy of the version being
    /// replaced that this save made there before it began.
    pub backups: Option<&'a BackupPolicy>,
    pub backup: Option<&'a Path>,
    /// Where on this machine a copy of the new version is kept when the
    /// volume cannot prove its name durable ([`Published::unproven`]).
    pub unconfirmed: Option<&'a Path>,
}

impl Host<'_> {
    /// Runs `work` once, inside a coordinated write of `path` where the
    /// host has a coordinator and it grants one ([`WriteCoordinator`]).
    pub fn coordinated(&self, path: &Path, work: &mut dyn FnMut(bool)) {
        match self.coordinator {
            Some(coordinator) => coordinator.0.coordinate(path, work),
            None => work(false),
        }
    }
}

/// The extended attribute a retained version carries in the backups: the
/// hash it was read with when it was replaced.
const RETAINED_HASH: &str = "com.powermove.version-sha256";

#[cfg(target_os = "macos")]
fn set_retained_hash(path: &Path, hash: &str) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let name = std::ffi::CString::new(RETAINED_HASH)?;
    // SAFETY: valid C strings and a buffer of the stated length.
    let rc = unsafe {
        libc::setxattr(
            path.as_ptr(),
            name.as_ptr(),
            hash.as_ptr().cast(),
            hash.len(),
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The hash a retained version in the backups was read with, if it is one.
#[cfg(target_os = "macos")]
fn retained_hash(path: &Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let name = std::ffi::CString::new(RETAINED_HASH).ok()?;
    let mut buf = [0u8; 64];
    // SAFETY: valid C strings and a buffer of the stated length.
    let n = unsafe {
        libc::getxattr(
            path.as_ptr(),
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    (n == 64).then(|| String::from_utf8_lossy(&buf).into_owned())
}
#[cfg(not(target_os = "macos"))]
fn retained_hash(_: &Path) -> Option<String> {
    None
}

/// The extended attribute a file a save is copying into a name carries
/// until the copy is complete and verified: the private name of the file
/// it is a copy of, which names the save's transaction. It is what makes an
/// incomplete file at that name this save's own, not equal first bytes.
const COPY_OF: &str = "com.powermove.copy-of";

#[cfg(target_os = "macos")]
fn mark_copy(file: &File, source: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let (Some(value), Ok(name)) = (
        source.file_name().and_then(OsStr::to_str),
        std::ffi::CString::new(COPY_OF),
    ) else {
        return false;
    };
    // SAFETY: a live descriptor, a C string and a buffer of the stated length.
    unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        ) == 0
    }
}
#[cfg(not(target_os = "macos"))]
fn mark_copy(_: &File, _: &Path) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn unmark_copy(file: &File) {
    use std::os::fd::AsRawFd;
    if let Ok(name) = std::ffi::CString::new(COPY_OF) {
        // SAFETY: a live descriptor and a C string.
        unsafe { libc::fremovexattr(file.as_raw_fd(), name.as_ptr(), 0) };
    }
}
#[cfg(not(target_os = "macos"))]
fn unmark_copy(_: &File) {}

/// Whether `path` is an incomplete copy of the file named `source` that a
/// save was making ([`COPY_OF`]).
#[cfg(target_os = "macos")]
pub(super) fn copy_of(path: &Path, source: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let (Ok(c_path), Ok(name), Some(expected)) = (
        std::ffi::CString::new(path.as_os_str().as_bytes()),
        std::ffi::CString::new(COPY_OF),
        source.file_name().and_then(OsStr::to_str),
    ) else {
        return false;
    };
    let mut buf = [0u8; 256];
    // SAFETY: valid C strings and a buffer of the stated length.
    let n = unsafe {
        libc::getxattr(
            c_path.as_ptr(),
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    n > 0 && &buf[..n as usize] == expected.as_bytes()
}
#[cfg(not(target_os = "macos"))]
pub(super) fn copy_of(_: &Path, _: &Path) -> bool {
    false
}

/// Moves `displaced`, the version a save replaced, into the backups in
/// place of `copy` (the copy of it that save made there before it began),
/// if both are on one volume: an exclusive rename, the file itself, marked
/// with the hash it was read with. `None` when it stays where it is.
#[cfg(target_os = "macos")]
fn into_backups(displaced: &Path, copy: &Path, hash: &str) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let folder = copy.parent()?;
    let name = copy.file_name()?.to_str()?;
    let key = backup_key(name)?;
    if fs::metadata(displaced).ok()?.dev() != fs::metadata(folder).ok()?.dev() {
        return None;
    }
    set_retained_hash(displaced, hash).ok()?;
    let stamp = name.split_once(".pmv")?.0;
    for collision in key.collision + 1..key.collision + 100 {
        let to = folder.join(format!("{stamp}.pmv-{collision}"));
        match sys::renamex(displaced, &to, sys::RENAME_EXCL) {
            Ok(()) => {
                // The copy made before the save is the app's own, and the
                // file it was a copy of is here now.
                let _ = fs::remove_file(copy);
                let _ = File::open(folder).and_then(|f| f.sync_all());
                return Some(to);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}
#[cfg(not(target_os = "macos"))]
fn into_backups(_: &Path, _: &Path, _: &str) -> Option<PathBuf> {
    None
}

/// What becomes of `displaced`, the version a save replaced, once it was
/// read and found to hold exactly the bytes the session read (`hash`). It
/// is not removed: something may still write into it (a program that had
/// it open, a sync client replacing its private name), and a hash taken a
/// moment ago says nothing about the next moment. It becomes the backup of
/// record in the backups folder where that is on the same volume
/// ([`into_backups`]; returns where), and otherwise stays beside the
/// project under the save's private name, which the record keeps naming.
/// Either way it is only ever removed by a later save's retention pass,
/// which reads it again first and keeps a changed one as "other version"
/// ([`retain`], [`transaction::prune_retained`]).
fn retire(displaced: PathBuf, hash: &str, tx: &mut Transaction, host: &Host) -> Option<PathBuf> {
    if let Some(hook) = &host.hooks.after_judged {
        hook(&displaced);
    }
    if let Some(moved) = host
        .backup
        .and_then(|copy| into_backups(&displaced, copy, hash))
    {
        return Some(moved);
    }
    let aside = tx.aside();
    let beside = if displaced != aside && rename_private(&displaced, &aside, host.hooks).is_ok() {
        aside
    } else {
        displaced
    };
    tx.retain(&beside, Some(hash));
    None
}

/// `publishFile`: atomically publishes `temp` (already flushed) at
/// `destination`, whose content hash will be `new_hash`. On `Err` the
/// destination holds what it held before, except for
/// [`ProjectIoError::Finalize`] (our file is in place but the directory sync
/// failed), [`ProjectIoError::Conflict`] (something else replaced our
/// file right after it was published; both other versions are kept) and
/// [`ProjectIoError::KeptBeside`] (another version took the path while the
/// file was being replaced; it holds that version, ours is beside it).
///
/// Before anything is moved the save writes and syncs its record
/// ([`Transaction`]); it is removed once nothing of the save is left beside
/// the project, and is what the next open goes by if the save is cut short.
///
/// An existing file is replaced by the atomic exchange, which takes
/// whatever is at the path at that instant and hands it back to be judged.
/// Where the volume has none (see [`exchanges`]) it is moved aside and the
/// new file installed at the empty path ([`replace_aside`]). Neither checks
/// the file and then renames over it, which would destroy a version that
/// arrived in between, unread.
///
/// The exchange is never undone. When the file it hands back is not the
/// version this save could replace, that file is kept beside the project
/// ([`Published::displaced`]) and ours stays published: no call puts a file
/// back only if the path still holds ours, so a put-back that lost that race
/// would displace a third version, unread, from the place it arrived at.
pub(crate) fn publish(
    temp: TempFile,
    destination: &Path,
    mode: &Publish,
    new_hash: &str,
    host: &Host,
) -> Result<Published, ProjectIoError> {
    // From the last check of the file to the last move, inside a
    // coordinated write where the system grants one.
    let mut temp = Some(temp);
    let mut published = None;
    host.coordinated(destination, &mut |_| {
        if let Some(temp) = temp.take() {
            published = Some(probing(|| {
                publish_now(temp, destination, mode, new_hash, host)
            }));
        }
    });
    published.unwrap_or_else(|| {
        Err(ProjectIoError::Invalid(
            "The save could not start: the file coordinator did not run it.".into(),
        ))
    })
}

fn publish_now(
    temp: TempFile,
    destination: &Path,
    mode: &Publish,
    new_hash: &str,
    host: &Host,
) -> Result<Published, ProjectIoError> {
    let hooks = host.hooks;
    let dir = destination.parent().unwrap_or(Path::new("."));
    let expected = match mode {
        Publish::CreateNew => None,
        Publish::Replace(v) => Some(v),
    };
    check_version(destination, expected, false)?;
    let incoming = lstat_file(&temp.path)?.ok_or(ProjectIoError::ChangedOnDisk)?;
    let route = match expected {
        None => Route::New,
        Some(_) if exchanges(destination, hooks)? => Route::Exchange,
        Some(_) => Route::Aside,
    };
    let mut tx = Transaction::begin(
        destination,
        &temp.path,
        route,
        expected.map(|v| v.hash.as_str()),
        new_hash,
        incoming.size,
        host,
    )?;
    // A volume that cannot prove a name durable (a network share) may lose
    // the new file's name after the save was reported. Nothing this Mac can
    // read proves otherwise (a read back can come from a cache), so the new
    // version is kept on this machine first, as the checkpoint of record:
    // until a later save's copy takes its place, or a save is proven
    // durable. No copy, no save: nothing is published without it.
    let unproven = tx.folder().sync != FolderSync::Full;
    let local_copy = match host.unconfirmed {
        Some(local) if unproven => match local_copy(&temp.path, local, new_hash) {
            Ok(copy) => Some(copy),
            Err(e) => {
                tx.finish();
                return Err(e);
            }
        },
        _ => None,
    };
    let mut published = Published::default();
    let exchanged = route == Route::Exchange && {
        if let Some(hook) = &hooks.before_exchange {
            hook(destination);
        }
        match swap_files(&temp.path, destination, hooks) {
            Ok(true) => true,
            // Proven a moment ago and refused now: the other route, and
            // nothing proven is relied on again.
            Ok(false) => {
                forget_probes();
                false
            }
            Err(e) => {
                tx.finish();
                return Err(e);
            }
        }
    };
    match expected {
        Some(expected) if exchanged => {
            // The one step recorded after it was taken: an exchange cannot
            // be half done, and what the two names hold says which it was.
            let _ = tx.step("exchanged");
            if let Some(hook) = &hooks.after_exchange {
                hook(destination);
            }
            let confirmed = confirm_installed(&mut tx, destination, &incoming, new_hash, hooks);
            // The folder did not sync: the displaced file stays where the
            // record names it, for the next open or save to settle.
            if let (Ok(_), Some(message)) = (&confirmed, tx.unsynced.clone()) {
                let _ = temp.keep();
                tx.strand();
                tx.finish();
                published.installed = confirmed.ok();
                published.unfinalized = Some(message);
                published.unproven = unproven;
                published.local_copy = local_copy;
                return Ok(published);
            }
            // `temp.path` names the displaced file: the version we were
            // allowed to replace (retained), or one to keep beside.
            if judged(&temp.path, expected, true) {
                published.backup = retire(temp.keep(), &expected.hash, &mut tx, host);
            } else {
                let kept = tx.keep(temp.keep(), "other version");
                published.displaced = Some(kept.unwrap_or_else(|private| private));
            }
            // The path no longer holds the file just published.
            match confirmed {
                Ok(installed) => published.installed = Some(installed),
                Err(_) => {
                    let _ = sync_directory(dir);
                    tx.settled(None);
                    tx.finish();
                    return Err(match published.displaced {
                        Some(recovery_path) => ProjectIoError::Conflict {
                            message: "The project file was replaced by something else while it \
                                      was being saved."
                                .into(),
                            recovery_path,
                        },
                        None => ProjectIoError::ChangedOnDisk,
                    });
                }
            }
        }
        Some(expected) => {
            match replace_aside(
                temp,
                destination,
                expected,
                &incoming,
                new_hash,
                &mut tx,
                host,
            ) {
                Ok(aside) => published = aside,
                Err(e) => {
                    tx.settled(None);
                    tx.finish();
                    return Err(e);
                }
            }
        }
        None => {
            let placed = tx
                .step("install")
                .and_then(|()| tx.place(&temp.path, destination, Some(new_hash)));
            match placed {
                Ok(Placed::Moved) => drop(temp.keep()),
                // The copy keeps the data; `temp` unlinks its name on drop.
                Ok(Placed::Copied) => drop(temp),
                Err(e) => {
                    // Nothing of this save is under the name: it was taken
                    // by something else, or a failed copy was taken back.
                    if matches!(e, ProjectIoError::DestinationExists(_))
                        || matches!(lstat_file(destination), Ok(None))
                    {
                        tx.finish();
                    }
                    return Err(e);
                }
            }
            match confirm_installed(&mut tx, destination, &incoming, new_hash, hooks) {
                Ok(installed) => published.installed = Some(installed),
                Err(e) => {
                    tx.finish();
                    return Err(e);
                }
            }
        }
    }
    // Installed, but the folder did not sync: the save is not adopted yet
    // (the session keeps it as a ticket and the next save syncs again).
    if let Some(message) = tx.unsynced.clone() {
        tx.finish();
        published.unfinalized = Some(message);
        published.unproven = unproven;
        published.local_copy = local_copy;
        return Ok(published);
    }
    // The save stands. Versions earlier saves retained are read again and
    // go; one that changed since is kept beside the project instead.
    published.changed = transaction::prune_retained(destination, &tx, hooks);
    if let Some(policy) = host.backups {
        let just_written = published.backup.as_deref().or(host.backup);
        for changed in retain(policy, just_written, hooks).unwrap_or_default() {
            match tx.keep_file(changed.clone(), "other version") {
                Ok((kept, Placed::Moved)) => published.changed.push(kept),
                // Its bytes are beside the project as a copy, or could not
                // be brought there: the file keeps them in the backups,
                // under a name the retention pass never prunes.
                other => {
                    published.changed.extend(other.ok().map(|(kept, _)| kept));
                    let _ = fs::rename(&changed, changed.with_extension("changed.pmv"));
                }
            }
        }
    }
    // Only now, after everything done since the install, is the name looked
    // at for the last time: what the receipt adopts is the file found here.
    let installed = published
        .installed
        .take()
        .ok_or(ProjectIoError::ChangedOnDisk)
        .and_then(|installed| still_installed(&mut tx, destination, installed, new_hash));
    let installed = match installed {
        Ok(installed) => installed,
        Err(e) => {
            tx.finish();
            return Err(e);
        }
    };
    published.installed = Some(installed);
    if let Some(expected) = expected {
        transaction::carry_notices(destination, &tx, &expected.hash, new_hash);
    }
    tx.settled(Some(new_hash));
    tx.finish();
    published.unproven = unproven;
    published.local_copy = local_copy;
    // A save whose name is proven durable needs no earlier local copy.
    if let (false, Some(local)) = (unproven, host.unconfirmed) {
        forget_local_copies(local, None);
    }
    Ok(published)
}

/// Keeps `staged`, the finished new version (hash `hash`), in the folder
/// `local` on this machine as `<hash>.pmv`, synced, and then drops the
/// local copies of earlier saves of the project. Returns where it is.
fn local_copy(staged: &Path, local: &Path, hash: &str) -> Result<PathBuf, ProjectIoError> {
    fs::create_dir_all(local).map_err(|e| io_err(local, e))?;
    let kept = local.join(format!("{hash}.pmv"));
    if !kept.is_file() {
        let writing = local.join(format!("{hash}.writing"));
        fs::copy(staged, &writing)
            .and_then(|_| File::open(&writing)?.sync_all())
            .and_then(|()| fs::rename(&writing, &kept))
            .map_err(|e| {
                let _ = fs::remove_file(&writing);
                io_err(&kept, e)
            })?;
        sync_directory(local)?;
    }
    forget_local_copies(local, Some(&kept));
    Ok(kept)
}

/// Removes the app's own local copies of unconfirmed saves in `local`,
/// other than `keep`.
pub(crate) fn forget_local_copies(local: &Path, keep: Option<&Path>) {
    for entry in fs::read_dir(local).into_iter().flatten().flatten() {
        let path = entry.path();
        if Some(path.as_path()) != keep && path.extension().is_some_and(|e| e == "pmv") {
            let _ = fs::remove_file(path);
        }
    }
}

// ── version backups (`ProjectFiles.backup` / `retain`) ─────────────────────

/// Where and how many previous versions to keep. The old app kept them in
/// `<userData>/Backups/<projectId>/`; the caller chooses the directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupPolicy {
    pub directory: PathBuf,
    /// Most versions kept (12 in the old app); older versions are deleted.
    pub max_versions: usize,
    /// Byte budget across kept versions (1 GiB in the old app).
    pub max_bytes: u64,
    /// Versions always kept even over budget (2 in the old app).
    pub min_versions: usize,
}

impl BackupPolicy {
    pub fn new(directory: PathBuf) -> BackupPolicy {
        BackupPolicy {
            directory,
            max_versions: 12,
            max_bytes: 1 << 30,
            min_versions: 2,
        }
    }
}

/// Where a backup sorts in save order: its stamp in milliseconds, then its
/// collision number. Stamps are assigned in save order (see [`backup`]), so
/// this order is the order the versions were saved, whatever the clock did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BackupKey {
    millis: i64,
    collision: u64,
}

/// `2026-10-31T21-38-05-123Z` (an ISO-8601 instant with `:` and `.` replaced,
/// as the old app named its backups).
fn backup_stamp(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}-{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        millis.rem_euclid(1000)
    )
}

/// The inverse of [`backup_stamp`] for the 24 stamp characters.
fn stamp_millis(stamp: &[u8]) -> i64 {
    let n = |range: std::ops::Range<usize>| {
        stamp[range]
            .iter()
            .fold(0i64, |acc, d| acc * 10 + i64::from(d - b'0'))
    };
    let (year, month, day) = (n(0..4), n(5..7), n(8..10));
    // Howard Hinnant's days_from_civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days * 24 + n(11..13)) * 60 + n(14..16)) * 60_000 + n(17..19) * 1000 + n(20..23)
}

/// A name this app gives a backup, the whole name and nothing else:
/// `^\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z\.pmv(?:-\d+)?$`.
/// Whether `name` is the file name of a version in a project's backups.
pub(super) fn is_backup_name(name: &str) -> bool {
    !name.contains('/') && backup_key(name).is_some()
}

fn backup_key(name: &str) -> Option<BackupKey> {
    let b = name.as_bytes();
    let shape = b"dddd-dd-ddTdd-dd-dd-dddZ.pmv";
    if b.len() < shape.len() {
        return None;
    }
    for (i, s) in shape.iter().enumerate() {
        let ok = match s {
            b'd' => b[i].is_ascii_digit(),
            c => b[i] == *c,
        };
        if !ok {
            return None;
        }
    }
    let collision = match &b[shape.len()..] {
        [] => 0,
        [b'-', digits @ ..] if !digits.is_empty() && digits.iter().all(u8::is_ascii_digit) => {
            std::str::from_utf8(digits).ok()?.parse().ok()?
        }
        _ => return None,
    };
    Some(BackupKey {
        millis: stamp_millis(b),
        collision,
    })
}

/// A name earlier builds gave a generation they moved to `archived/`: a
/// backup name, or a backup name plus `.<uuid v4>` when the archive already
/// held that name. Anything else in `archived/` is not ours.
fn archived_key(name: &str) -> Option<BackupKey> {
    if let Some(key) = backup_key(name) {
        return Some(key);
    }
    let split = name.len().checked_sub(37)?;
    let (base, suffix) = (name.get(..split)?, name.get(split..)?);
    if is_uuid_v4(suffix.strip_prefix('.')?) {
        backup_key(base)
    } else {
        None
    }
}

/// [`random_uuid`]'s form: lowercase hex, hyphenated, version 4.
pub(super) fn is_uuid_v4(uuid: &str) -> bool {
    let uuid = uuid.as_bytes();
    let hyphens = [8, 13, 18, 23];
    uuid.len() == 36
        && uuid.iter().enumerate().all(|(i, c)| {
            if hyphens.contains(&i) {
                *c == b'-'
            } else {
                matches!(c, b'0'..=b'9' | b'a'..=b'f')
            }
        })
        && uuid[14] == b'4'
        && matches!(uuid[19], b'8' | b'9' | b'a' | b'b')
}

/// Removes the empty staging files ([`temporary_sibling`]) a save cut short
/// left next to `destination` (Quit Now exits before a staged file is
/// written or dropped). Only `.<file name>.<uuid v4>.tmp` exactly, only an
/// empty regular file, listed and removed through the folder held open
/// without following links. Call it while holding the project's lock, so
/// no save of this project is staging one. Best effort: returns how many
/// went.
pub(crate) fn remove_abandoned_temps(destination: &Path) -> usize {
    let (Some(parent), Some(name)) = (destination.parent(), destination.file_name()) else {
        return 0;
    };
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let Some(name) = name.to_str() else {
        return 0;
    };
    let ours = |entry: &str| private_sibling(entry, name, ".tmp");
    let Ok(Some(dir)) = Dir::open(parent) else {
        return 0;
    };
    let Ok(entries) = dir.entries() else {
        return 0;
    };
    let mut removed = 0;
    for (entry, kind) in entries {
        if matches!(kind, Kind::File { size: 0 })
            && entry.to_str().is_some_and(ours)
            && dir.unlink(&entry).is_ok()
        {
            removed += 1;
        }
    }
    if removed > 0 {
        let _ = dir.sync();
    }
    removed
}

/// Directories held open by descriptor. Backups are listed, created and
/// deleted relative to a descriptor opened without following links, so a
/// link planted anywhere inside the backup tree (a project folder,
/// `archived/`) is refused, not followed, and a swap after the check cannot
/// redirect a delete outside the tree.
#[cfg(unix)]
mod held {
    use std::ffi::{CStr, CString, OsStr, OsString};
    use std::fs::File;
    use std::io::{self, Seek};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::raw::c_int;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;

    pub(super) enum Kind {
        File { size: u64 },
        Dir,
        Other,
    }

    pub(super) struct Dir(OwnedFd);

    const DIR_FLAGS: c_int =
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    fn component(name: &OsStr) -> io::Result<CString> {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a single file name",
            ));
        }
        CString::new(bytes).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in name"))
    }

    fn checked(rc: c_int) -> io::Result<c_int> {
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(rc)
        }
    }

    /// `None` when nothing is there; `NotADirectory` for a link or a file.
    fn opened(fd: c_int) -> io::Result<Option<Dir>> {
        if fd >= 0 {
            // SAFETY: `fd` was just returned by open/openat and is owned here.
            return Ok(Some(Dir(unsafe { OwnedFd::from_raw_fd(fd) })));
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::ENOENT) => Ok(None),
            Some(libc::ELOOP) | Some(libc::ENOTDIR) => Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "is a link or not a folder",
            )),
            _ => Err(e),
        }
    }

    impl Dir {
        /// Opens `path`'s last component without following a link.
        pub fn open(path: &Path) -> io::Result<Option<Dir>> {
            let c = CString::new(path.as_os_str().as_bytes())?;
            // SAFETY: `c` is a valid NUL-terminated path for the call.
            opened(unsafe { libc::open(c.as_ptr(), DIR_FLAGS) })
        }

        /// Opens (optionally creating) the folder `name` in this one.
        pub fn sub(&self, name: &OsStr, create: bool) -> io::Result<Option<Dir>> {
            let c = component(name)?;
            if create {
                // SAFETY: valid descriptor and NUL-terminated name.
                let rc = unsafe { libc::mkdirat(self.fd(), c.as_ptr(), 0o755) };
                if let Err(e) = checked(rc)
                    && e.kind() != io::ErrorKind::AlreadyExists
                {
                    return Err(e);
                }
            }
            // SAFETY: valid descriptor and NUL-terminated name.
            opened(unsafe { libc::openat(self.fd(), c.as_ptr(), DIR_FLAGS) })
        }

        fn fd(&self) -> c_int {
            self.0.as_raw_fd()
        }

        /// Every entry, classified without following links.
        pub fn entries(&self) -> io::Result<Vec<(OsString, Kind)>> {
            // SAFETY: duplicates a valid descriptor; the copy is handed to
            // fdopendir, which owns it until closedir.
            let dup = checked(unsafe { libc::fcntl(self.fd(), libc::F_DUPFD_CLOEXEC, 0) })?;
            // SAFETY: `dup` is an open directory descriptor.
            let stream = unsafe { libc::fdopendir(dup) };
            if stream.is_null() {
                let e = io::Error::last_os_error();
                // SAFETY: fdopendir failed, so `dup` is still ours.
                unsafe { libc::close(dup) };
                return Err(e);
            }
            let mut names = Vec::new();
            // SAFETY: `stream` is a valid DIR*; the duplicate shares the
            // offset with our descriptor, so start from the beginning.
            unsafe { libc::rewinddir(stream) };
            loop {
                // SAFETY: valid stream; the entry lives until the next call.
                let entry = unsafe { libc::readdir(stream) };
                if entry.is_null() {
                    break;
                }
                // SAFETY: `d_name` is NUL-terminated.
                let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if name != b"." && name != b".." {
                    names.push(OsString::from_vec(name.to_vec()));
                }
            }
            // SAFETY: closes the stream and `dup` once.
            unsafe { libc::closedir(stream) };
            Ok(names
                .into_iter()
                .map(|n| {
                    let kind = self.kind(&n);
                    (n, kind)
                })
                .collect())
        }

        pub fn kind(&self, name: &OsStr) -> Kind {
            let Ok(c) = component(name) else {
                return Kind::Other;
            };
            // SAFETY: an all-zero `stat` is a valid out-parameter.
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: valid descriptor, name and out-pointer.
            let rc =
                unsafe { libc::fstatat(self.fd(), c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
            if rc != 0 {
                return Kind::Other;
            }
            match st.st_mode & libc::S_IFMT {
                libc::S_IFREG => Kind::File {
                    size: st.st_size as u64,
                },
                libc::S_IFDIR => Kind::Dir,
                _ => Kind::Other,
            }
        }

        /// Removes the entry `name` from this folder (a link is removed,
        /// never followed).
        pub fn unlink(&self, name: &OsStr) -> io::Result<()> {
            let c = component(name)?;
            // SAFETY: valid descriptor and name.
            checked(unsafe { libc::unlinkat(self.fd(), c.as_ptr(), 0) }).map(drop)
        }

        /// Removes the empty folder `name` (fails for anything else).
        pub fn remove_dir(&self, name: &OsStr) -> io::Result<()> {
            let c = component(name)?;
            // SAFETY: valid descriptor and name.
            checked(unsafe { libc::unlinkat(self.fd(), c.as_ptr(), libc::AT_REMOVEDIR) }).map(drop)
        }

        /// `syncDirectory` on the held folder.
        pub fn sync(&self) -> io::Result<()> {
            File::from(self.0.try_clone()?).sync_all()
        }

        fn open_file(&self, name: &OsStr, flags: c_int, mode: libc::c_uint) -> io::Result<File> {
            let c = component(name)?;
            // SAFETY: valid descriptor and name; the result is owned below.
            let fd = checked(unsafe {
                libc::openat(
                    self.fd(),
                    c.as_ptr(),
                    flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    mode,
                )
            })?;
            // SAFETY: `fd` was just opened and is owned by the File.
            Ok(unsafe { File::from_raw_fd(fd) })
        }

        /// Copies `source` to the new entry `name` (an APFS clone where the
        /// volume supports it), failing with `AlreadyExists` if anything is
        /// there, then makes it durable.
        pub fn copy_new(&self, source: &File, name: &OsStr) -> io::Result<()> {
            #[cfg(target_os = "macos")]
            {
                let c = component(name)?;
                // SAFETY: valid descriptors and name.
                let rc =
                    unsafe { libc::fclonefileat(source.as_raw_fd(), self.fd(), c.as_ptr(), 0) };
                match checked(rc) {
                    Ok(_) => return self.sync_new(name),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(e),
                    // Not cloneable here (another volume type): copy bytes.
                    Err(_) => {}
                }
            }
            let mut out =
                self.open_file(name, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL, 0o600)?;
            let copied = (|| {
                let mut src = source;
                src.rewind()?;
                io::copy(&mut src, &mut out)?;
                out.set_permissions(source.metadata()?.permissions())?;
                out.sync_all()
            })();
            if copied.is_err() {
                let _ = self.unlink(name);
            }
            copied
        }

        fn sync_new(&self, name: &OsStr) -> io::Result<()> {
            let synced = self
                .open_file(name, libc::O_RDONLY, 0)
                .and_then(|f| f.sync_all());
            if synced.is_err() {
                let _ = self.unlink(name);
            }
            synced
        }
    }
}

/// The same interface by path where descriptors are not available.
#[cfg(not(unix))]
mod held {
    use std::ffi::{OsStr, OsString};
    use std::fs::{self, File};
    use std::io;
    use std::path::{Path, PathBuf};

    pub(super) enum Kind {
        File { size: u64 },
        Dir,
        Other,
    }

    pub(super) struct Dir(PathBuf);

    fn kind_of(meta: io::Result<fs::Metadata>) -> Kind {
        match meta {
            Ok(m) if m.is_file() => Kind::File { size: m.len() },
            Ok(m) if m.is_dir() => Kind::Dir,
            _ => Kind::Other,
        }
    }

    impl Dir {
        pub fn open(path: &Path) -> io::Result<Option<Dir>> {
            match kind_of(fs::symlink_metadata(path)) {
                Kind::Dir => Ok(Some(Dir(path.to_owned()))),
                _ if !path.exists() => Ok(None),
                _ => Err(io::Error::new(io::ErrorKind::NotADirectory, "not a folder")),
            }
        }
        pub fn sub(&self, name: &OsStr, create: bool) -> io::Result<Option<Dir>> {
            let path = self.0.join(name);
            if create
                && let Err(e) = fs::create_dir(&path)
                && e.kind() != io::ErrorKind::AlreadyExists
            {
                return Err(e);
            }
            Dir::open(&path)
        }
        pub fn entries(&self) -> io::Result<Vec<(OsString, Kind)>> {
            Ok(fs::read_dir(&self.0)?
                .flatten()
                .map(|e| (e.file_name(), kind_of(fs::symlink_metadata(e.path()))))
                .collect())
        }
        pub fn kind(&self, name: &OsStr) -> Kind {
            kind_of(fs::symlink_metadata(self.0.join(name)))
        }
        pub fn unlink(&self, name: &OsStr) -> io::Result<()> {
            fs::remove_file(self.0.join(name))
        }
        pub fn remove_dir(&self, name: &OsStr) -> io::Result<()> {
            fs::remove_dir(self.0.join(name))
        }
        pub fn sync(&self) -> io::Result<()> {
            Ok(())
        }
        pub fn copy_new(&self, source: &File, name: &OsStr) -> io::Result<()> {
            let mut out = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.0.join(name))?;
            let mut src = source;
            io::copy(&mut src, &mut out)?;
            out.sync_all()
        }
    }
}

use held::{Dir, Kind};

/// `NotADirectory` from [`held`] means a link (or a file) where a backup
/// folder belongs: refused, never followed.
fn backup_dir_err(path: &Path, e: std::io::Error) -> ProjectIoError {
    if e.kind() == std::io::ErrorKind::NotADirectory {
        ProjectIoError::Invalid(format!(
            "{} is a link or a file, not a backup folder. Powermove keeps backups only in a real folder.",
            path.display()
        ))
    } else {
        io_err(path, e)
    }
}

/// The backup root and this project's folder in it, both opened without
/// following links. `Ok(None)` when either does not exist (and `create` is
/// false).
fn open_backups(policy: &BackupPolicy, create: bool) -> Result<Option<Dir>, ProjectIoError> {
    let folder = &policy.directory;
    let name = folder
        .file_name()
        .ok_or_else(|| ProjectIoError::Invalid("The backup folder has no name.".into()))?;
    let root = match folder.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if create {
        fs::create_dir_all(root).map_err(|e| io_err(root, e))?;
    }
    let Some(root_dir) = Dir::open(root).map_err(|e| backup_dir_err(root, e))? else {
        return Ok(None);
    };
    root_dir
        .sub(name, create)
        .map_err(|e| backup_dir_err(folder, e))
}

/// A folder's backups (regular files with the app's exact names), newest
/// first in save order, with their sizes.
fn generations(dir: &Dir) -> Result<Vec<(BackupKey, OsString, u64)>, std::io::Error> {
    let mut found: Vec<_> = dir
        .entries()?
        .into_iter()
        .filter_map(|(name, kind)| {
            let Kind::File { size } = kind else {
                return None;
            };
            let key = backup_key(name.to_str()?)?;
            Some((key, name, size))
        })
        .collect();
    found.sort_by(|a, b| b.cmp(a));
    Ok(found)
}

/// Backups of the policy's directory, newest first in save order.
pub fn list_backups(policy: &BackupPolicy) -> Result<Vec<PathBuf>, ProjectIoError> {
    let Some(dir) = open_backups(policy, false)? else {
        return Ok(Vec::new());
    };
    let found = generations(&dir).map_err(|e| io_err(&policy.directory, e))?;
    Ok(found
        .into_iter()
        .map(|(_, name, _)| policy.directory.join(name))
        .collect())
}

/// Copies (clones) the current destination into the backup directory and
/// syncs it. Returns the backup's path.
///
/// The stamp is the wall clock unless that would sort at or before the
/// newest backup already kept (the clock moved backward, or an earlier build
/// stamped with a clock that ran ahead): then it is 1 ms after that one. A
/// backup's name order is therefore its save order, which is what retention
/// prunes by.
pub(crate) fn backup(policy: &BackupPolicy, destination: &Path) -> Result<PathBuf, ProjectIoError> {
    let dir = open_backups(policy, true)?
        .ok_or_else(|| io_err(&policy.directory, std::io::ErrorKind::NotFound.into()))?;
    let source = File::open(destination).map_err(|e| io_err(destination, e))?;
    let newest = generations(&dir)
        .map_err(|e| io_err(&policy.directory, e))?
        .first()
        .map(|(key, _, _)| key.millis);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let stamp = backup_stamp(newest.map_or(now, |newest| now.max(newest + 1)));
    let mut collision = 0u32;
    loop {
        let name = if collision == 0 {
            format!("{stamp}.pmv")
        } else {
            format!("{stamp}.pmv-{collision}")
        };
        let file = policy.directory.join(&name);
        match dir.copy_new(&source, name.as_ref()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                collision += 1;
                continue;
            }
            Err(e) => return Err(io_err(&file, e)),
        }
        if let Err(e) = dir.sync() {
            let _ = dir.unlink(name.as_ref());
            return Err(io_err(&policy.directory, e));
        }
        return Ok(file);
    }
}

/// The fresh private name a version in the backups is captured under while
/// it is disposed of: `.<its name>.<uuid>.gone`, in the same folder.
fn captured_backup(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{}.gone", random_uuid()))
}

/// Moves a captured version back under its own name in the backups, never
/// over a file that is there.
fn rename_back(captured: &Path, own: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        sys::renamex(captured, own, sys::RENAME_EXCL).is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        fs::hard_link(captured, own).is_ok() && fs::remove_file(captured).is_ok()
    }
}

/// The backup a name made by [`captured_backup`] belongs to.
fn backup_of_captured(captured: &Path) -> Option<PathBuf> {
    let name = captured.file_name()?.to_str()?;
    let rest = name.strip_prefix('.')?.strip_suffix(".gone")?;
    let (own, uuid) = rest.split_at(rest.len().checked_sub(37)?);
    (is_backup_name(own) && is_uuid_v4(uuid.strip_prefix('.')?))
        .then(|| captured.with_file_name(own))
}

/// Keep the newest versions within the budgets (always the newest
/// `min_versions` in save order, and always `just_written`) and delete older
/// ones, as the old app pruned its backups after each save (v1.2.0 kept 5).
/// A version that is the replaced file itself ([`into_backups`]) and no
/// longer holds the bytes it was replaced with is not deleted: it is
/// returned, for the caller to keep beside the project.
/// Generations earlier builds moved to `archived/` are older than every kept
/// one and go too, so a project's backups stay bounded. Only regular files
/// with the exact names the app writes are deleted; recovery copies
/// (`*.recovery.pmv`) and anything else stay. Linked folders are refused or
/// skipped, never followed.
pub(crate) fn retain(
    policy: &BackupPolicy,
    just_written: Option<&Path>,
    hooks: &PublishHooks,
) -> Result<Vec<PathBuf>, ProjectIoError> {
    let mut changed = Vec::new();
    let Some(dir) = open_backups(policy, false)? else {
        return Ok(changed);
    };
    let folder_err = |e| io_err(&policy.directory, e);
    let keep = just_written.and_then(Path::file_name);
    let mut bytes = 0u64;
    let mut removed = false;
    // A version captured to be disposed of when the app was cut short goes
    // back under its own name, and is read again when its turn comes.
    for entry in fs::read_dir(&policy.directory)
        .into_iter()
        .flatten()
        .flatten()
    {
        let captured = entry.path();
        if let Some(own) = backup_of_captured(&captured)
            && matches!(lstat_file(&captured), Ok(Some(_)))
        {
            rename_back(&captured, &own);
        }
    }
    for (index, (_, name, size)) in generations(&dir)
        .map_err(folder_err)?
        .into_iter()
        .enumerate()
    {
        bytes = bytes.saturating_add(size);
        if Some(name.as_os_str()) == keep {
            continue;
        }
        if index >= policy.min_versions
            && (index >= policy.max_versions || bytes > policy.max_bytes)
        {
            // A version a save replaced and moved here is the file itself:
            // read again before it goes, and kept if something wrote into
            // it since it was replaced.
            let path = policy.directory.join(&name);
            if let Some(hash) = retained_hash(&path) {
                let fresh = captured_backup(&path);
                let left = dispose(&path, &fresh, &holds_hash(&hash, None), hooks);
                removed |= left.is_empty();
                // Changed: back under its own name, for the caller to keep.
                changed.extend(
                    left.into_iter()
                        .filter(|left| *left == path || rename_back(left, &path))
                        .map(|_| path.clone()),
                );
                continue;
            }
            // The app's own copy, made here and never under the project's
            // name.
            dir.unlink(&name).map_err(folder_err)?;
            removed = true;
        }
    }
    let archived = OsStr::new("archived");
    let archive = match dir.sub(archived, false) {
        Ok(archive) => archive,
        // A linked `archived` is not ours to clean.
        Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => None,
        Err(e) => return Err(folder_err(e)),
    };
    if let Some(archive) = archive {
        let archive_path = policy.directory.join(archived);
        for (name, kind) in archive.entries().map_err(|e| io_err(&archive_path, e))? {
            let ours = name.to_str().and_then(archived_key).is_some();
            if ours && matches!(kind, Kind::File { .. }) {
                archive
                    .unlink(&name)
                    .map_err(|e| io_err(&archive_path, e))?;
            }
        }
        // Gone once empty; anything else in it stays.
        let _ = dir.remove_dir(archived);
        removed = true;
    }
    if removed {
        dir.sync().map_err(folder_err)?;
    }
    Ok(changed)
}

/// A full disk must not cost the user their current work: frees `needed`
/// bytes from generations earlier builds archived (`archived/`), across
/// every project, oldest first. Kept versions, anything not named by the
/// app, and anything behind a link stay.
/// Returns the bytes freed; a save that failed on space retries only if any.
pub(crate) fn reclaim_archived(policy: &BackupPolicy, needed: u64) -> u64 {
    let Some(root) = policy.directory.parent() else {
        return 0;
    };
    let Ok(Some(root)) = Dir::open(root) else {
        return 0;
    };
    let archived = OsStr::new("archived");
    let mut archives = vec![];
    let mut files: Vec<(BackupKey, usize, OsString, u64)> = vec![];
    for (project, kind) in root.entries().into_iter().flatten() {
        if !matches!(kind, Kind::Dir) {
            continue;
        }
        let Ok(Some(project)) = root.sub(&project, false) else {
            continue;
        };
        let Ok(Some(archive)) = project.sub(archived, false) else {
            continue;
        };
        for (name, kind) in archive.entries().into_iter().flatten() {
            if let (Kind::File { size }, Some(key)) = (kind, name.to_str().and_then(archived_key)) {
                files.push((key, archives.len(), name, size));
            }
        }
        archives.push(archive);
    }
    files.sort();
    let mut freed = 0;
    for (_, archive, name, size) in files {
        if freed >= needed {
            break;
        }
        if archives[archive].unlink(&name).is_ok() {
            freed += size;
        }
    }
    freed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_names_match_the_old_pattern() {
        let stamp = backup_stamp(1_793_489_885_123);
        assert_eq!(stamp, "2026-10-31T23-38-05-123Z");
        assert_eq!(stamp_millis(stamp.as_bytes()), 1_793_489_885_123);
        for millis in [0, 951_782_400_000, 4_102_444_799_999] {
            assert_eq!(stamp_millis(backup_stamp(millis).as_bytes()), millis);
        }
        let key = |name: &str| backup_key(name).map(|k| (k.millis, k.collision));
        assert_eq!(key(&format!("{stamp}.pmv")), Some((1_793_489_885_123, 0)));
        assert_eq!(key(&format!("{stamp}.pmv-3")), Some((1_793_489_885_123, 3)));
        assert_eq!(key(&format!("{stamp}.pmv-")), None);
        assert_eq!(key(&format!("{stamp}.pmv.recovery.pmv")), None);
        assert_eq!(key("notes.pmv"), None);
        // Collisions sort numerically: `-10` is newer than `-9`.
        assert!(key(&format!("{stamp}.pmv-10")) > key(&format!("{stamp}.pmv-9")));
    }

    #[test]
    fn a_document_place_is_not_a_bundle_a_hidden_folder_or_a_system_folder() {
        for place in [
            "/Volumes/Work/job/cut.pmv",
            "/Users/someone/Movies/2026.11 final v1.2/cut.pmv",
            "/Users/someone/Dropbox/job/cut.pmv",
            "/private/tmp/job/cut.pmv",
        ] {
            assert!(is_document_place(Path::new(place)), "{place}");
        }
        for place in [
            "/Users/someone/.Trash/cut.pmv",
            "/Volumes/Work/.Trashes/501/cut.pmv",
            "/Users/someone/Tools/Thing.app/Contents/Resources/cut.pmv",
            "/Users/someone/Pictures/Photos Library.photoslibrary/originals/cut.pmv",
            "/Users/someone/work/.git/cut.pmv",
            "/Applications/cut.pmv",
            "/System/Library/cut.pmv",
        ] {
            assert!(!is_document_place(Path::new(place)), "{place}");
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let library = home.join("Library");
            assert!(!is_document_place(&library.join("Caches/x/cut.pmv")));
            assert!(is_document_place(
                &library.join("Mobile Documents/com~apple~CloudDocs/job/cut.pmv")
            ));
            assert!(is_document_place(
                &library.join("CloudStorage/Provider-Name/job/cut.pmv")
            ));
        }
    }

    #[test]
    fn uuids_are_v4_shaped_and_distinct() {
        let a = random_uuid();
        let b = random_uuid();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
    }
}
