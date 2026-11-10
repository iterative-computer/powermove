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
//!   `RENAME_SWAP`) and then verifies the *displaced* file is exactly the
//!   version we expected. If another writer got there first, our bytes are
//!   swapped back; if that is not possible both versions are kept and the
//!   error names the recovery file.
//! * A first save never overwrites: exclusive rename (`RENAME_EXCL`) or a
//!   hard link, both of which fail if anything appeared at the destination.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileVersion {
    pub path: PathBuf,
    pub stat: FileStat,
    /// Lowercase hex SHA-256 of the whole file.
    pub hash: String,
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

pub(crate) fn hash_reader(file: &File, path: &Path) -> Result<Sha256, ProjectIoError> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut reader = file;
    loop {
        let n = read_retry(|| reader.read(&mut buf)).map_err(|e| io_err(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
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

/// `syncDirectory`: makes a rename/link durable.
pub fn sync_directory(dir: &Path) -> Result<(), ProjectIoError> {
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let handle = File::open(dir).map_err(|e| io_err(dir, e))?;
    handle.sync_all().map_err(|e| io_err(dir, e))
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

/// `swapFiles`: `Ok(true)` when exchanged, `Ok(false)` when the volume or
/// platform does not support exchange (the caller falls back).
fn swap_files(a: &Path, b: &Path) -> Result<bool, ProjectIoError> {
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

/// `installFile`: exclusive rename where supported. `Ok(false)` to fall back.
fn install_exclusive(temp: &Path, destination: &Path) -> Result<bool, ProjectIoError> {
    #[cfg(target_os = "macos")]
    {
        match sys::renamex(temp, destination, sys::RENAME_EXCL) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(
                ProjectIoError::DestinationExists(destination.display().to_string()),
            ),
            Err(e) if matches!(e.raw_os_error(), Some(45) | Some(22)) => Ok(false),
            Err(e) => Err(io_err(destination, e)),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (temp, destination);
        Ok(false)
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

/// Renames the staged file to `<temp>.conflict.pmv` so neither version is
/// lost, and returns the error naming it (`preserveConflict`).
fn preserve_conflict(temp: TempFile, destination: &Path, message: &str) -> ProjectIoError {
    let mut recovery = temp.path.clone().into_os_string();
    recovery.push(".conflict.pmv");
    let recovery = PathBuf::from(recovery);
    let kept = temp.keep();
    match fs::rename(&kept, &recovery) {
        Ok(()) => {
            let _ = sync_directory(destination.parent().unwrap_or(Path::new(".")));
            ProjectIoError::Conflict {
                message: message.to_owned(),
                recovery_path: recovery,
            }
        }
        Err(_) => ProjectIoError::Conflict {
            message: format!("{message} Recovery finalization failed."),
            recovery_path: kept,
        },
    }
}

/// `publishFile`: atomically publishes `temp` (already flushed) at
/// `destination`, whose content hash will be `new_hash`. On `Err` the
/// destination holds what it held before, except for
/// [`ProjectIoError::Finalize`] (our file is in place but the directory sync
/// failed) and [`ProjectIoError::Conflict`] (both versions kept).
pub(crate) fn publish(
    temp: TempFile,
    destination: &Path,
    mode: &Publish,
    new_hash: &str,
) -> Result<(), ProjectIoError> {
    let dir = destination.parent().unwrap_or(Path::new("."));
    let expected = match mode {
        Publish::CreateNew => None,
        Publish::Replace(v) => Some(v),
    };
    check_version(destination, expected, false)?;
    let incoming = lstat_file(&temp.path)?.ok_or(ProjectIoError::ChangedOnDisk)?;
    match expected {
        Some(expected) => {
            if swap_files(&temp.path, destination)? {
                // `temp.path` now names the displaced file. Verify it is the
                // version we were allowed to replace.
                let displaced = FileVersion::read(&temp.path).ok().flatten();
                let ok = displaced.as_ref().is_some_and(|d| {
                    d.hash == expected.hash
                        && d.stat.same_identity(&expected.stat)
                        && d.stat.same_permissions(&expected.stat)
                });
                if !ok {
                    if displaced.is_none() {
                        return Err(preserve_conflict(
                            temp,
                            destination,
                            "Could not verify the displaced project.",
                        ));
                    }
                    // Only roll back our own published bytes.
                    let current = FileVersion::read(destination).ok().flatten();
                    let ours = current
                        .as_ref()
                        .is_some_and(|c| c.hash == new_hash && c.stat.same_identity(&incoming));
                    if ours && swap_files(&temp.path, destination).unwrap_or(false) {
                        let rolled = FileVersion::read(&temp.path).ok().flatten();
                        if rolled
                            .as_ref()
                            .is_none_or(|r| r.hash != new_hash || !r.stat.same_identity(&incoming))
                        {
                            return Err(preserve_conflict(
                                temp,
                                destination,
                                "The project changed during saving. The original was restored.",
                            ));
                        }
                        let _ = sync_directory(dir);
                        return Err(ProjectIoError::ChangedOnDisk);
                    }
                    return Err(preserve_conflict(
                        temp,
                        destination,
                        "The project changed during saving.",
                    ));
                }
                // Published. `temp` (the displaced old version) is removed on
                // drop.
                drop(temp);
            } else {
                check_version(destination, Some(expected), true)?;
                fs::rename(&temp.path, destination).map_err(|e| io_err(destination, e))?;
                let _ = temp.keep();
            }
        }
        None => {
            if install_exclusive(&temp.path, destination)? {
                let _ = temp.keep();
            } else {
                match fs::hard_link(&temp.path, destination) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        return Err(ProjectIoError::DestinationExists(
                            destination.display().to_string(),
                        ));
                    }
                    Err(e) => return Err(io_err(destination, e)),
                }
                // The link keeps the data; `temp` unlinks its name on drop.
                drop(temp);
            }
        }
    }
    sync_directory(dir).map_err(|e| ProjectIoError::Finalize(e.to_string()))
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
    let uuid = suffix.strip_prefix('.')?.as_bytes();
    let hyphens = [8, 13, 18, 23];
    let shaped = uuid.len() == 36
        && uuid.iter().enumerate().all(|(i, c)| {
            if hyphens.contains(&i) {
                *c == b'-'
            } else {
                matches!(c, b'0'..=b'9' | b'a'..=b'f')
            }
        })
        && uuid[14] == b'4'
        && matches!(uuid[19], b'8' | b'9' | b'a' | b'b');
    if shaped { backup_key(base) } else { None }
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

/// Keep the newest versions within the budgets (always the newest
/// `min_versions` in save order, and always `just_written`) and delete older
/// ones, as the old app pruned its backups after each save (v1.2.0 kept 5).
/// Generations earlier builds moved to `archived/` are older than every kept
/// one and go too, so a project's backups stay bounded. Only regular files
/// with the exact names the app writes are deleted; recovery copies
/// (`*.recovery.pmv`) and anything else stay. Linked folders are refused or
/// skipped, never followed.
pub(crate) fn retain(
    policy: &BackupPolicy,
    just_written: Option<&Path>,
) -> Result<(), ProjectIoError> {
    let Some(dir) = open_backups(policy, false)? else {
        return Ok(());
    };
    let folder_err = |e| io_err(&policy.directory, e);
    let keep = just_written.and_then(Path::file_name);
    let mut bytes = 0u64;
    let mut removed = false;
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
    Ok(())
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
    fn uuids_are_v4_shaped_and_distinct() {
        let a = random_uuid();
        let b = random_uuid();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
    }
}
