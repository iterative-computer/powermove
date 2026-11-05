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
//!   the newer checkpoint is durable, so recovery always has a chain.
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
//! supersedes them once its own first checkpoint is durable.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl Default for JournalConfig {
    fn default() -> Self {
        JournalConfig {
            flush_interval: Duration::from_millis(200),
            checkpoint_revisions: 50,
            max_segment_bytes: 4 * 1024 * 1024,
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
        /// Sessions this one replaces once its checkpoint is durable.
        supersedes: Vec<PathBuf>,
    },
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
                                supersedes,
                            } => {
                                if let Err(e) =
                                    write_checkpoint(&dir, &project, &base, segment, &state)
                                {
                                    shared
                                        .errors
                                        .lock()
                                        .expect("errors")
                                        .push_back(format!("checkpoint {segment}: {e}"));
                                    continue;
                                }
                                prune_before(&dir, segment);
                                for old in supersedes {
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
    dir: PathBuf,
    project: String,
    base: String,
    segment: u64,
    file: File,
    target: Arc<SyncTarget>,
    bytes: u64,
    revisions: u64,
}

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
    pub fn create(
        durability: &Durability,
        project_dir: &Path,
        project: &str,
        base: &str,
        initial: CheckpointState,
        supersedes: Vec<PathBuf>,
    ) -> io::Result<Journal> {
        let dir = project_dir.join(session_name());
        fs::create_dir_all(&dir)?;
        sync_dir(project_dir)?;
        let (file, target) = open_segment(durability, &dir, project, base, 0, initial.revision)?;
        durability.submit(Job::Checkpoint {
            dir: dir.clone(),
            project: project.to_owned(),
            base: base.to_owned(),
            segment: 0,
            state: Box::new(initial),
            supersedes,
        });
        Ok(Journal {
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
        if self.revisions >= config.checkpoint_revisions || self.bytes >= config.max_segment_bytes {
            self.rotate(durability, state())?;
        }
        Ok(())
    }

    fn rotate(&mut self, durability: &Durability, state: CheckpointState) -> io::Result<()> {
        let next = self.segment + 1;
        let (file, target) = open_segment(
            durability,
            &self.dir,
            &self.project,
            &self.base,
            next,
            state.revision,
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
            supersedes: Vec::new(),
        });
        Ok(())
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

fn open_segment(
    durability: &Durability,
    dir: &Path,
    project: &str,
    base: &str,
    segment: u64,
    revision: u64,
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
    head.insert("version", Value::Number(1.0));
    head.insert("project", Value::from(project));
    head.insert("base", Value::from(base));
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
    segment: u64,
    state: &CheckpointState,
) -> io::Result<()> {
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
    meta.insert("version", Value::Number(1.0));
    meta.insert("project", Value::from(project));
    meta.insert("base", Value::from(base));
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
    drop(file);
    fs::remove_file(&doc_tmp)?;
    fs::rename(&tmp, &final_path)?;
    sync_dir(dir)
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
    document: Value,
    history: Value,
    workspace: Value,
}

fn read_checkpoint(path: &Path) -> Option<Loaded> {
    let mut text = String::new();
    File::open(path).ok()?.read_to_string(&mut text).ok()?;
    let (meta, doc) = text.split_once('\n')?;
    let meta = parse(meta).ok()?;
    let m = meta.as_object()?;
    if m.get("format")?.as_str()? != CHECKPOINT_FORMAT {
        return None;
    }
    if pm_model::hash::sha256_hex(doc.as_bytes()) != m.get("documentHash")?.as_str()? {
        return None;
    }
    Some(Loaded {
        revision: m.get("revision")?.as_f64()? as u64,
        base: m.get("base")?.as_str()?.to_owned(),
        document: parse(doc).ok()?,
        history: m.get("history")?.clone(),
        workspace: m.get("workspace").cloned().unwrap_or(Value::Null),
    })
}

// ── recovery ────────────────────────────────────────────────────────────────

/// Recovered state.
#[derive(Debug)]
pub struct Recovered {
    pub project: String,
    /// The session's base hash.
    pub base: String,
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
    let Some(session) = sessions(project_dir).pop() else {
        return Recovery::Nothing;
    };
    match recover_session(&session, base_document) {
        Err(reason) => Recovery::Unrecoverable { session, reason },
        Ok(recovered) => match current_base {
            Some(current) if current != recovered.base => Recovery::Copy {
                recovered,
                current_base: current.to_owned(),
            },
            _ => Recovery::Recovered(recovered),
        },
    }
}

fn segment_head(path: &Path) -> Option<Map> {
    let mut text = String::new();
    File::open(path).ok()?.read_to_string(&mut text).ok()?;
    let first = text.lines().next()?;
    let (sum, json) = first.split_once(' ')?;
    (checksum(json) == sum).then_some(())?;
    parse(json).ok()?.as_object().cloned()
}

fn recover_session(session: &Path, base_document: Option<&Value>) -> Result<Recovered, String> {
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
        if let Some(loaded) = read_checkpoint(&session.join(checkpoint_name(n))) {
            start = Some((n, loaded));
            break;
        }
    }
    let (from, loaded) = match start {
        Some(found) => found,
        None => {
            // Only the session's first segment can start from the base.
            if first != 0 {
                return Err("no checkpoint covers the oldest journal segment".into());
            }
            let doc = base_document.ok_or("no checkpoint was written and no base was given")?;
            if pm_model::hash::canonical_hash(doc) != session_base {
                return Err("the given document is not this journal's base".into());
            }
            let revision = head.get("revision").and_then(Value::as_f64).unwrap_or(1.0) as u64;
            (
                0,
                Loaded {
                    revision,
                    base: session_base.clone(),
                    document: doc.clone(),
                    history: Value::Null,
                    workspace: Value::Null,
                },
            )
        }
    };
    let open_comp = loaded
        .document
        .get("compId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let history = if loaded.history.is_null() {
        History::new()
    } else {
        History::from_saved(&loaded.history, open_comp.as_deref())
            .ok_or("the checkpoint's history is invalid")?
    };
    let mut r = Replay {
        document: Document::new(Some(loaded.document), loaded.revision),
        history,
        revision: loaded.revision,
        workspace: loaded.workspace,
        replayed: 0,
    };
    let mut truncated = false;
    'segments: for n in segments.into_iter().filter(|&n| n >= from) {
        let mut text = Vec::new();
        File::open(session.join(segment_name(n)))
            .and_then(|mut f| f.read_to_end(&mut text))
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&text);
        let mut lines = text.split_inclusive('\n');
        let _head = lines.next();
        for raw in lines {
            let Some(body) = raw.strip_suffix('\n') else {
                truncated = true; // torn final line
                break 'segments;
            };
            let Some((sum, json)) = body.split_once(' ') else {
                truncated = true;
                break 'segments;
            };
            if checksum(json) != sum {
                truncated = true;
                break 'segments;
            }
            let Ok(value) = parse(json) else {
                truncated = true;
                break 'segments;
            };
            if let Err(e) = r.apply(&value) {
                return Err(format!("journal replay failed: {e}"));
            }
        }
    }
    Ok(Recovered {
        project,
        base: loaded.base,
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
            supersedes: vec![],
        });
        let error = d.barrier().unwrap_err();
        assert!(error.to_string().contains("checkpoint 0"), "{error}");
    }
}
