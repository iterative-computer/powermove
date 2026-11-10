//! Crash-safe journal, checkpoints and recovery (plan v6 §0 "never losing
//! work", §1 "crash recovery ≤ 1 s", §6 "journals keyed to base revision").
//!
//! # Layout
//!
//! ```text
//! <journal root>/<project id>/<session>/
//!     journal-00000000.log      checksummed JSON lines, append-only
//!     checkpoint-00000001.json  state at the start of segment 1
//!     journal-00000001.log
//! ```
//!
//! One **session** per open of a project. Its first segment's header names
//! the session's **base hash**: the SHA-256 of the canonical document the
//! session started from (the library slot or file as opened). Every durable
//! change ([`Tap`]: published revisions, history pushes, undo/redo cursor
//! moves, squashes, clears, workspace writes) is one line
//! `<16 hex sha256> <json>`.
//!
//! # Durability (the rule that picked it)
//!
//! * **Write before acknowledging.** The actor `write(2)`s the call's lines
//!   before it replies, so a process crash (SIGKILL, abort, OOM) loses no
//!   acknowledged write: the bytes are in the kernel's page cache.
//! * **`F_FULLFSYNC` within the interval** (default 200 ms) on a background
//!   thread bounds loss on power failure or OS crash to < 1 s, and the actor
//!   never waits on the disk (§3.2: no caller blocks).
//! * **Checkpoints** every 50 revisions or 4 MiB of segment (today's
//!   `CHECKPOINT_REVISIONS`/`MAX_JOURNAL_BYTES`) rotate to a new segment and
//!   write `checkpoint-<n>.json` on a background thread (temp file, full
//!   sync, rename, directory sync). Older segments are deleted only after
//!   the newer checkpoint is durable, so recovery always has a chain. A large
//!   document checkpoints by segment size alone: fifty edits are a few
//!   kilobytes of journal, and rewriting 180 MB for them buys nothing.
//!
//! # Entries
//!
//! A revision's line holds the operations the edit produced, id-keyed where
//! they can be, so a line is the size of the change. Nothing here supersedes
//! or compacts earlier lines: recovery replays every line in order, so an
//! operation at `layers` never has to stand in for a later whole value.
//!
//! # Recovery
//!
//! [`recover`] loads the newest valid checkpoint, replays the following
//! segments line by line, and stops at the first torn or corrupt line (the
//! tail of an interrupted write). It **refuses to replay onto a different
//! base**: when the caller's current base (what the library slot or file
//! holds now) differs from the session's base hash, the result is
//! [`Recovery::Copy`], to be opened as a copy, never merged. Sessions are
//! never deleted by recovery; a new session that adopts recovered work
//! supersedes them once its own first checkpoint is durable, and until then
//! recovery continues from the session it superseded.
//!
//! **Save points.** A save journals `{"t":"save"}` with the hash of the
//! document it is about to publish and the identity of the undo history it
//! writes, durably, before it replaces the file; a save whose save point
//! cannot be made durable does not publish. The file is then either the old
//! base or that save, and recovery replays onto either (from the start, or
//! from just after the save point) without waiting for any checkpoint. A
//! save point is adopted only when the file holds both its document and its
//! history: another writer that installed the same document with its own
//! history gets a copy, never this session's history over its own. Later
//! heads and checkpoints list a save as an ancestor only once it published.
//!
//! **Format.** Heads and checkpoints carry `version` ([`FORMAT`]). A reader
//! replays every version up to its own and refuses a newer one as made by a
//! newer Powermove (the files are kept), instead of failing on a record it
//! does not know. Version 2 added save points and ancestors with history
//! identities.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pm_model::json::{Map, Value, parse, to_canonical_string};
use pm_model::ops::{ops_from_value, ops_to_value};
use pm_model::patch::{patches_from_value, patches_to_value};
use pm_model::{Document, Op};

use crate::commands::Origin;
use crate::document::Tap;
use crate::history::{History, HistoryRecord, HistorySnapshot};

const JOURNAL_FORMAT: &str = "powermove-kernel-journal";
const CHECKPOINT_FORMAT: &str = "powermove-kernel-checkpoint";
/// The journal and checkpoint version this build writes and the newest it
/// reads.
pub const FORMAT: u32 = 2;
/// Why a journal from a newer build is not replayed.
pub const NEWER_FORMAT: &str = "This recovery journal was made by a newer Powermove. Open the project there; the journal is kept.";

/// A document this size or larger checkpoints by journal size (`VERIFY_MIN_CHARS`).
const LARGE_DOCUMENT_BYTES: u64 = 4 << 20;

/// Journal knobs.
#[derive(Clone, Debug)]
pub struct JournalConfig {
    /// Requested `F_FULLFSYNC` cadence. Capped at 100 ms to leave headroom
    /// inside the 200 ms P1 flush budget; zero is clamped to 1 ms.
    pub flush_interval: Duration,
    /// Checkpoint after this many revisions (`CHECKPOINT_REVISIONS`).
    pub checkpoint_revisions: u64,
    /// ... or this many segment bytes (`MAX_JOURNAL_BYTES`).
    pub max_segment_bytes: u64,
    /// Failure injection (tests): while the flag is set, the checkpoint
    /// writer waits before writing each checkpoint, as a slow disk or a
    /// large document does.
    #[doc(hidden)]
    pub hold_checkpoints: Option<Arc<(Mutex<bool>, Condvar)>>,
    /// Failure injection (tests): the next save point fails this way.
    #[doc(hidden)]
    pub fail_save_point: Option<Arc<Mutex<Option<SaveFault>>>>,
}

/// How an injected save-point failure goes (tests).
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFault {
    /// Half the line reaches the file, then the write fails.
    TornWrite,
    /// The line is written; its `F_FULLFSYNC` fails.
    SyncFails,
    /// A torn write the segment cannot be truncated back from.
    TornWriteUntruncatable,
}

impl Default for JournalConfig {
    fn default() -> Self {
        JournalConfig {
            flush_interval: Duration::from_millis(200),
            checkpoint_revisions: 50,
            max_segment_bytes: 4 * 1024 * 1024,
            hold_checkpoints: None,
            fail_save_point: None,
        }
    }
}

/// What a checkpoint holds.
#[derive(Clone, Debug)]
pub struct CheckpointState {
    pub revision: u64,
    pub document: Arc<Document>,
    pub history: HistorySnapshot,
    pub workspace: Value,
}

// ── background durability ──────────────────────────────────────────────────

struct SyncTarget {
    file: File,
    dirty: AtomicBool,
    syncing: Mutex<()>,
}

impl SyncTarget {
    fn sync(&self) -> io::Result<()> {
        // A second caller must wait even after the flusher claimed `dirty`.
        let _guard = self.syncing.lock().expect("sync target");
        if self.dirty.swap(false, Ordering::AcqRel)
            && let Err(e) = full_sync(&self.file)
        {
            self.dirty.store(true, Ordering::Release);
            return Err(e);
        }
        Ok(())
    }
}

/// `F_FULLFSYNC` where available (macOS: flushes the drive cache too).
pub(crate) fn full_sync(file: &File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: a valid open descriptor; F_FULLFSYNC takes no argument.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } == 0 {
            return Ok(());
        }
    }
    file.sync_all()
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    full_sync(&File::open(dir)?)
}

enum Job {
    Checkpoint {
        dir: PathBuf,
        project: String,
        base: String,
        segment: u64,
        state: Box<CheckpointState>,
        /// States the session passed through that may be on disk (its base
        /// and its published saves): any of them is an ancestor of `state`.
        bases: Vec<Ancestor>,
        /// Sessions this one replaces once its checkpoint is durable.
        supersedes: Vec<PathBuf>,
        /// Told the checkpoint's size, which says whether the document is large.
        measured: Arc<AtomicU64>,
    },
    /// Sessions a durable save made redundant (deleted in queue order).
    Retire(Vec<PathBuf>),
    Barrier(mpsc::Sender<()>),
}

struct Shared {
    targets: Mutex<Vec<Arc<SyncTarget>>>,
    stop: AtomicBool,
    wake: Condvar,
    lock: Mutex<()>,
    errors: Mutex<VecDeque<String>>,
}

/// The kernel's durability threads: one fsync loop, one checkpoint writer.
pub struct Durability {
    shared: Arc<Shared>,
    jobs: Option<mpsc::Sender<Job>>,
    threads: Vec<thread::JoinHandle<()>>,
    config: JournalConfig,
}

impl Durability {
    pub fn new(config: JournalConfig) -> Durability {
        let shared = Arc::new(Shared {
            targets: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
            wake: Condvar::new(),
            lock: Mutex::new(()),
            errors: Mutex::new(VecDeque::new()),
        });
        let flusher = {
            let shared = shared.clone();
            let interval = config
                .flush_interval
                .clamp(Duration::from_millis(1), Duration::from_millis(100));
            thread::Builder::new()
                .name("pm-kernel-fsync".into())
                .spawn(move || {
                    loop {
                        let cycle = Instant::now();
                        let stop = shared.stop.load(Ordering::Acquire);
                        let targets: Vec<Arc<SyncTarget>> =
                            shared.targets.lock().expect("targets").clone();
                        for t in &targets {
                            if let Err(e) = t.sync() {
                                shared
                                    .errors
                                    .lock()
                                    .expect("errors")
                                    .push_back(e.to_string());
                            }
                        }
                        // Forget rotated-away files once synced.
                        shared.targets.lock().expect("targets").retain(|t| {
                            Arc::strong_count(t) > 2 || t.dirty.load(Ordering::Acquire)
                        });
                        if stop {
                            break;
                        }
                        let guard = shared.lock.lock().expect("lock");
                        // Charge fsync work to this cycle, rather than adding
                        // a full sleep after every disk flush (N1).
                        let _ = shared
                            .wake
                            .wait_timeout(guard, interval.saturating_sub(cycle.elapsed()))
                            .expect("wait");
                    }
                })
                .expect("spawn fsync thread")
        };
        let (jobs, rx) = mpsc::channel::<Job>();
        let writer = {
            let shared = shared.clone();
            let hold = config.hold_checkpoints.clone();
            thread::Builder::new()
                .name("pm-kernel-checkpoint".into())
                .spawn(move || {
                    for job in rx {
                        match job {
                            Job::Checkpoint {
                                dir,
                                project,
                                base,
                                segment,
                                state,
                                bases,
                                supersedes,
                                measured,
                            } => {
                                if let Some(hold) = &hold {
                                    let (held, wake) = &**hold;
                                    let mut held = held.lock().expect("hold");
                                    while *held {
                                        held = wake.wait(held).expect("hold");
                                    }
                                }
                                let bytes = match write_checkpoint(
                                    &dir, &project, &base, &bases, segment, &state,
                                ) {
                                    Ok(bytes) => bytes,
                                    Err(e) => {
                                        shared
                                            .errors
                                            .lock()
                                            .expect("errors")
                                            .push_back(format!("checkpoint {segment}: {e}"));
                                        continue;
                                    }
                                };
                                measured.store(bytes, Ordering::Release);
                                prune_before(&dir, segment);
                                for old in supersedes {
                                    let _ = fs::remove_dir_all(old);
                                }
                            }
                            Job::Retire(old) => {
                                for old in old {
                                    let _ = fs::remove_dir_all(old);
                                }
                            }
                            Job::Barrier(done) => {
                                let _ = done.send(());
                            }
                        }
                    }
                })
                .expect("spawn checkpoint thread")
        };
        Durability {
            shared,
            jobs: Some(jobs),
            threads: vec![flusher, writer],
            config,
        }
    }

    pub fn config(&self) -> &JournalConfig {
        &self.config
    }

    fn register(&self, file: &File) -> io::Result<Arc<SyncTarget>> {
        let target = Arc::new(SyncTarget {
            file: file.try_clone()?,
            dirty: AtomicBool::new(false),
            syncing: Mutex::new(()),
        });
        self.shared
            .targets
            .lock()
            .expect("targets")
            .push(target.clone());
        Ok(target)
    }

    fn submit(&self, job: Job) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(job);
        }
    }

    /// Waits until queued checkpoints are written (tests, clean shutdown).
    /// Never call on the main thread.
    pub fn wait_idle(&self) {
        let (tx, rx) = mpsc::channel();
        self.submit(Job::Barrier(tx));
        let _ = rx.recv();
    }

    /// Finish queued checkpoint creation/rename/pruning, then fsync every
    /// registered segment (including rotated or closed journals). The actor
    /// must not submit writes while waiting. Errors never certify durability.
    pub fn barrier(&self) -> io::Result<()> {
        self.wait_idle();
        let targets = self.shared.targets.lock().expect("targets").clone();
        for target in targets {
            target.sync()?;
        }
        let errors = self.take_errors();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(errors.join("; ")))
        }
    }

    /// A handle that flushes every journal segment from any thread.
    pub fn syncer(&self) -> JournalSync {
        JournalSync(self.shared.clone())
    }

    /// Background errors since the last call (shown as a notice, never
    /// silently dropped).
    pub fn take_errors(&self) -> Vec<String> {
        self.shared
            .errors
            .lock()
            .expect("errors")
            .drain(..)
            .collect()
    }
}

/// Flushes journal segments without the kernel actor (Quit Now while the
/// actor is busy): every line the actor acknowledged is already written, and
/// this makes it durable. Checkpoints are not waited for; recovery never
/// needs one that has not landed (see [`Journal::mark_save`]).
#[derive(Clone)]
pub struct JournalSync(Arc<Shared>);

impl JournalSync {
    /// `F_FULLFSYNC`s every registered segment. Blocking; never on the main
    /// thread.
    pub fn sync(&self) -> io::Result<()> {
        let targets = self.0.targets.lock().expect("targets").clone();
        for target in targets {
            target.sync()?;
        }
        Ok(())
    }
}

impl Drop for Durability {
    fn drop(&mut self) {
        self.jobs.take();
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

// ── the journal ─────────────────────────────────────────────────────────────

/// One session's journal (owned by the kernel actor).
pub struct Journal {
    /// The last checkpoint's size: what "large" is measured by.
    checkpoint_bytes: Arc<AtomicU64>,
    dir: PathBuf,
    project: String,
    base: String,
    segment: u64,
    file: File,
    target: Arc<SyncTarget>,
    bytes: u64,
    revisions: u64,
    /// Checkpoint 0, held back while the session's state is its base.
    deferred: Option<Box<CheckpointState>>,
    /// The base and every state a save published since: what a file or slot
    /// may hold that this session's work descends from.
    bases: Vec<Ancestor>,
    /// The save point of a save that has not published yet.
    pending: Option<Ancestor>,
}

/// A state a session's work descends from: a document hash and, for a save,
/// the identity of the undo history it wrote ([`History::identity`]). A
/// session's start base has none: it is the base whatever history it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ancestor {
    pub hash: String,
    pub history: Option<String>,
}

impl Ancestor {
    fn start(hash: &str) -> Ancestor {
        Ancestor {
            hash: hash.to_owned(),
            history: None,
        }
    }
    /// Whether a base whose document hashes to `hash`, with undo history of
    /// identity `history` (`None`: unknown), is this state.
    fn is(&self, hash: &str, history: Option<&str>) -> bool {
        self.hash == hash
            && self
                .history
                .as_deref()
                .is_none_or(|own| history == Some(own))
    }
}

/// How many save points a session remembers as possible bases.
const MAX_BASES: usize = 32;

fn segment_name(n: u64) -> String {
    format!("journal-{n:08}.log")
}

fn checkpoint_name(n: u64) -> String {
    format!("checkpoint-{n:08}.json")
}

fn numbered(dir: &Path, prefix: &str, suffix: &str) -> Vec<u64> {
    let mut out: Vec<u64> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_prefix(prefix)?
                .strip_suffix(suffix)?
                .parse()
                .ok()
        })
        .collect();
    out.sort_unstable();
    out
}

/// A new session directory name: sortable by creation time.
fn session_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("s{nanos:024}-{:010}", std::process::id())
}

/// Sessions of a project, oldest first.
pub fn sessions(project_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(project_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('s') && e.path().is_dir())
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

impl Journal {
    /// Starts a session under `project_dir` for a document whose canonical
    /// hash is `base`, checkpointing `initial` in the background.
    /// `supersedes`: older sessions (of this project or the one it copies)
    /// to delete once that checkpoint is durable.
    ///
    /// `saved`: `initial` is exactly what the project's file or library slot
    /// durably holds (document, history and workspace). Its checkpoint then
    /// waits for the first change: until one, recovery starts from that base
    /// ([`recover_from`]), and a large project is not rewritten just to be
    /// opened or saved. The superseded sessions hold nothing the base lacks
    /// and are deleted at once.
    pub fn create(
        durability: &Durability,
        project_dir: &Path,
        project: &str,
        base: &str,
        initial: CheckpointState,
        supersedes: Vec<PathBuf>,
        saved: bool,
    ) -> io::Result<Journal> {
        let dir = project_dir.join(session_name());
        fs::create_dir_all(&dir)?;
        sync_dir(project_dir)?;
        let bases = vec![Ancestor::start(base)];
        // Adopted recovered work starts from checkpoint 0, not the base.
        // Until that lands, the session it supersedes here still holds the
        // state, and recovery continues from it.
        let start = (!saved && !supersedes.is_empty()).then(|| {
            supersedes
                .iter()
                .filter(|old| old.parent() == Some(project_dir))
                .filter_map(|old| old.file_name()?.to_str())
                .max()
                .unwrap_or("")
        });
        let (file, target) = open_segment(
            durability,
            &dir,
            Head {
                project,
                base,
                bases: &bases,
                start,
                segment: 0,
                revision: initial.revision,
            },
        )?;
        let checkpoint_bytes = Arc::new(AtomicU64::new(0));
        let mut deferred = None;
        if saved {
            if !supersedes.is_empty() {
                durability.submit(Job::Retire(supersedes));
            }
            deferred = Some(Box::new(initial));
        } else {
            durability.submit(Job::Checkpoint {
                dir: dir.clone(),
                project: project.to_owned(),
                base: base.to_owned(),
                segment: 0,
                state: Box::new(initial),
                bases: bases.clone(),
                supersedes,
                measured: checkpoint_bytes.clone(),
            });
        }
        Ok(Journal {
            bases,
            pending: None,
            deferred,
            checkpoint_bytes,
            dir,
            project: project.to_owned(),
            base: base.to_owned(),
            segment: 0,
            file,
            target,
            bytes: 0,
            revisions: 0,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn base_hash(&self) -> &str {
        &self.base
    }

    /// Appends one call's durable changes. Returns after `write(2)`; the
    /// fsync follows within the flush interval. `state` is asked for only
    /// when a checkpoint is due.
    pub(crate) fn append(
        &mut self,
        durability: &Durability,
        taps: &[Tap],
        workspace: Option<&Value>,
        state: impl FnOnce() -> CheckpointState,
    ) -> io::Result<()> {
        if taps.is_empty() && workspace.is_none() {
            return Ok(());
        }
        // A layout change alone (a tab switched, a panel moved) isn't project
        // work: its record replays on the base, so the base's checkpoint waits
        // for the first edit instead of rewriting the whole document.
        if !taps.is_empty()
            && let Some(initial) = self.deferred.take()
        {
            durability.submit(Job::Checkpoint {
                dir: self.dir.clone(),
                project: self.project.clone(),
                base: self.base.clone(),
                segment: 0,
                state: initial,
                bases: self.bases.clone(),
                supersedes: Vec::new(),
                measured: self.checkpoint_bytes.clone(),
            });
        }
        let mut buf = String::new();
        for tap in taps {
            if matches!(tap, Tap::Rev { .. }) {
                self.revisions += 1;
            }
            line(&mut buf, &encode_tap(tap));
        }
        if let Some(ws) = workspace {
            let mut m = Map::new();
            m.insert("t", Value::from("ws"));
            m.insert("v", ws.clone());
            line(&mut buf, &Value::Object(m));
        }
        (&self.file).write_all(buf.as_bytes())?;
        self.target.dirty.store(true, Ordering::Release);
        self.bytes += buf.len() as u64;
        let config = durability.config();
        let large = self.checkpoint_bytes.load(Ordering::Acquire) >= LARGE_DOCUMENT_BYTES;
        if (!large && self.revisions >= config.checkpoint_revisions)
            || self.bytes >= config.max_segment_bytes
        {
            self.rotate(durability, state())?;
        }
        Ok(())
    }

    fn rotate(&mut self, durability: &Durability, state: CheckpointState) -> io::Result<()> {
        let next = self.segment + 1;
        let (file, target) = open_segment(
            durability,
            &self.dir,
            Head {
                project: &self.project,
                base: &self.base,
                bases: &self.bases,
                start: None,
                segment: next,
                revision: state.revision,
            },
        )?;
        // The old segment stays registered until its last fsync.
        self.file = file;
        self.target = target;
        self.segment = next;
        self.bytes = 0;
        self.revisions = 0;
        durability.submit(Job::Checkpoint {
            dir: self.dir.clone(),
            project: self.project.clone(),
            base: self.base.clone(),
            segment: next,
            state: Box::new(state),
            bases: self.bases.clone(),
            supersedes: Vec::new(),
            measured: self.checkpoint_bytes.clone(),
        });
        Ok(())
    }

    /// Records that a save is about to publish the state as of this line
    /// (document hash `hash` and undo history `history`, at `revision`) and
    /// makes it durable before returning: the barrier a save must pass
    /// before the file or slot is replaced. Whatever the disk then holds,
    /// the old base or this save, recovery replays onto it: from the start,
    /// or from just after this line, so edits journalled while the save ran
    /// (or that a Quit Now left without a checkpoint) are never stranded on
    /// a base that is gone.
    ///
    /// On an error the save must not publish. The segment is cut back to
    /// before the line, so a torn save point never stands between the edits
    /// before it and those after; when it cannot be cut, the journal moves
    /// to a new segment and checkpoint (`state`).
    pub(crate) fn mark_save(
        &mut self,
        durability: &Durability,
        hash: &str,
        history: &str,
        revision: u64,
        state: impl FnOnce() -> CheckpointState,
    ) -> io::Result<()> {
        let mut point = Map::new();
        point.insert("h", Value::from(hash));
        point.insert("hh", Value::from(history));
        point.insert("r", Value::Number(revision as f64));
        let mut m = Map::new();
        m.insert("t", Value::from("save"));
        m.insert("v", Value::Object(point));
        let mut buf = String::new();
        line(&mut buf, &Value::Object(m));
        let fault = durability
            .config()
            .fail_save_point
            .as_ref()
            .and_then(|f| f.lock().expect("save fault").take());
        let len = self.file.metadata()?.len();
        let written = match fault {
            Some(SaveFault::TornWrite | SaveFault::TornWriteUntruncatable) => {
                (&self.file).write_all(&buf.as_bytes()[..buf.len() / 2])?;
                Err(io::Error::other("injected torn save point"))
            }
            _ => (&self.file).write_all(buf.as_bytes()),
        };
        let synced = written.and_then(|()| {
            self.target.dirty.store(true, Ordering::Release);
            if fault == Some(SaveFault::SyncFails) {
                return Err(io::Error::other("injected save point sync failure"));
            }
            self.target.sync()
        });
        if let Err(e) = synced {
            let cut = if fault == Some(SaveFault::TornWriteUntruncatable) {
                Err(io::Error::other("injected truncate failure"))
            } else {
                self.file.set_len(len)
            };
            self.target.dirty.store(true, Ordering::Release);
            if cut.is_err() {
                self.rotate(durability, state())?;
            }
            return Err(e);
        }
        self.bytes += buf.len() as u64;
        self.pending = Some(Ancestor {
            hash: hash.to_owned(),
            history: Some(history.to_owned()),
        });
        Ok(())
    }

    /// The save whose save point was journalled last has published: later
    /// heads and checkpoints list it as an ancestor.
    pub(crate) fn published(&mut self) {
        let Some(save) = self.pending.take() else {
            return;
        };
        self.bases.retain(|b| *b != save);
        self.bases.push(save);
        if self.bases.len() > MAX_BASES {
            // The start base stays: it is what the session was opened on.
            self.bases.remove(1);
        }
    }

    /// Flushes now (clean close; tests). Never on the main thread.
    pub fn sync_now(&self) -> io::Result<()> {
        self.target.sync()
    }

    /// Call only after the durability barrier, with no concurrent writer.
    pub(crate) fn archive(&mut self) -> io::Result<()> {
        archive_path(&mut self.dir)
    }
}

/// Archive rather than delete rejected recovery work. No checkpoint writer
/// may still reference this path: the actor must pass its barrier first.
pub(crate) fn archive_session(mut path: PathBuf) -> io::Result<()> {
    archive_path(&mut path)
}

fn archive_path(path: &mut PathBuf) -> io::Result<()> {
    if path.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("discarded")) {
        let parent = path.parent().expect("session parent");
        let archive = parent.join("discarded");
        fs::create_dir_all(&archive)?;
        // Make the destination directory durable before moving the only copy.
        sync_dir(parent)?;
        let destination = archive.join(path.file_name().expect("session name"));
        if destination.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Journal archive exists",
            ));
        }
        fs::rename(&*path, &destination)?;
        // Keep a live journal's path correct even if directory syncing fails;
        // retrying DiscardClose can finish the barrier without another move.
        *path = destination;
    }
    let archive = path.parent().expect("archive directory");
    sync_dir(archive)?;
    sync_dir(archive.parent().expect("project directory"))
}

/// A segment's header line.
struct Head<'a> {
    project: &'a str,
    base: &'a str,
    bases: &'a [Ancestor],
    /// The session holding the start state until checkpoint 0 lands.
    start: Option<&'a str>,
    segment: u64,
    revision: u64,
}

fn open_segment(
    durability: &Durability,
    dir: &Path,
    Head {
        project,
        base,
        bases,
        start,
        segment,
        revision,
    }: Head,
) -> io::Result<(File, Arc<SyncTarget>)> {
    let path = dir.join(segment_name(segment));
    let mut options = OpenOptions::new();
    options.create_new(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    let mut head = Map::new();
    head.insert("t", Value::from("head"));
    head.insert("format", Value::from(JOURNAL_FORMAT));
    head.insert("version", Value::Number(f64::from(FORMAT)));
    head.insert("project", Value::from(project));
    head.insert("base", Value::from(base));
    head.insert("bases", bases_value(bases));
    if let Some(continues) = start {
        // `continues`: the session holding the start state ("" for none).
        head.insert("start", Value::from("checkpoint"));
        head.insert("continues", Value::from(continues));
    }
    head.insert("segment", Value::Number(segment as f64));
    head.insert("revision", Value::Number(revision as f64));
    let mut buf = String::new();
    line(&mut buf, &Value::Object(head));
    (&file).write_all(buf.as_bytes())?;
    full_sync(&file)?;
    sync_dir(dir)?;
    let target = durability.register(&file)?;
    Ok((file, target))
}

/// `bases`: a start base as its hash, a save as `{"h","hh"}`.
fn bases_value(bases: &[Ancestor]) -> Value {
    Value::Array(
        bases
            .iter()
            .map(|b| match &b.history {
                None => Value::from(b.hash.as_str()),
                Some(history) => {
                    let mut m = Map::new();
                    m.insert("h", Value::from(b.hash.as_str()));
                    m.insert("hh", Value::from(history.as_str()));
                    Value::Object(m)
                }
            })
            .collect(),
    )
}

/// A head's or checkpoint's version: `Err` for one newer than this build.
fn version_of(m: &Map) -> Result<u32, String> {
    let version = m.get("version").and_then(Value::as_f64).unwrap_or(1.0);
    if version > f64::from(FORMAT) {
        return Err(NEWER_FORMAT.into());
    }
    Ok(version as u32)
}

/// A head's or checkpoint's ancestors. Version 1 listed saves by document
/// hash alone, before they published: only its `base` is an ancestor.
fn bases_of(m: &Map, base: &str) -> Vec<Ancestor> {
    let mut bases: Vec<Ancestor> = Vec::new();
    if version_of(m).is_ok_and(|v| v >= 2)
        && let Some(listed) = m.get("bases").and_then(Value::as_array)
    {
        for b in listed {
            match b {
                Value::String(_) => bases.extend(b.as_str().map(Ancestor::start)),
                Value::Object(o) => {
                    if let (Some(h), Some(hh)) = (
                        o.get("h").and_then(Value::as_str),
                        o.get("hh").and_then(Value::as_str),
                    ) {
                        bases.push(Ancestor {
                            hash: h.to_owned(),
                            history: Some(hh.to_owned()),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    if !bases.iter().any(|b| b.hash == base && b.history.is_none()) {
        bases.insert(0, Ancestor::start(base));
    }
    bases
}

fn checksum(text: &str) -> String {
    pm_model::hash::sha256_hex(text.as_bytes())[..16].to_owned()
}

fn line(buf: &mut String, value: &Value) {
    let text = to_canonical_string(value);
    buf.push_str(&checksum(&text));
    buf.push(' ');
    buf.push_str(&text);
    buf.push('\n');
}

pub(crate) fn origin_name(origin: Origin) -> &'static str {
    match origin {
        Origin::Interface => "interface",
        Origin::Agent => "agent",
        Origin::Mcp => "mcp",
        Origin::Extension => "extension",
    }
}

pub(crate) fn origin_from(name: Option<&str>) -> Origin {
    match name {
        Some("agent") => Origin::Agent,
        Some("mcp") => Origin::Mcp,
        Some("extension") => Origin::Extension,
        _ => Origin::Interface,
    }
}

fn encode_record(m: &mut Map, r: &HistoryRecord) {
    m.insert("label", Value::from(r.label.as_str()));
    m.insert("origin", Value::from(origin_name(r.origin)));
    m.insert("f", ops_to_value(&r.forward));
    m.insert("i", ops_to_value(&r.inverse));
    if let Some(saved) = &r.saved {
        m.insert(
            "s",
            Value::Array(vec![patches_to_value(&saved.0), patches_to_value(&saved.1)]),
        );
    }
}

fn encode_tap(tap: &Tap) -> Value {
    let mut m = Map::new();
    match tap {
        Tap::Rev {
            base,
            revision,
            ops,
        } => {
            m.insert("t", Value::from("rev"));
            m.insert("b", Value::Number(*base as f64));
            m.insert("r", Value::Number(*revision as f64));
            m.insert("ops", ops_to_value(ops));
        }
        Tap::Push(r) => {
            m.insert("t", Value::from("push"));
            encode_record(&mut m, r);
        }
        Tap::PushExternal(r) => {
            m.insert("t", Value::from("ext"));
            m.insert("label", Value::from(r.label.as_str()));
        }
        Tap::Undo { external } => {
            m.insert("t", Value::from("undo"));
            m.insert("x", Value::Bool(*external));
        }
        Tap::Redo { external } => {
            m.insert("t", Value::from("redo"));
            m.insert("x", Value::Bool(*external));
        }
        Tap::Squash { count, record } => {
            m.insert("t", Value::from("squash"));
            m.insert("n", Value::Number(*count as f64));
            encode_record(&mut m, record);
        }
        Tap::Metadata { ops } => {
            m.insert("t", Value::from("metadata"));
            m.insert("ops", ops_to_value(ops));
        }
        Tap::Clear => {
            m.insert("t", Value::from("clear"));
        }
    }
    Value::Object(m)
}

// ── checkpoints ─────────────────────────────────────────────────────────────

struct HashingFile<W: Write> {
    out: W,
    hasher: sha2::Sha256,
    error: Option<io::Error>,
}

impl<W: Write> pm_model::json::write::Sink for HashingFile<W> {
    fn push_str(&mut self, s: &str) {
        use sha2::Digest;
        if self.error.is_some() {
            return;
        }
        self.hasher.update(s.as_bytes());
        if let Err(e) = self.out.write_all(s.as_bytes()) {
            self.error = Some(e);
        }
    }
}

/// `checkpoint-<n>.json`: one metadata line, then the canonical document.
fn write_checkpoint(
    dir: &Path,
    project: &str,
    base: &str,
    bases: &[Ancestor],
    segment: u64,
    state: &CheckpointState,
) -> io::Result<u64> {
    use sha2::Digest;
    let final_path = dir.join(checkpoint_name(segment));
    let tmp = dir.join(format!("{}.tmp", checkpoint_name(segment)));
    // The metadata line carries the document's hash, so the document is
    // streamed (hashed as it goes) into its own temp file first, then the
    // two are assembled into the checkpoint's temp file.
    let doc_tmp = dir.join(format!("{}.doc.tmp", checkpoint_name(segment)));
    let digest = {
        let doc_file = File::create(&doc_tmp)?;
        let mut sink = HashingFile {
            out: BufWriter::with_capacity(64 * 1024, doc_file),
            hasher: sha2::Sha256::new(),
            error: None,
        };
        match state.document.root() {
            Some(root) => pm_model::json::write::write_canonical(root, &mut sink),
            None => pm_model::json::write::Sink::push_str(&mut sink, "null"),
        }
        if let Some(e) = sink.error.take() {
            return Err(e);
        }
        sink.out.flush()?;
        pm_model::hash::hex(&sink.hasher.finalize().into())
    };
    let mut meta = Map::new();
    meta.insert("format", Value::from(CHECKPOINT_FORMAT));
    meta.insert("version", Value::Number(f64::from(FORMAT)));
    meta.insert("project", Value::from(project));
    meta.insert("base", Value::from(base));
    meta.insert("bases", bases_value(bases));
    meta.insert("segment", Value::Number(segment as f64));
    meta.insert("revision", Value::Number(state.revision as f64));
    meta.insert("documentHash", Value::from(digest.as_str()));
    meta.insert("history", state.history.to_saved());
    meta.insert("workspace", state.workspace.clone());
    let mut out = BufWriter::with_capacity(64 * 1024, File::create(&tmp)?);
    out.write_all(to_canonical_string(&Value::Object(meta)).as_bytes())?;
    out.write_all(b"\n")?;
    io::copy(&mut File::open(&doc_tmp)?, &mut out)?;
    out.flush()?;
    let file = out.into_inner().map_err(|e| e.into_error())?;
    full_sync(&file)?;
    let bytes = file.metadata()?.len();
    drop(file);
    fs::remove_file(&doc_tmp)?;
    fs::rename(&tmp, &final_path)?;
    sync_dir(dir)?;
    Ok(bytes)
}

/// Deletes segments and checkpoints older than `segment` (its checkpoint is
/// durable now).
fn prune_before(dir: &Path, segment: u64) {
    for n in numbered(dir, "journal-", ".log") {
        if n < segment {
            let _ = fs::remove_file(dir.join(segment_name(n)));
        }
    }
    for n in numbered(dir, "checkpoint-", ".json") {
        if n < segment {
            let _ = fs::remove_file(dir.join(checkpoint_name(n)));
        }
    }
}

struct Loaded {
    revision: u64,
    base: String,
    bases: Vec<Ancestor>,
    document: Option<pm_model::Node>,
    history: LoadedHistory,
    workspace: Value,
}
enum LoadedHistory {
    Saved(Value),
    Ready(History),
}

/// A checkpoint whose recorded document hash is `base`'s takes its document
/// from `base` (a session's first checkpoint is the state it opened), so only
/// its metadata line is read. `Err` for one a newer build wrote.
fn read_checkpoint(path: &Path, base: Option<&Document>) -> Result<Option<Loaded>, String> {
    use std::io::BufRead;
    let Ok(file) = File::open(path) else {
        return Ok(None);
    };
    let mut file = io::BufReader::with_capacity(1 << 20, file);
    let mut meta = String::new();
    let meta = file
        .read_line(&mut meta)
        .ok()
        .and_then(|_| parse(meta.strip_suffix('\n')?).ok());
    let Some(Value::Object(m)) = meta else {
        return Ok(None);
    };
    if m.get("format").and_then(Value::as_str) != Some(CHECKPOINT_FORMAT) {
        return Ok(None);
    }
    version_of(&m)?;
    Ok(load_checkpoint(&m, file, base))
}

fn load_checkpoint(
    m: &Map,
    mut file: io::BufReader<File>,
    base: Option<&Document>,
) -> Option<Loaded> {
    let hash = m.get("documentHash")?.as_str()?;
    let revision = m.get("revision")?.as_f64()? as u64;
    let document = match base.filter(|b| b.canonical_hash().as_deref() == Some(hash)) {
        Some(base) => base.root().cloned(),
        None => {
            let mut doc = Vec::new();
            file.read_to_end(&mut doc).ok()?;
            if pm_model::hash::sha256_hex(&doc) != hash {
                return None;
            }
            Some(pm_model::Node::parse_bytes(&doc, revision).ok()?)
        }
    };
    let base = m.get("base")?.as_str()?.to_owned();
    Some(Loaded {
        revision,
        bases: bases_of(m, &base),
        base,
        document,
        history: LoadedHistory::Saved(m.get("history")?.clone()),
        workspace: m.get("workspace").cloned().unwrap_or(Value::Null),
    })
}

// ── recovery ────────────────────────────────────────────────────────────────

/// Recovered state.
#[derive(Debug)]
pub struct Recovered {
    pub project: String,
    /// The session's base hash, or the save point the caller's current base
    /// matched.
    pub base: String,
    /// Every base the recovered state descends from: the session's base and
    /// each save it journalled. A current base that is one of them, history
    /// included, is safe to adopt.
    pub bases: Vec<Ancestor>,
    pub revision: u64,
    pub document: Document,
    pub history: History,
    pub workspace: Value,
    /// Journal lines replayed after the checkpoint.
    pub replayed: usize,
    /// A torn or corrupt tail was ignored (an interrupted write).
    pub truncated: bool,
    /// The session directory (kept until a new session supersedes it).
    pub session: PathBuf,
}

#[derive(Debug)]
pub enum Recovery {
    /// No journal: nothing to recover.
    Nothing,
    /// Safe to adopt: the library slot or file still holds the base.
    Recovered(Recovered),
    /// The base changed elsewhere since the session began: open the
    /// recovered work as a copy, never on top of the changed project.
    Copy {
        recovered: Recovered,
        current_base: String,
    },
    /// The journal cannot be replayed (no checkpoint and no matching base
    /// document). The files are kept.
    Unrecoverable { session: PathBuf, reason: String },
}

/// Recovers the newest session of a project. `current_base`: the canonical
/// hash of what the library slot or file holds now (`None`: unknown, adopt).
/// `base_document`: that document, used only when no checkpoint landed yet
/// and its hash matches the session's base.
pub fn recover(
    project_dir: &Path,
    current_base: Option<&str>,
    base_document: Option<&Value>,
) -> Recovery {
    let document = base_document.map(|v| Document::new(Some(v.clone()), 1));
    recover_from(
        project_dir,
        current_base,
        document.as_ref().map(|document| Base {
            document,
            history: None,
            workspace: None,
        }),
    )
}

/// What a session's base holds, for replay when no checkpoint landed.
#[derive(Clone, Copy)]
pub struct Base<'a> {
    /// The canonical document (its hash is checked against the session's).
    pub document: &'a Document,
    /// Its undo history and workspace (`None`: none).
    pub history: Option<&'a History>,
    pub workspace: Option<&'a Value>,
}

/// [`recover`] with the base's undo history and workspace, so a session
/// replayed from its base keeps them.
pub fn recover_from(
    project_dir: &Path,
    current_base: Option<&str>,
    base: Option<Base>,
) -> Recovery {
    let Some(session) = sessions(project_dir).pop() else {
        return Recovery::Nothing;
    };
    if let Some(head) = numbered(&session, "journal-", ".log")
        .first()
        .and_then(|&n| segment_head(&session.join(segment_name(n))))
        && let Err(reason) = version_of(&head)
    {
        return Recovery::Unrecoverable { session, reason };
    }
    if numbered(&session, "checkpoint-", ".json").is_empty()
        && !has_changes(&session)
        && continued(&session).is_none()
    {
        // Opened (or saved) and never changed: the base holds all of it.
        return Recovery::Nothing;
    }
    match recover_session(&session, base) {
        Err(reason) => Recovery::Unrecoverable { session, reason },
        Ok(mut recovered) => match current_base {
            // A base the session passed through (its start, or one of its
            // saves with the history that save wrote) is an ancestor of the
            // recovered state: adopt it.
            Some(current)
                if recovered
                    .bases
                    .iter()
                    .any(|b| b.is(current, history_identity(base).as_deref())) =>
            {
                recovered.base = current.to_owned();
                Recovery::Recovered(recovered)
            }
            Some(current) if current != recovered.base => Recovery::Copy {
                recovered,
                current_base: current.to_owned(),
            },
            _ => Recovery::Recovered(recovered),
        },
    }
}

/// The identity of the undo history a base holds (`None`: no base given).
fn history_identity(base: Option<Base>) -> Option<String> {
    let base = base?;
    Some(match base.history {
        Some(history) => history.identity(),
        None => History::new().identity(),
    })
}

/// The session a session continues while its checkpoint 0 has not landed.
fn continued(session: &Path) -> Option<PathBuf> {
    if !numbered(session, "checkpoint-", ".json").is_empty() {
        return None;
    }
    let first = *numbered(session, "journal-", ".log").first()?;
    let head = segment_head(&session.join(segment_name(first)))?;
    if head.get("start")?.as_str()? != "checkpoint" {
        return None;
    }
    let name = head.get("continues")?.as_str()?;
    let earlier = session.parent()?.join(name);
    (!name.is_empty() && !name.contains('/') && earlier.is_dir()).then_some(earlier)
}

fn segment_head(path: &Path) -> Option<Map> {
    let mut text = String::new();
    File::open(path).ok()?.read_to_string(&mut text).ok()?;
    let first = text.lines().next()?;
    let (sum, json) = first.split_once(' ')?;
    (checksum(json) == sum).then_some(())?;
    parse(json).ok()?.as_object().cloned()
}

/// Whether any segment of `session` holds an edit after its header. Workspace
/// records alone aren't unsaved work: the layout is also filed in the app's
/// state, and the window never called them changes.
fn has_changes(session: &Path) -> bool {
    numbered(session, "journal-", ".log").into_iter().any(|n| {
        let mut text = Vec::new();
        File::open(session.join(segment_name(n)))
            .and_then(|mut f| f.read_to_end(&mut text))
            .is_ok_and(|_| {
                text.split(|&b| b == b'\n')
                    .filter(|l| !l.is_empty())
                    .skip(1)
                    .any(|l| !is_bookkeeping_line(l))
            })
    })
}
/// `<checksum> {"t":"ws",…}` or `{"t":"save",…}`: a workspace record or a
/// save point, neither of them an edit.
fn is_bookkeeping_line(line: &[u8]) -> bool {
    line.splitn(2, |&b| b == b' ').nth(1).is_some_and(|json| {
        json.starts_with(br#"{"t":"ws","#) || json.starts_with(br#"{"t":"save","#)
    })
}

/// A segment's checksummed lines after its header, up to the first torn or
/// corrupt one (`Err(())` marks where the valid prefix ends). `Err` for a
/// segment a newer build wrote.
fn segment_lines(path: &Path) -> Result<Vec<Result<Value, ()>>, String> {
    let mut text = Vec::new();
    File::open(path)
        .and_then(|mut f| f.read_to_end(&mut text))
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&text);
    let mut lines = text.split_inclusive('\n');
    if let Some((sum, json)) = lines.next().and_then(|h| h.trim_end().split_once(' '))
        && checksum(json) == sum
        && let Ok(Value::Object(head)) = parse(json)
    {
        version_of(&head)?;
    }
    let mut out = Vec::new();
    for raw in lines {
        let value = raw
            .strip_suffix('\n')
            .and_then(|body| body.split_once(' '))
            .filter(|(sum, json)| checksum(json) == *sum)
            .and_then(|(_, json)| parse(json).ok());
        match value {
            Some(value) => out.push(Ok(value)),
            None => {
                out.push(Err(()));
                break;
            }
        }
    }
    Ok(out)
}

/// A `save` line ([`Journal::mark_save`]): the ancestor it names, `None`
/// for one without a history identity (version 1, never adopted), and its
/// revision.
fn save_point(value: &Value) -> Option<(Option<Ancestor>, u64)> {
    let m = value.as_object()?;
    if m.get("t")?.as_str()? != "save" {
        return None;
    }
    let point = m.get("v")?.as_object()?;
    let ancestor = point.get("hh").and_then(Value::as_str).map(|hh| Ancestor {
        hash: point
            .get("h")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        history: Some(hh.to_owned()),
    });
    Some((ancestor, point.get("r")?.as_f64()? as u64))
}

fn recover_session(session: &Path, base: Option<Base>) -> Result<Recovered, String> {
    let segments = numbered(session, "journal-", ".log");
    let first = *segments.first().ok_or("the session has no journal")?;
    let head = segment_head(&session.join(segment_name(first)))
        .ok_or("the journal header is unreadable")?;
    let project = head
        .get("project")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let session_base = head
        .get("base")
        .and_then(Value::as_str)
        .ok_or("the journal has no base")?
        .to_owned();
    // The newest checkpoint that verifies.
    let mut start = None;
    for n in numbered(session, "checkpoint-", ".json").into_iter().rev() {
        if n < first {
            continue;
        }
        if let Some(loaded) = read_checkpoint(
            &session.join(checkpoint_name(n)),
            base.as_ref().map(|b| b.document),
        )? {
            start = Some(((n, 0), loaded));
            break;
        }
    }
    // The lines from the segment recovery starts at; older segments (kept
    // until the checkpoint's prune) are behind it.
    let from = start.as_ref().map_or(first, |((n, _), _)| *n);
    let mut lines = Vec::new();
    for &n in segments.iter().filter(|&&n| n >= from) {
        let segment = segment_lines(&session.join(segment_name(n)))?;
        let torn = segment.last().is_some_and(Result::is_err);
        lines.push((n, segment));
        if torn {
            break;
        }
    }
    let ((from, skip), loaded) = match start {
        Some(found) => found,
        None if let Some(earlier) = continued(session) => {
            // Checkpoint 0 never landed: the session this one superseded
            // still holds its start state.
            let earlier = recover_session(&earlier, base)?;
            let revision = head.get("revision").and_then(Value::as_f64).unwrap_or(0.0) as u64;
            if earlier.truncated || earlier.revision != revision {
                return Err("the session's first checkpoint never landed".into());
            }
            (
                (first, 0),
                Loaded {
                    revision,
                    base: earlier.base,
                    bases: {
                        let mut bases = earlier.bases;
                        let start = Ancestor::start(&session_base);
                        if !bases.contains(&start) {
                            bases.push(start);
                        }
                        bases
                    },
                    document: earlier.document.root().cloned(),
                    history: LoadedHistory::Ready(earlier.history),
                    workspace: earlier.workspace,
                },
            )
        }
        None => {
            let identity = history_identity(base);
            let base = base.ok_or("no checkpoint was written and no base was given")?;
            let hash = base.document.canonical_hash().unwrap_or_default();
            let mut bases = bases_of(&head, &session_base);
            // The base may be a save this session published while edits
            // went on (or that a Quit Now left without a checkpoint): replay
            // what followed its save point, if the base holds the history
            // that save wrote too (another writer's same document with its
            // own history is not this session's save).
            let mut point = None;
            if hash != session_base {
                for (n, segment) in &lines {
                    for (i, value) in segment.iter().enumerate() {
                        let Some((ancestor, revision)) = value.as_ref().ok().and_then(save_point)
                        else {
                            continue;
                        };
                        let Some(ancestor) = ancestor else { continue };
                        if ancestor.is(&hash, identity.as_deref()) {
                            point = Some(((*n, i + 1), revision));
                        }
                        bases.push(ancestor);
                    }
                }
            }
            let (at, revision) = match point {
                Some(found) => found,
                // Only the session's first segment can start from the base.
                None if first != 0 => {
                    return Err("no checkpoint covers the oldest journal segment".into());
                }
                None if hash != session_base => {
                    return Err("the given document is not this journal's base".into());
                }
                None => (
                    (0, 0),
                    head.get("revision").and_then(Value::as_f64).unwrap_or(1.0) as u64,
                ),
            };
            (
                at,
                Loaded {
                    revision,
                    base: hash,
                    bases,
                    document: base.document.root().cloned(),
                    history: LoadedHistory::Ready(base.history.cloned().unwrap_or_default()),
                    workspace: base.workspace.cloned().unwrap_or(Value::Null),
                },
            )
        }
    };
    let open_comp = loaded
        .document
        .as_ref()
        .and_then(|d| d.get("compId"))
        .and_then(|id| match id.kind() {
            pm_model::NodeKind::String(s) => s.as_str().map(str::to_owned),
            _ => None,
        });
    let history = match loaded.history {
        LoadedHistory::Ready(history) => history,
        LoadedHistory::Saved(saved) if saved.is_null() => History::new(),
        LoadedHistory::Saved(saved) => History::from_saved(&saved, open_comp.as_deref())
            .ok_or("the checkpoint's history is invalid")?,
    };
    let mut r = Replay {
        document: Document::from_root(loaded.document, loaded.revision),
        history,
        revision: loaded.revision,
        workspace: loaded.workspace,
        replayed: 0,
    };
    let mut bases = loaded.bases;
    let mut truncated = false;
    for (n, segment) in lines.iter().filter(|(n, _)| *n >= from) {
        let skip = if *n == from { skip } else { 0 };
        for value in segment.iter().skip(skip) {
            let Ok(value) = value else {
                // The tail of an interrupted write.
                truncated = true;
                break;
            };
            if let Some((ancestor, _)) = save_point(value) {
                if let Some(ancestor) = ancestor
                    && !bases.contains(&ancestor)
                {
                    bases.push(ancestor);
                }
                continue;
            }
            if let Err(e) = r.apply(value) {
                return Err(format!("journal replay failed: {e}"));
            }
        }
    }
    Ok(Recovered {
        project,
        base: loaded.base,
        bases,
        revision: r.revision,
        document: r.document,
        history: r.history,
        workspace: r.workspace,
        replayed: r.replayed,
        truncated,
        session: session.to_path_buf(),
    })
}

struct Replay {
    document: Document,
    history: History,
    revision: u64,
    workspace: Value,
    replayed: usize,
}

fn decode_ops(v: Option<&Value>) -> Result<Vec<Op>, String> {
    ops_from_value(v.ok_or("missing operations")?).map_err(|e| e.0.to_string())
}

impl Replay {
    fn push(&mut self, m: &Map) -> Result<(), String> {
        let saved = match m.get("s").and_then(Value::as_array) {
            Some(pair) if pair.len() == 2 => Some((
                patches_from_value(&pair[0]).map_err(|e| e.0.to_string())?,
                patches_from_value(&pair[1]).map_err(|e| e.0.to_string())?,
            )),
            _ => None,
        };
        self.history.push(
            m.get("label")
                .and_then(Value::as_str)
                .unwrap_or("Edit")
                .to_owned(),
            origin_from(m.get("origin").and_then(Value::as_str)),
            decode_ops(m.get("f"))?,
            decode_ops(m.get("i"))?,
            saved,
            self.revision,
        );
        Ok(())
    }

    fn apply(&mut self, value: &Value) -> Result<(), String> {
        let m = value.as_object().ok_or("a journal line is not an object")?;
        self.replayed += 1;
        match m.get("t").and_then(Value::as_str) {
            Some("rev") => {
                let base = m.get("b").and_then(Value::as_f64).ok_or("no base")? as u64;
                let revision = m.get("r").and_then(Value::as_f64).ok_or("no revision")? as u64;
                if base != self.revision {
                    return Err(format!(
                        "revision gap: at {}, record on {base}",
                        self.revision
                    ));
                }
                let ops = decode_ops(m.get("ops"))?;
                self.document
                    .apply_ops_at(&ops, revision)
                    .map_err(|e| e.to_string())?;
                self.revision = revision;
            }
            Some("metadata") => {
                let ops = decode_ops(m.get("ops"))?;
                if ops.iter().any(|op| !matches!(op, Op::Leaf(_)) || op.path().len()!=1 || !matches!(op.path().first(),Some(pm_model::patch::PathPart::Key(key)) if key.as_str().is_some_and(|key|crate::memory_metadata::ALL_FIELDS.contains(&key)))) {return Err("Invalid metadata journal path".into());}
                self.document = crate::memory_metadata::apply(&self.document, &ops)?;
            }
            Some("push") => self.push(m)?,
            // Closure entries die with their realm (not persisted, as before).
            Some("ext") => {}
            Some("undo") => {
                if m.get("x") != Some(&Value::Bool(true)) && self.history.can_undo() {
                    self.history.mark_undone();
                }
            }
            Some("redo") => {
                if m.get("x") != Some(&Value::Bool(true)) && self.history.can_redo() {
                    self.history.mark_redone();
                }
            }
            Some("squash") => {
                let n = m.get("n").and_then(Value::as_f64).unwrap_or(0.0) as usize;
                self.history.pop_top(n);
                self.push(m)?;
            }
            Some("clear") => self.history.clear(),
            Some("ws") => self.workspace = m.get("v").cloned().unwrap_or(Value::Null),
            other => return Err(format!("unknown journal record {other:?}")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod durability_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn background_fsync_completes_within_200ms_even_with_an_oversized_request() {
        // Observe the real background worker, without a barrier that would mask
        // a missing periodic flush. This is process/filesystem evidence only;
        // physical power-loss and F_FULLFSYNC verification require macOS.
        let path = std::env::temp_dir().join(format!("pm-fsync-{}", crate::new_id("probe")));
        let file = File::create(&path).unwrap();
        let d = Durability::new(JournalConfig {
            flush_interval: Duration::from_secs(60),
            ..Default::default()
        });
        let target = d.register(&file).unwrap();
        for _ in 0..4 {
            (&file).write_all(b"synthetic\n").unwrap();
            let start = Instant::now();
            target.dirty.store(true, Ordering::Release);
            while target.dirty.load(Ordering::Acquire) {
                assert!(
                    start.elapsed() < Duration::from_millis(200),
                    "periodic fsync missed 200 ms"
                );
                thread::sleep(Duration::from_millis(1));
            }
            // dirty is claimed before fsync: wait for its actual completion.
            let _completed = target.syncing.lock().unwrap();
            assert!(
                start.elapsed() <= Duration::from_millis(200),
                "fsync completion missed 200 ms"
            );
            assert!(d.take_errors().is_empty());
        }
        drop(d);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn barrier_waits_for_a_flush_that_already_claimed_dirty() {
        let d = Arc::new(Durability::new(JournalConfig::default()));
        let target = d.register(&File::open("/dev/null").unwrap()).unwrap();
        // Model the flusher between dirty.swap(false) and fsync completion.
        let flushing = target.syncing.lock().unwrap();
        assert!(!target.dirty.load(Ordering::Acquire));
        let (started, start) = mpsc::channel();
        let (done, finish) = mpsc::channel();
        let worker = {
            let d = d.clone();
            thread::spawn(move || {
                started.send(()).unwrap();
                done.send(d.barrier()).unwrap();
            })
        };
        start.recv().unwrap();
        assert!(matches!(
            finish.recv_timeout(Duration::from_millis(30)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(flushing);
        finish
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn failed_fsync_keeps_the_target_dirty_for_retry() {
        use std::os::{fd::OwnedFd, unix::net::UnixStream};
        let (socket, _peer) = UnixStream::pair().unwrap();
        let target = SyncTarget {
            file: File::from(OwnedFd::from(socket)),
            dirty: AtomicBool::new(true),
            syncing: Mutex::new(()),
        };
        assert!(target.sync().is_err(), "a socket cannot be fsynced");
        assert!(target.dirty.load(Ordering::Acquire));
    }

    #[test]
    fn barrier_reports_checkpoint_failure() {
        let d = Durability::new(JournalConfig::default());
        d.submit(Job::Checkpoint {
            // A file cannot be a checkpoint directory.
            dir: PathBuf::from("/dev/null"),
            project: "synthetic".into(),
            base: "base".into(),
            segment: 0,
            state: Box::new(CheckpointState {
                revision: 1,
                document: Arc::new(Document::new(None, 1)),
                history: History::new().snapshot(),
                workspace: Value::Null,
            }),
            bases: vec![],
            supersedes: vec![],
            measured: Default::default(),
        });
        let error = d.barrier().unwrap_err();
        assert!(error.to_string().contains("checkpoint 0"), "{error}");
    }
}
