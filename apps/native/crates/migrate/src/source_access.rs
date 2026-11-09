//! Originals in folders macOS guards: Desktop, Documents, Downloads, iCloud Drive,
//! cloud storage and other volumes. The first read of one shows a system prompt and
//! blocks until it is answered. Migration reads them only after the person was told
//! (`Choice::Allow`), never waits on an unanswered prompt, and reports what it
//! couldn't reach. Metadata (`lstat`) is not guarded; only opening is.
//!
//! Each guarded folder's answer is an [`Outcome`] the caller persists, so a later
//! run never asks again in the background: only granted folders open then, and
//! a folder seen for the first time waits (`Pending`) for the person. Folders
//! are matched after resolving symlinked ancestors, so an alias into Downloads
//! is Downloads.
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
    /// Someone can be told: a folder without an answer is counted and left
    /// `Pending` for the explanation, never opened.
    Ask,
    /// The person's last answer at first launch (kept for older states; each
    /// folder's own [`Outcome`] is what counts).
    Allow,
    Skip,
    /// Nobody is there to ask (a later launch, behind the open library): only
    /// granted folders open; a new one is recorded `Pending`, never prompted.
    Defer,
}
/// What became of a guarded folder. Persisted by the caller across runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// macOS allowed it: later runs open it directly.
    Granted,
    /// The person read the explanation and chose Continue: the next foreground
    /// open asks macOS (never in the background).
    Allowed,
    /// macOS refused it.
    Denied,
    /// The person chose Skip These Files.
    Skipped,
    /// The prompt went unanswered past the timeout. Never retried silently.
    Unanswered,
    /// Seen while nobody could be asked; waits for the person's answer.
    Pending,
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
    /// Resolved through symlinks, as paths are before matching.
    roots: Vec<(PathBuf, String)>,
    outcomes: BTreeMap<PathBuf, Outcome>,
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
            roots: roots
                .into_iter()
                .map(|(root, name)| (resolve_dir(&root, &mut 0), name))
                .collect(),
            outcomes: BTreeMap::new(),
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

/// Seeds the policy with folder outcomes a previous run persisted.
pub fn remember(outcomes: &BTreeMap<PathBuf, Outcome>) {
    POLICY.with(|p| {
        if let Some(policy) = p.borrow_mut().as_mut() {
            for (root, outcome) in outcomes {
                policy.outcomes.insert(resolve_dir(root, &mut 0), *outcome);
            }
        }
    });
}
/// Every folder outcome known now, for the caller to persist.
pub fn outcomes() -> BTreeMap<PathBuf, Outcome> {
    POLICY.with(|p| {
        p.borrow()
            .as_ref()
            .map_or(BTreeMap::new(), |policy| policy.outcomes.clone())
    })
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
    let resolved = resolve(path);
    policy
        .roots
        .iter()
        .find(|(root, _)| resolved.starts_with(root))
        .cloned()
}
/// `path` with every symlinked ancestor resolved, using `lstat`/`readlink` only:
/// metadata, which macOS doesn't guard (reading a guarded folder would prompt).
/// The last component stays as named; opens refuse a symlink there.
pub fn resolve(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolve_dir(parent, &mut 0).join(name),
        _ => path.to_owned(),
    }
}
fn resolve_dir(path: &Path, hops: &mut u32) -> PathBuf {
    use std::path::Component;
    let parts: Vec<Component> = path.components().collect();
    let mut out = PathBuf::new();
    for (i, part) in parts.iter().enumerate() {
        match part {
            Component::Prefix(_) | Component::RootDir => out.push(part.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => {
                let next = out.join(name);
                let link =
                    std::fs::symlink_metadata(&next).is_ok_and(|m| m.file_type().is_symlink());
                match std::fs::read_link(&next) {
                    Ok(target) if link && *hops < 40 => {
                        *hops += 1;
                        let mut rest = if target.is_absolute() {
                            target
                        } else {
                            out.join(target)
                        };
                        for later in &parts[i + 1..] {
                            rest.push(later.as_os_str());
                        }
                        return resolve_dir(&rest, hops);
                    }
                    _ => out = next,
                }
            }
        }
    }
    out
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
        let background = policy.choice == Choice::Defer;
        match policy.outcomes.get(&root) {
            Some(Outcome::Granted) => Plan::Direct,
            Some(Outcome::Denied | Outcome::Skipped | Outcome::Unanswered) => Plan::Refuse,
            Some(Outcome::Allowed) if !background => {
                Plan::Guarded(root, name, policy.activity.clone(), policy.timeout)
            }
            Some(Outcome::Allowed) => Plan::Refuse,
            // No answer for this folder yet: in the foreground it joins the
            // explanation; in the background it waits.
            Some(Outcome::Pending) | None => {
                if !background {
                    policy.asked.entry(name).or_default().insert(path.into());
                }
                policy.outcomes.insert(root, Outcome::Pending);
                Plan::Refuse
            }
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
    let (result, outcome) = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(file)) => break (Ok(file), Some(Outcome::Granted)),
            Ok(Err(e)) if e.kind() == io::ErrorKind::PermissionDenied => {
                break (Err(e), Some(Outcome::Denied));
            }
            Ok(Err(e)) => break (Err(e), None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break (Err(io::Error::other("folder access check stopped")), None);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if started.elapsed() >= PROMPT_SHOWN && activity.waiting().is_none() {
                    activity.set_waiting(Some(name.clone()));
                }
                if activity.skip.swap(false, Ordering::AcqRel) {
                    break (Err(not_reached()), Some(Outcome::Skipped));
                }
                if started.elapsed() >= timeout {
                    break (Err(not_reached()), Some(Outcome::Unanswered));
                }
            }
        }
    };
    activity.set_waiting(None);
    if let Some(outcome) = outcome {
        POLICY.with(|p| {
            if let Some(policy) = p.borrow_mut().as_mut() {
                policy.outcomes.insert(root, outcome);
            }
        });
    }
    result
}

/// The person's answer to the explanation: every folder waiting on it
/// (`Pending`) becomes `Allowed` or `Skipped`. Folders seen later wait again.
pub fn answer(outcomes: &mut BTreeMap<PathBuf, Outcome>, allow: bool) {
    for outcome in outcomes.values_mut() {
        if *outcome == Outcome::Pending {
            *outcome = if allow {
                Outcome::Allowed
            } else {
                Outcome::Skipped
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn downloads(dir: &Path) -> PathBuf {
        resolve_dir(&dir.join("Downloads"), &mut 0)
    }
    fn roots(dir: &Path) -> Vec<(PathBuf, String)> {
        vec![(dir.join("Downloads"), "Downloads".into())]
    }
    fn read() -> OpenOptions {
        let mut o = OpenOptions::new();
        o.read(true);
        o
    }
    fn guarded(choice: Choice, dir: &Path, timeout: Duration, activity: Arc<Activity>) -> Scope {
        std::fs::create_dir_all(dir.join("Downloads")).unwrap();
        install_with(choice, activity, roots(dir), timeout)
    }
    fn allowed(dir: &Path) {
        remember(&BTreeMap::from([(dir.join("Downloads"), Outcome::Allowed)]));
    }

    #[test]
    fn asking_counts_guarded_files_and_only_the_answer_opens_them() {
        let dir = tempfile::tempdir().unwrap();
        let _scope = guarded(Choice::Ask, dir.path(), PROMPT_TIMEOUT, Arc::default());
        let guarded = dir.path().join("Downloads/a.mov");
        let plain = dir.path().join("b.mov");
        std::fs::write(&guarded, b"a").unwrap();
        std::fs::write(&plain, b"b").unwrap();
        assert!(open(&plain, &read()).is_ok());
        let error = open(&guarded, &read()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(open(&guarded, &read()).is_err());
        assert_eq!(asked(), vec![("Downloads".into(), 1)]);
        let mut persisted = outcomes();
        assert_eq!(persisted[&downloads(dir.path())], Outcome::Pending);
        answer(&mut persisted, true);
        remember(&persisted);
        assert!(open(&guarded, &read()).is_ok());
        assert_eq!(outcomes()[&downloads(dir.path())], Outcome::Granted);
    }

    /// An alias whose ancestor links into Downloads is Downloads.
    #[test]
    fn a_symlinked_ancestor_into_a_guarded_folder_is_that_folder() {
        let dir = tempfile::tempdir().unwrap();
        let _scope = guarded(Choice::Ask, dir.path(), PROMPT_TIMEOUT, Arc::default());
        std::fs::create_dir(dir.path().join("Downloads/inner")).unwrap();
        std::fs::write(dir.path().join("Downloads/inner/a.mov"), b"a").unwrap();
        std::os::unix::fs::symlink(dir.path().join("Downloads"), dir.path().join("alias")).unwrap();
        std::fs::create_dir(dir.path().join("elsewhere")).unwrap();
        std::os::unix::fs::symlink("../alias/inner", dir.path().join("elsewhere/link")).unwrap();
        for path in [
            "alias/inner/a.mov",
            "elsewhere/link/a.mov",
            "elsewhere/../alias/inner/a.mov",
        ] {
            let error = open(&dir.path().join(path), &read()).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{path}");
        }
        assert_eq!(asked(), vec![("Downloads".into(), 3)]);
    }

    /// Behind the open library nobody can be asked: only granted folders open;
    /// a new folder waits, and an unanswered one is never retried.
    #[test]
    fn the_background_never_prompts_and_never_retries() {
        let dir = tempfile::tempdir().unwrap();
        let _scope = guarded(Choice::Defer, dir.path(), PROMPT_TIMEOUT, Arc::default());
        let file = dir.path().join("Downloads/a.mov");
        std::fs::write(&file, b"a").unwrap();
        assert!(open(&file, &read()).is_err());
        assert_eq!(outcomes()[&downloads(dir.path())], Outcome::Pending);
        assert!(asked().is_empty(), "nobody to explain it to");
        for refused in [
            Outcome::Allowed,
            Outcome::Unanswered,
            Outcome::Skipped,
            Outcome::Denied,
        ] {
            remember(&BTreeMap::from([(dir.path().join("Downloads"), refused)]));
            assert!(open(&file, &read()).is_err(), "{refused:?}");
        }
        remember(&BTreeMap::from([(
            dir.path().join("Downloads"),
            Outcome::Granted,
        )]));
        assert!(open(&file, &read()).is_ok());
    }

    /// A FIFO's open blocks until a writer comes, as an unanswered prompt does.
    fn fifo(path: &Path) {
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    }

    #[test]
    fn an_unanswered_prompt_is_skipped_by_the_person_and_never_waited_on_again() {
        let dir = tempfile::tempdir().unwrap();
        let activity = Arc::new(Activity::default());
        let _scope = guarded(Choice::Ask, dir.path(), PROMPT_TIMEOUT, activity.clone());
        allowed(dir.path());
        let blocked = dir.path().join("Downloads/a.mov");
        fifo(&blocked);
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
        assert_eq!(outcomes()[&downloads(dir.path())], Outcome::Skipped);
        let started = Instant::now();
        assert!(open(&blocked, &read()).is_err());
        assert!(started.elapsed() < PROMPT_SHOWN);
    }

    #[test]
    fn an_unanswered_prompt_times_out_and_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let _scope = guarded(
            Choice::Ask,
            dir.path(),
            Duration::from_millis(600),
            Arc::default(),
        );
        allowed(dir.path());
        let blocked = dir.path().join("Downloads/a.mov");
        fifo(&blocked);
        let error = open(&blocked, &read()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(outcomes()[&downloads(dir.path())], Outcome::Unanswered);
    }
}
