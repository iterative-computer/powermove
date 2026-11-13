//! Local exclusion and conservative open-as-copy policy (plan §6).
//!
//! Two kinds of lock file. A *record* lock ([`FileLock::acquire`]: the
//! library's, and a project's on a volume other machines write) carries its
//! owner's pid and a heartbeat and stays at a stable inode: unlinking a
//! locked inode allows two writers. A *local* lock ([`lock_project`] on a
//! volume only this Mac writes) is an flock on a file in the user's lock
//! folder ([`user_lock_root`]), never beside the project: nothing a sync
//! client can replace or upload, and nothing to refresh.
//!
//! On a volume other machines write, the record lock is believed for 30
//! seconds after its last heartbeat. That measures silence, not death: a
//! machine that stalls longer is no longer excluded by the lock. What then
//! protects its save in progress is the save's own record
//! ([`crate::project_io::Owner`]): another machine never settles it unasked.
use pm_model::json::{Value, parse};
use std::os::{
    fd::AsRawFd,
    unix::{
        ffi::OsStrExt,
        fs::{OpenOptionsExt, PermissionsExt},
    },
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // EPERM means alive, owned by another user.
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyReason {
    ForeignLibrary,
    ChangedOnDisk,
    OfflineFile,
    ReadOnly,
    OlderElectron,
    Locked,
    UncertainLock,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenDecision {
    Writable,
    AsCopy(CopyReason),
}

/// Owns the advisory lock independently of descriptor lifetime. A fork can
/// retain a copy of the same open-file description after our descriptor closes.
struct LockedFile {
    file: File,
    owns_record: bool,
    /// A local lock's own file, removed while the lock is still held.
    remove: Option<PathBuf>,
}
impl std::ops::Deref for LockedFile {
    type Target = File;
    fn deref(&self) -> &File {
        &self.file
    }
}
impl std::ops::DerefMut for LockedFile {
    fn deref_mut(&mut self) -> &mut File {
        &mut self.file
    }
}
impl Drop for LockedFile {
    fn drop(&mut self) {
        if self.owns_record {
            let _ = write_record(&mut self.file, 0);
        }
        // Removed before the unlock: whoever opened this inode meanwhile
        // locks it, finds the name no longer leads to it, and starts over.
        if let Some(path) = &self.remove {
            let _ = fs::remove_file(path);
        }
        // Release only after the last heartbeat/record write, including failed
        // initialization and failed thread spawn. Never unlink the stable inode.
        loop {
            if unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) } == 0
                || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
            {
                break;
            }
        }
    }
}

pub struct FileLock {
    path: PathBuf,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
    /// A local lock, held here (it has no heartbeat thread to hold it).
    _local: Option<LockedFile>,
}
/// Whether `path` still names the file open as `file`.
fn names(path: &Path, file: &File) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::symlink_metadata(path), file.metadata()) {
        (Ok(at), Ok(held)) => at.dev() == held.dev() && at.ino() == held.ino(),
        _ => false,
    }
}
impl FileLock {
    pub fn acquire(path: &Path) -> io::Result<Self> {
        Self::acquire_on(path, is_local_volume(path))
    }
    /// [`Self::acquire`] on a volume that is (`local`) or is not one only
    /// this Mac writes.
    fn acquire_on(path: &Path, local: bool) -> io::Result<Self> {
        let (file, created) = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            Ok(file) => (file, true),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(path)?,
                false,
            ),
            Err(e) => return Err(e),
        };
        // SAFETY: live file descriptor, non-blocking advisory exclusive lock.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut file = LockedFile {
            file,
            owns_record: false,
            remove: None,
        };
        // The lock is on the file we opened. If the name leads elsewhere by
        // now (something replaced the lock file), holding it excludes nobody.
        if !names(path, &file) {
            return Err(io::Error::other(
                "lock file was replaced while it was being taken",
            ));
        }
        if created {
            // Bookkeeping beside the person's project: kept (the inode must
            // stay), not shown in Finder.
            hide(path);
        }
        let mut text = Vec::new();
        file.read_to_end(&mut text)?;
        match record(&text) {
            // We hold the flock, so no process on this Mac holds it (the
            // native app and the transition release both flock). A record
            // younger than 30 s still names a live owner when its pid runs,
            // or when the volume is shared and another Mac may hold it.
            Some((pid, heartbeat)) => {
                let young = now_ms().saturating_sub(heartbeat) < 30_000;
                if pid != 0 && young && (pid_alive(pid) || !local) {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "lock is live or younger than 30 seconds",
                    ));
                }
            }
            // No readable record: a lock created but never written, or torn
            // by a crash or a full disk mid-write. On a volume only this Mac
            // writes, the flock we hold already proves no owner lives, so the
            // record is stale and is written afresh below; refusing it
            // refused the project (or the whole library) until someone
            // deleted the file by hand. On a shared volume flock proves
            // nothing about another Mac: unknown ownership is refused.
            None if created || local => {}
            None => {
                return Err(io::Error::other(
                    "lock record is unreadable on a shared volume",
                ));
            }
        }
        file.owns_record = true;
        write_record(&mut file, std::process::id())?;
        let (stop, rx) = mpsc::channel();
        let named = path.to_path_buf();
        let worker = thread::Builder::new()
            .name("pm-lock-heartbeat".into())
            .spawn(move || {
                while let Err(mpsc::RecvTimeoutError::Timeout) =
                    rx.recv_timeout(Duration::from_secs(5))
                {
                    // A lock file replaced underneath is someone else's now:
                    // its record is not ours to refresh.
                    if names(&named, &file) {
                        let _ = write_record(&mut file, std::process::id());
                    }
                }
                // LockedFile writes PID 0 and explicitly unlocks before close.
            })?;
        Ok(Self {
            path: path.to_path_buf(),
            stop,
            worker: Some(worker),
            _local: None,
        })
    }
    /// A local lock: an flock on `path` and nothing else. No record (on a
    /// volume only this Mac writes the flock is the whole truth), so nothing
    /// is rewritten while it is held. Its file is removed on release, which
    /// is safe because every taker checks that the name still leads to the
    /// file it locked and starts over when it does not.
    fn acquire_local(path: &Path) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        for _ in 0..32 {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)?;
            // SAFETY: live file descriptor, non-blocking advisory exclusive lock.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut locked = LockedFile {
                file,
                owns_record: false,
                remove: None,
            };
            if names(path, &locked) {
                locked.remove = Some(path.to_path_buf());
                return Ok(Self {
                    path: path.to_path_buf(),
                    stop: mpsc::channel().0,
                    worker: None,
                    _local: Some(locked),
                });
            }
            // Its holder released and removed it between our open and our
            // lock: this inode has no name. Take the name's current file.
        }
        Err(io::Error::other("lock file kept being replaced"))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
/// A lock record's owner pid and heartbeat, if it is whole and valid.
fn record(text: &[u8]) -> Option<(u32, u64)> {
    let v = parse(std::str::from_utf8(text).ok()?).ok()?;
    let pid = v
        .get("pid")
        .and_then(Value::as_f64)
        .filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= i32::MAX as f64)? as u32;
    let heartbeat = v
        .get("heartbeat")
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n >= 0.0)? as u64;
    Some((pid, heartbeat))
}
/// Marks the lock file hidden (Finder shows projects, not their locks).
fn hide(path: &Path) {
    #[cfg(target_os = "macos")]
    if let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) {
        // SAFETY: a valid C path; a failure only leaves the file visible.
        unsafe { libc::chflags(c.as_ptr(), libc::UF_HIDDEN) };
    }
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}
fn write_record(file: &mut File, pid: u32) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let text = format!("{{\"pid\":{pid},\"heartbeat\":{}}}", now_ms());
    file.write_all(text.as_bytes())?;
    file.set_len(text.len() as u64)?;
    file.sync_all()
}
impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// Where every build and every profile of the app that this user runs on
/// this Mac keeps its local project locks: one folder per user, so the
/// shipped app, a development build and a second profile exclude each other
/// from a project file. In the user's Application Support (not a temporary
/// or cache folder, which the system may clean under a held lock), found
/// through the user's account, not the environment.
pub fn user_lock_root() -> PathBuf {
    // SAFETY: getpwuid returns a pointer to static storage or null; the
    // directory is copied out before anything else is called.
    let home = unsafe {
        let entry = libc::getpwuid(libc::getuid());
        (!entry.is_null() && !(*entry).pw_dir.is_null()).then(|| {
            PathBuf::from(std::ffi::OsStr::from_bytes(
                std::ffi::CStr::from_ptr((*entry).pw_dir).to_bytes(),
            ))
        })
    }
    .or_else(|| std::env::var_os("HOME").map(PathBuf::from));
    match home {
        Some(home) => home.join("Library/Application Support/Powermove Locks"),
        None => std::env::temp_dir().join("com.powermove.locks"),
    }
}
/// A volume only this Mac writes (`MNT_LOCAL`), where flock is authoritative.
pub(crate) fn is_local_volume(path: &Path) -> bool {
    let Ok(c) = std::ffi::CString::new(existing(path).as_os_str().as_bytes()) else {
        return false;
    };
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: valid path and output storage, read only on success.
    unsafe {
        libc::statfs(c.as_ptr(), stat.as_mut_ptr()) == 0
            && stat.assume_init().f_flags & libc::MNT_LOCAL as u32 != 0
    }
}
/// The record lock beside a project on a volume other machines write.
pub fn project_lock_path(project: &Path) -> PathBuf {
    project.with_extension("pmvlock")
}
/// The local lock of the project file `project`, under `root`: named by the
/// place the project has (its folder's device and inode, and its file name).
/// A save replaces the file's own inode every time; its place stays, and a
/// rename or move is followed by taking the lock of the new place.
pub fn local_lock_path(root: &Path, project: &Path) -> io::Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    // The name as the volume stores it (case, normalization), if it exists.
    let project = fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    let name = project
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file path"))?;
    let folder = match project.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let folder = fs::metadata(folder)?;
    let mut key = format!("{}:{}:", folder.dev(), folder.ino()).into_bytes();
    key.extend_from_slice(name.as_bytes());
    Ok(root.join(format!("{}.lock", &pm_model::hash::sha256_hex(&key)[..32])))
}
/// Exclusive use of the project file `project` among the instances that can
/// reach it.
///
/// On a volume only this Mac writes (a folder a sync client keeps is one),
/// that is every instance sharing `root` ([`user_lock_root`]: every build
/// and profile this user runs): a local lock there, and no file beside the
/// project. What excludes another
/// Mac editing the same synced file is nothing but the save's stale-write
/// check; no lock file can, because a sync client carries files, not locks.
///
/// On a volume other machines write, the record lock beside the project is
/// the only thing every machine sees.
pub fn lock_project(root: &Path, project: &Path) -> io::Result<FileLock> {
    if is_local_volume(project) {
        FileLock::acquire_local(&local_lock_path(root, project)?)
    } else {
        FileLock::acquire(&project_lock_path(project))
    }
}
/// Whether `path` is in a folder a sync client keeps (iCloud Drive, a file
/// provider, a folder named for one): where another computer can change the
/// file underneath an open project.
pub fn is_synced(path: &Path) -> bool {
    fn named(path: &Path) -> bool {
        path.components().any(|c| {
            let s = c.as_os_str().to_string_lossy().to_lowercase();
            [
                "dropbox",
                "onedrive",
                "google drive",
                "icloud drive",
                "icloud",
                "cloudstorage",
                "mobile documents",
            ]
            .contains(&s.as_str())
                || s.starts_with("onedrive-")
                || s.starts_with("googledrive-")
                || s.starts_with("com~apple~clouddocs")
        })
    }
    fn provided(path: &Path) -> bool {
        let Ok(p) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        // SAFETY: a valid C path; the first call sizes the list, the second
        // fills a buffer of that size.
        unsafe {
            let size = libc::listxattr(p.as_ptr(), std::ptr::null_mut(), 0, 0);
            if size <= 0 {
                return false;
            }
            let mut buf = vec![0u8; size as usize];
            let n = libc::listxattr(p.as_ptr(), buf.as_mut_ptr().cast(), buf.len(), 0);
            n > 0
                && buf[..n as usize]
                    .split(|b| *b == 0)
                    .any(|k| k.starts_with(b"com.apple.file-provider"))
        }
    }
    let resolved = fs::canonicalize(existing(path)).unwrap_or_else(|_| path.to_path_buf());
    named(path) || named(&resolved) || resolved.ancestors().any(provided)
}
fn existing(path: &Path) -> &Path {
    let mut p = path;
    while !p.exists() {
        match p.parent() {
            Some(parent) => p = parent,
            None => break,
        }
    }
    p
}
pub fn is_read_only(path: &Path) -> bool {
    let p = existing(path);
    if fs::metadata(p)
        .map(|m| m.permissions().mode() & 0o222 == 0)
        .unwrap_or(true)
    {
        return true;
    }
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else {
        return true;
    };
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: valid path and output storage, initialized only on success.
    unsafe {
        if libc::statvfs(c.as_ptr(), stat.as_mut_ptr()) != 0 {
            return true;
        }
        if stat.assume_init().f_flag & libc::ST_RDONLY as libc::c_ulong != 0 {
            return true;
        }
    }
    let parent = if p.is_dir() {
        p
    } else {
        p.parent().unwrap_or(p)
    };
    fs::metadata(parent)
        .map(|m| m.permissions().mode() & 0o222 == 0)
        .unwrap_or(true)
}
/// Who holds the original app's profile, from its SingletonLock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElectronOwner {
    None,
    /// A live process on this Mac: it can be quit, and the gate waits for that.
    Local,
    /// Another Mac, or an unreadable lock. Nothing tells us when it goes away.
    Unknown,
}
/// Chromium SingletonLock is normally a symlink `<hostname>-<pid>`.
/// Unknown/remote owners are conservative; an explicitly dead local owner is safe.
pub fn electron_owner(profile: &Path) -> ElectronOwner {
    let lock = profile.join("SingletonLock");
    let value = match fs::read_link(&lock) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => match fs::read_to_string(&lock) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return ElectronOwner::None,
            Err(_) => return ElectronOwner::Unknown,
        },
    };
    let Some((host, pid)) = value.trim().rsplit_once('-') else {
        return ElectronOwner::Unknown;
    };
    let mut hostname = [0i8; 256];
    // SAFETY: writable hostname buffer; NUL ensured by final initialized byte.
    unsafe {
        if libc::gethostname(hostname.as_mut_ptr(), 255) != 0 {
            return ElectronOwner::Unknown;
        }
    }
    let local = unsafe { std::ffi::CStr::from_ptr(hostname.as_ptr()) }.to_string_lossy();
    if host != local {
        return ElectronOwner::Unknown;
    }
    match pid.parse::<u32>() {
        Ok(pid) if pid_alive(pid) => ElectronOwner::Local,
        Ok(_) => ElectronOwner::None,
        Err(_) => ElectronOwner::Unknown,
    }
}
pub fn older_electron_running(profile: &Path) -> bool {
    electron_owner(profile) != ElectronOwner::None
}
/// Whether `path` can be opened and saved in place. A file in a folder a
/// sync client keeps (iCloud Drive, Dropbox, OneDrive, any file provider) is
/// an ordinary local file to this app, as it was to v1: it opens and saves
/// in place, and a version the client lands underneath is caught by the
/// save's stale-write check, never overwritten.
pub fn open_decision(path: &Path, electron_profile: Option<&Path>) -> OpenDecision {
    if is_read_only(path) {
        OpenDecision::AsCopy(CopyReason::ReadOnly)
    } else if electron_profile.is_some_and(older_electron_running) {
        OpenDecision::AsCopy(CopyReason::OlderElectron)
    } else {
        OpenDecision::Writable
    }
}

#[cfg(test)]
mod record_tests {
    use super::*;

    /// A local lock removes its file on release. Takers that race a release
    /// never end up two at once: one that locked a file the releaser had
    /// already removed sees that the name no longer leads to it and takes
    /// the name's current file instead.
    #[test]
    fn a_local_lock_taken_while_another_releases_has_one_holder() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = std::env::temp_dir().join(format!("pm-lock-local-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("one.lock");
        let holders = AtomicUsize::new(0);
        let taken = AtomicUsize::new(0);
        thread::scope(|scope| {
            for _ in 0..6 {
                scope.spawn(|| {
                    for _ in 0..400 {
                        if let Ok(lock) = FileLock::acquire_local(&path) {
                            assert_eq!(holders.fetch_add(1, Ordering::SeqCst), 0);
                            taken.fetch_add(1, Ordering::SeqCst);
                            thread::yield_now();
                            holders.fetch_sub(1, Ordering::SeqCst);
                            drop(lock);
                        }
                    }
                });
            }
        });
        assert!(taken.load(Ordering::SeqCst) > 0);
        assert!(!path.exists(), "the last release removed it");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_synced_folder_is_known_by_its_name() {
        assert!(is_synced(Path::new("/Users/someone/Dropbox/job/cut.pmv")));
        assert!(is_synced(Path::new(
            "/Users/someone/Library/Mobile Documents/com~apple~CloudDocs/cut.pmv"
        )));
        assert!(is_synced(Path::new(
            "/Users/someone/Library/CloudStorage/OneDrive-Personal/cut.pmv"
        )));
        assert!(!is_synced(&std::env::temp_dir().join("job/cut.pmv")));
    }

    /// With the flock held, an empty, torn or malformed record is stale on a
    /// volume only this Mac writes (it is rewritten and the lock taken), and
    /// refused on a shared one, where another Mac may own it.
    #[test]
    fn an_unreadable_record_is_stale_only_where_flock_is_authoritative() {
        let dir = std::env::temp_dir().join(format!("pm-lock-record-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (n, torn) in [
            &b""[..],
            b"{\"pid\":41",
            b"broken",
            b"{\"pid\":-1,\"heartbeat\":1}",
        ]
        .into_iter()
        .enumerate()
        {
            let path = dir.join(format!("shared-{n}.pmvlock"));
            fs::write(&path, torn).unwrap();
            assert!(FileLock::acquire_on(&path, false).is_err(), "{torn:?}");
            assert_eq!(
                fs::read(&path).unwrap(),
                torn,
                "a refused record is not rewritten"
            );
            let path = dir.join(format!("local-{n}.pmvlock"));
            fs::write(&path, torn).unwrap();
            let lock =
                FileLock::acquire_on(&path, true).unwrap_or_else(|e| panic!("{torn:?}: {e}"));
            let (pid, _) = record(&fs::read(&path).unwrap()).expect("a whole record");
            assert_eq!(pid, std::process::id());
            drop(lock);
            assert_eq!(record(&fs::read(&path).unwrap()).unwrap().0, 0);
        }
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod inheritance_tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn final_drop_unlocks_the_stable_inode_despite_a_forked_descriptor() {
        let root = std::env::temp_dir().join(crate::new_id("lock-inheritance"));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("synthetic.lock");
        let lock = FileLock::acquire(&path).unwrap();
        let inode = std::fs::metadata(&path).unwrap().ino();
        let mut pipe = [-1; 2];
        // Only async-signal-safe libc calls execute in the child. In particular,
        // the inherited Rust allocator, heartbeat and mutexes are never entered.
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        let child = unsafe { libc::fork() };
        if child == 0 {
            unsafe {
                libc::close(pipe[1]);
                let mut byte = 0u8;
                libc::read(pipe[0], (&mut byte as *mut u8).cast(), 1);
                libc::_exit(0);
            }
        }
        unsafe {
            libc::close(pipe[0]);
        }
        if child < 0 {
            unsafe {
                libc::close(pipe[1]);
            }
            panic!("fork: {}", io::Error::last_os_error());
        }
        drop(lock);
        let reopened = FileLock::acquire(&path);
        let stable_inode = std::fs::metadata(&path).unwrap().ino();
        // Reap before checking the result, including the historical failure.
        unsafe {
            let byte = 1u8;
            libc::write(pipe[1], (&byte as *const u8).cast(), 1);
            libc::close(pipe[1]);
            libc::waitpid(child, std::ptr::null_mut(), 0);
        }
        let reopened = reopened.unwrap_or_else(|error| {
            panic!("clean reopen must not retain the fork's advisory lock: {error}")
        });
        assert_eq!(inode, stable_inode, "the lock file must never be unlinked");
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
}
