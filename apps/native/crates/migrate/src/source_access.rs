//! Originals in folders macOS guards: Desktop, Documents, Downloads, iCloud Drive,
//! cloud storage and other volumes. The first read of one shows a system prompt and
//! blocks until it is answered. Migration reads them only after the person was told
//! (`Choice::Allow`), never waits on an unanswered prompt, and reports what it
//! couldn't reach. Metadata (`lstat`) is not guarded; only opening is.
//!
//! The policy is per thread: the migration worker installs it, and code on that
//! thread opens sources through [`open`]. Without a policy every open is direct.
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Choice {
    /// Not told yet: guarded sources are counted, never opened.
    Ask,
    Allow,
    Skip,
}

/// Shared with the window, which polls it: bytes for the running step, and the
/// folder macOS is asking about while an open waits on its prompt.
#[derive(Debug, Default)]
pub struct Activity {
    bytes: AtomicU64,
    total: AtomicU64,
    waiting: Mutex<Option<String>>,
    skip: AtomicBool,
}
impl Activity {
    /// (done, total) bytes of the running step; total 0 means unknown.
    pub fn bytes(&self) -> (u64, u64) {
        (
            self.bytes.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }
    /// The folder whose macOS prompt is unanswered, by its Finder name.
    pub fn waiting(&self) -> Option<String> {
        self.waiting.lock().unwrap().clone()
    }
    /// The person chose to go on without the folder macOS is asking about.
    pub fn skip_waiting(&self) {
        self.skip.store(true, Ordering::Release);
    }
    fn set_waiting(&self, folder: Option<String>) {
        *self.waiting.lock().unwrap() = folder;
    }
}

/// An unanswered prompt stops being waited on after this; the files are then
/// reported as not reached, exactly as if the person had skipped them.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);
/// An open that takes longer than this is waiting on a prompt.
const PROMPT_SHOWN: Duration = Duration::from_millis(400);

struct Policy {
    choice: Choice,
    roots: Vec<(PathBuf, String)>,
    granted: BTreeSet<PathBuf>,
    skipped: BTreeSet<PathBuf>,
    asked: BTreeMap<String, BTreeSet<PathBuf>>,
    activity: Arc<Activity>,
    timeout: Duration,
}
thread_local! {
    static POLICY: RefCell<Option<Policy>> = const { RefCell::new(None) };
}

/// Removes the thread's policy when dropped.
pub struct Scope(());
impl Drop for Scope {
    fn drop(&mut self) {
        POLICY.with(|p| *p.borrow_mut() = None);
    }
}

/// This Mac's guarded folders, by the name Finder shows.
pub fn default_roots() -> Vec<(PathBuf, String)> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        let home = PathBuf::from(home);
        for (relative, name) in [
            ("Desktop", "Desktop"),
            ("Documents", "Documents"),
            ("Downloads", "Downloads"),
            ("Library/Mobile Documents", "iCloud Drive"),
        ] {
            roots.push((home.join(relative), name.to_owned()));
        }
        if let Ok(entries) = std::fs::read_dir(home.join("Library/CloudStorage")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                roots.push((entry.path(), name));
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir("/Volumes") {
        for entry in entries.flatten() {
            // The startup disk's entry links back to `/`, which isn't guarded.
            if entry.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            roots.push((entry.path(), name));
        }
    }
    roots
}

pub fn install(choice: Choice, activity: Arc<Activity>) -> Scope {
    install_with(choice, activity, default_roots(), PROMPT_TIMEOUT)
}
/// Tests name their own guarded folders and timeout.
pub fn install_with(
    choice: Choice,
    activity: Arc<Activity>,
    roots: Vec<(PathBuf, String)>,
    timeout: Duration,
) -> Scope {
    POLICY.with(|p| {
        *p.borrow_mut() = Some(Policy {
            choice,
            roots,
            granted: BTreeSet::new(),
            skipped: BTreeSet::new(),
            asked: BTreeMap::new(),
            activity,
            timeout,
        })
    });
    Scope(())
}
/// The installed policy with `choice`, or a fresh one for this scope. Later
/// launches merge media behind the open library with the recorded answer.
pub fn ensure(choice: Choice) -> Option<Scope> {
    if POLICY.with(|p| p.borrow().is_some()) {
        choose(choice);
        None
    } else {
        Some(install(choice, Arc::default()))
    }
}
/// Changes the installed policy's choice, keeping what it learned.
pub fn choose(choice: Choice) {
    POLICY.with(|p| {
        if let Some(policy) = p.borrow_mut().as_mut() {
            policy.choice = choice;
            policy.asked.clear();
        }
    });
}

/// Guarded folders seen while asking, with how many files each holds.
pub fn asked() -> Vec<(String, usize)> {
    POLICY.with(|p| {
        p.borrow().as_ref().map_or(Vec::new(), |policy| {
            policy
                .asked
                .iter()
                .map(|(name, paths)| (name.clone(), paths.len()))
                .collect()
        })
    })
}

/// The Finder name of the guarded folder holding `path`, if any.
pub fn folder_of(path: &Path) -> Option<String> {
    POLICY.with(|p| {
        p.borrow()
            .as_ref()
            .and_then(|policy| root_of(policy, path).map(|(_, name)| name))
    })
}
fn root_of(policy: &Policy, path: &Path) -> Option<(PathBuf, String)> {
    policy
        .roots
        .iter()
        .find(|(root, _)| path.starts_with(root))
        .cloned()
}

/// Counts bytes the running step has read once (never its verification passes).
pub fn add_bytes(n: u64) {
    POLICY.with(|p| {
        if let Some(policy) = p.borrow().as_ref() {
            policy.activity.bytes.fetch_add(n, Ordering::Relaxed);
        }
    });
}
/// A step that moves about `total` bytes begins; 0 means unknown.
pub fn begin_bytes(total: u64) {
    POLICY.with(|p| {
        if let Some(policy) = p.borrow().as_ref() {
            policy.activity.bytes.store(0, Ordering::Relaxed);
            policy.activity.total.store(total, Ordering::Relaxed);
        }
    });
}

/// A source that couldn't be opened because of folder access: denied by macOS,
/// skipped, not answered, or not asked yet. The caller records it as not reached.
pub fn is_unreached(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|e| e.downcast_ref::<io::Error>())
        .any(|e| e.kind() == io::ErrorKind::PermissionDenied)
}

fn not_reached() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "folder access was not given",
    )
}

/// Opens `path`. A guarded one is opened only with `Choice::Allow`, on a helper
/// thread, so an unanswered macOS prompt never holds the caller.
pub fn open(path: &Path, options: &OpenOptions) -> io::Result<File> {
    enum Plan {
        Direct,
        Refuse,
        Guarded(PathBuf, String, Arc<Activity>, Duration),
    }
    let plan = POLICY.with(|p| {
        let mut borrowed = p.borrow_mut();
        let Some(policy) = borrowed.as_mut() else {
            return Plan::Direct;
        };
        let Some((root, name)) = root_of(policy, path) else {
            return Plan::Direct;
        };
        if policy.skipped.contains(&root) {
            return Plan::Refuse;
        }
        if policy.granted.contains(&root) {
            return Plan::Direct;
        }
        match policy.choice {
            Choice::Ask => {
                policy.asked.entry(name).or_default().insert(path.into());
                Plan::Refuse
            }
            Choice::Skip => Plan::Refuse,
            Choice::Allow => Plan::Guarded(root, name, policy.activity.clone(), policy.timeout),
        }
    });
    let (root, name, activity, timeout) = match plan {
        Plan::Direct => return options.open(path),
        Plan::Refuse => return Err(not_reached()),
        Plan::Guarded(root, name, activity, timeout) => (root, name, activity, timeout),
    };
    let (tx, rx) = mpsc::channel();
    let (target, opener) = (path.to_owned(), options.clone());
    // Left running if abandoned: it returns once the prompt goes away.
    std::thread::spawn(move || {
        let _ = tx.send(opener.open(&target));
    });
    activity.skip.store(false, Ordering::Release);
    let started = Instant::now();
    let result = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => break Some(result),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Some(Err(io::Error::other("folder access check stopped")));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if started.elapsed() >= PROMPT_SHOWN && activity.waiting().is_none() {
                    activity.set_waiting(Some(name.clone()));
                }
                if activity.skip.swap(false, Ordering::AcqRel) || started.elapsed() >= timeout {
                    break None;
                }
            }
        }
    };
    activity.set_waiting(None);
    POLICY.with(|p| {
        if let Some(policy) = p.borrow_mut().as_mut() {
            match &result {
                Some(Ok(_)) => {
                    policy.granted.insert(root);
                }
                Some(Err(e)) if e.kind() == io::ErrorKind::PermissionDenied => {
                    policy.skipped.insert(root);
                }
                None => {
                    policy.skipped.insert(root);
                }
                Some(Err(_)) => {}
            }
        }
    });
    result.unwrap_or_else(|| Err(not_reached()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(dir: &Path) -> Vec<(PathBuf, String)> {
        vec![(dir.join("Downloads"), "Downloads".into())]
    }
    fn read() -> OpenOptions {
        let mut o = OpenOptions::new();
        o.read(true);
        o
    }

    #[test]
    fn asking_counts_guarded_files_without_opening_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Downloads")).unwrap();
        let guarded = dir.path().join("Downloads/a.mov");
        let plain = dir.path().join("b.mov");
        std::fs::write(&guarded, b"a").unwrap();
        std::fs::write(&plain, b"b").unwrap();
        let _scope = install_with(
            Choice::Ask,
            Arc::default(),
            roots(dir.path()),
            PROMPT_TIMEOUT,
        );
        assert!(open(&plain, &read()).is_ok());
        let error = open(&guarded, &read()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(open(&guarded, &read()).is_err());
        assert_eq!(asked(), vec![("Downloads".into(), 1)]);
        choose(Choice::Allow);
        assert!(open(&guarded, &read()).is_ok());
        assert!(asked().is_empty());
    }

    /// A FIFO's open blocks until a writer comes, as an unanswered prompt does.
    fn fifo(path: &Path) {
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    }

    #[test]
    fn an_unanswered_prompt_is_skipped_by_the_person_and_never_waited_on_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Downloads")).unwrap();
        let blocked = dir.path().join("Downloads/a.mov");
        fifo(&blocked);
        let activity = Arc::new(Activity::default());
        let _scope = install_with(
            Choice::Allow,
            activity.clone(),
            roots(dir.path()),
            PROMPT_TIMEOUT,
        );
        let watcher = {
            let activity = activity.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                while activity.waiting().is_none() {
                    assert!(started.elapsed() < Duration::from_secs(10));
                    std::thread::sleep(Duration::from_millis(20));
                }
                assert_eq!(activity.waiting().as_deref(), Some("Downloads"));
                activity.skip_waiting();
            })
        };
        let error = open(&blocked, &read()).unwrap_err();
        watcher.join().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(activity.waiting(), None);
        // The folder is skipped for the rest of the run: no second wait.
        let started = Instant::now();
        assert!(open(&blocked, &read()).is_err());
        assert!(started.elapsed() < PROMPT_SHOWN);
    }

    #[test]
    fn an_unanswered_prompt_times_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Downloads")).unwrap();
        let blocked = dir.path().join("Downloads/a.mov");
        fifo(&blocked);
        let _scope = install_with(
            Choice::Allow,
            Arc::default(),
            roots(dir.path()),
            Duration::from_millis(600),
        );
        let error = open(&blocked, &read()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
