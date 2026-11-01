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

use std::ffi::OsString;
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
/// `<userData>/Backups/<projectId>/`; the shell lane chooses the directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupPolicy {
    pub directory: PathBuf,
    /// Most versions kept (12 in the old app).
    pub max_versions: usize,
    /// Byte budget across versions (1 GiB in the old app).
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

/// `2026-10-31T21-38-05-123Z` (an ISO-8601 instant with `:` and `.` replaced,
/// as the old app named its backups).
fn backup_stamp(now: std::time::SystemTime) -> String {
    let d = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs() as i64;
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
        d.subsec_millis()
    )
}

fn is_backup_name(name: &str) -> bool {
    // `^\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z\.pmv(?:-\d+)?$`
    let b = name.as_bytes();
    if b.len() < 28 {
        return false;
    }
    let shape = b"dddd-dd-ddTdd-dd-dd-dddZ.pmv";
    for (i, s) in shape.iter().enumerate() {
        let ok = match s {
            b'd' => b[i].is_ascii_digit(),
            c => b[i] == *c,
        };
        if !ok {
            return false;
        }
    }
    let rest = &b[28..];
    rest.is_empty()
        || (rest.len() > 1 && rest[0] == b'-' && rest[1..].iter().all(u8::is_ascii_digit))
}

/// Backups of the policy's directory, newest first.
pub fn list_backups(policy: &BackupPolicy) -> Result<Vec<PathBuf>, ProjectIoError> {
    let entries = match fs::read_dir(&policy.directory) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err(&policy.directory, e)),
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_backup_name(n))
        .collect();
    names.sort_by(|a, b| b.cmp(a));
    Ok(names
        .into_iter()
        .map(|n| policy.directory.join(n))
        .collect())
}

/// Copies (clones) the current destination into the backup directory and
/// syncs it. Returns the backup's path.
pub(crate) fn backup(policy: &BackupPolicy, destination: &Path) -> Result<PathBuf, ProjectIoError> {
    fs::create_dir_all(&policy.directory).map_err(|e| io_err(&policy.directory, e))?;
    let stamp = backup_stamp(std::time::SystemTime::now());
    let mut collision = 0u32;
    loop {
        let name = if collision == 0 {
            format!("{stamp}.pmv")
        } else {
            format!("{stamp}.pmv-{collision}")
        };
        let file = policy.directory.join(name);
        if fs::symlink_metadata(&file).is_ok() {
            collision += 1;
            continue;
        }
        fs::copy(destination, &file).map_err(|e| io_err(&file, e))?;
        let synced = File::open(&file)
            .and_then(|h| h.sync_all())
            .map_err(|e| io_err(&file, e))
            .and_then(|()| sync_directory(&policy.directory));
        if let Err(e) = synced {
            let _ = fs::remove_file(&file);
            return Err(e);
        }
        return Ok(file);
    }
}

/// `retain`: keeps at most `max_versions` and `max_bytes`, but never fewer
/// than `min_versions`, newest first.
pub(crate) fn retain(policy: &BackupPolicy) -> Result<(), ProjectIoError> {
    let mut bytes = 0u64;
    for (index, file) in list_backups(policy)?.into_iter().enumerate() {
        let size = fs::symlink_metadata(&file).map(|m| m.len()).unwrap_or(0);
        bytes += size;
        if index >= policy.min_versions
            && (index >= policy.max_versions || bytes > policy.max_bytes)
        {
            fs::remove_file(&file).map_err(|e| io_err(&file, e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_names_match_the_old_pattern() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_793_489_885_123);
        let stamp = backup_stamp(t);
        assert_eq!(stamp, "2026-10-31T23-38-05-123Z");
        assert!(is_backup_name(&format!("{stamp}.pmv")));
        assert!(is_backup_name(&format!("{stamp}.pmv-3")));
        assert!(!is_backup_name(&format!("{stamp}.pmv-")));
        assert!(!is_backup_name(&format!("{stamp}.pmv.recovery.pmv")));
        assert!(!is_backup_name("notes.pmv"));
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
