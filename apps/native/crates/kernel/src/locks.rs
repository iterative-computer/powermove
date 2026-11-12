//! Local exclusion and conservative open-as-copy policy (plan §6).
//! Lock files remain at stable inodes: unlinking a locked inode allows two writers.
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
}
impl FileLock {
    pub fn acquire(path: &Path) -> io::Result<Self> {
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
        };
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        if !created && text.is_empty() {
            return Err(io::Error::other("empty lock has unknown ownership"));
        }
        if !text.is_empty() {
            let v = parse(&text).map_err(|_| io::Error::other("unreadable lock record"))?;
            let pid = v
                .get("pid")
                .and_then(Value::as_f64)
                .filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= i32::MAX as f64)
                .ok_or_else(|| io::Error::other("invalid lock pid"))? as u32;
            let heartbeat = v
                .get("heartbeat")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or_else(|| io::Error::other("invalid lock heartbeat"))?
                as u64;
            // We hold the flock, so no process on this Mac holds it (the
            // native app and the transition release both flock). A record
            // younger than 30 s still names a live owner when its pid runs,
            // or when the volume is shared and another Mac may hold it.
            let young = now_ms().saturating_sub(heartbeat) < 30_000;
            if pid != 0 && young && (pid_alive(pid) || !is_local_volume(path)) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "lock is live or younger than 30 seconds",
                ));
            }
        }
        file.owns_record = true;
        write_record(&mut file, std::process::id())?;
        let (stop, rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("pm-lock-heartbeat".into())
            .spawn(move || {
                while let Err(mpsc::RecvTimeoutError::Timeout) =
                    rx.recv_timeout(Duration::from_secs(5))
                {
                    let _ = write_record(&mut file, std::process::id());
                }
                // LockedFile writes PID 0 and explicitly unlocks before close.
            })?;
        Ok(Self {
            path: path.to_path_buf(),
            stop,
            worker: Some(worker),
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
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

/// A volume only this Mac writes (`MNT_LOCAL`), where flock is authoritative.
fn is_local_volume(path: &Path) -> bool {
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
pub fn project_lock_path(project: &Path) -> PathBuf {
    project.with_extension("pmvlock")
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
