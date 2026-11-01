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
    CloudFolder,
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

pub struct FileLock {
    path: PathBuf,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl FileLock {
    pub fn acquire(path: &Path) -> io::Result<Self> {
        let (mut file, created) = match OpenOptions::new()
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
            if pid != 0 && (pid_alive(pid) || now_ms().saturating_sub(heartbeat) < 30_000) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "lock is live or younger than 30 seconds",
                ));
            }
        }
        write_record(&mut file, std::process::id())?;
        let (stop, rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("pm-lock-heartbeat".into())
            .spawn(move || {
                loop {
                    match rx.recv_timeout(Duration::from_secs(5)) {
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            let _ = write_record(&mut file, std::process::id());
                        }
                        _ => {
                            let _ = write_record(&mut file, 0);
                            break;
                        }
                    }
                }
                // The descriptor's close releases flock. File is intentionally kept.
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
fn cloud_name(path: &Path) -> bool {
    path.components().any(|p| {
        let s = p.as_os_str().to_string_lossy().to_lowercase();
        s == "mobile documents"
            || s == "icloud drive"
            || s == "icloud"
            || s.starts_with("com~apple~clouddocs")
            || s.starts_with("dropbox")
            || s.starts_with("onedrive")
    })
}
fn provider_xattr(path: &Path) -> bool {
    #[cfg(not(target_os = "macos"))]
    let _ = path;
    #[cfg(target_os = "macos")]
    {
        let Ok(p) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return true;
        };
        // SAFETY: valid C path; size query, followed by owned output buffer.
        let size = unsafe { libc::listxattr(p.as_ptr(), std::ptr::null_mut(), 0, 0) };
        if size > 0 {
            let mut buf = vec![0u8; size as usize];
            let n = unsafe { libc::listxattr(p.as_ptr(), buf.as_mut_ptr().cast(), buf.len(), 0) };
            return n > 0
                && buf[..n as usize]
                    .split(|b| *b == 0)
                    .any(|k| k.starts_with(b"com.apple.file-provider"));
        }
    }
    false
}
pub fn is_cloud_path(path: &Path) -> bool {
    if cloud_name(path) {
        return true;
    }
    let resolved = fs::canonicalize(existing(path)).unwrap_or_else(|_| path.to_path_buf());
    cloud_name(&resolved) || resolved.ancestors().any(provider_xattr)
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
/// Chromium SingletonLock is normally a symlink `<hostname>-<pid>`.
/// Unknown/remote owners are conservative; an explicitly dead local owner is safe.
pub fn older_electron_running(profile: &Path) -> bool {
    let lock = profile.join("SingletonLock");
    let value = match fs::read_link(&lock) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => match fs::read_to_string(&lock) {
            Ok(s) => s,
            Err(e) => return e.kind() != io::ErrorKind::NotFound,
        },
    };
    let Some((host, pid)) = value.trim().rsplit_once('-') else {
        return true;
    };
    let mut hostname = [0i8; 256];
    // SAFETY: writable hostname buffer; NUL ensured by final initialized byte.
    unsafe {
        if libc::gethostname(hostname.as_mut_ptr(), 255) != 0 {
            return true;
        }
    }
    let local = unsafe { std::ffi::CStr::from_ptr(hostname.as_ptr()) }.to_string_lossy();
    host != local || pid.parse::<u32>().map(pid_alive).unwrap_or(true)
}
pub fn open_decision(path: &Path, electron_profile: Option<&Path>) -> OpenDecision {
    if is_cloud_path(path) {
        OpenDecision::AsCopy(CopyReason::CloudFolder)
    } else if is_read_only(path) {
        OpenDecision::AsCopy(CopyReason::ReadOnly)
    } else if electron_profile.is_some_and(older_electron_running) {
        OpenDecision::AsCopy(CopyReason::OlderElectron)
    } else {
        OpenDecision::Writable
    }
}
