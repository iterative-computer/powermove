//! Owner-fenced native document kernels and one bounded ingestion worker.
use std::{
    collections::BTreeMap,
    io::{BufWriter, Read, Write},
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::JoinHandle,
};

use pm_model::{Document, NodeKind, Value};
use pm_protocol::{
    common::{DocumentId, SessionId},
    error::EngineProcess,
};
use pm_wasm::kernel::{Kernel, PreparedDocument, integer, member, text};

const MAX_OWNERS: usize = 32;
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PERSIST_BYTES: usize = 512 * 1024 * 1024;
// One owner teardown can retire a prepared load, refresh, persisted snapshot,
// checkpoint and live kernel while the reader's channel is occupied.
const MAX_DEFERRED_RETIREMENTS: usize = 5;
const MAX_CHECKPOINT_BATCHES: usize = 128;
const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;
const READER_BUSY: &str = "The native document reader is busy; retry the load";
const RETIREMENT_BUSY: &str =
    "The native document reader is retiring an earlier tree; retry the load";
const REFRESH_STALE: &str = "The native document changed during checkpoint refresh";
const PERSIST_STALE: &str = "The native document changed during checkpoint persistence";

#[derive(Default)]
struct ReaderMetrics {
    owner: AtomicUsize,
    input: AtomicUsize,
    input_peak: AtomicUsize,
    retired: AtomicUsize,
    retired_on_reader: AtomicUsize,
    retired_inline: AtomicUsize,
}
impl ReaderMetrics {
    fn input(&self, bytes: usize) {
        self.input.store(bytes, Ordering::Relaxed);
        self.input_peak.fetch_max(bytes, Ordering::Relaxed);
    }
}
struct Reading(Arc<ReaderMetrics>);
impl Drop for Reading {
    fn drop(&mut self) {
        self.0.input(0);
        self.0.owner.store(0, Ordering::Relaxed);
    }
}

enum RetiredValue {
    Prepared(PreparedDocument),
    Persisted(Persisted),
    Document(pm_model::Document, usize),
    Kernel(Box<Kernel>),
}
impl RetiredValue {
    fn bytes(&self) -> usize {
        match self {
            Self::Prepared(p) => p.tree_bytes_estimate.max(p.source_bytes),
            Self::Persisted(p) => p.prepared.tree_bytes_estimate.max(p.prepared.source_bytes),
            Self::Document(_, bytes) => *bytes,
            Self::Kernel(k) => {
                k.tree_bytes_estimate.max(k.source_bytes) + std::mem::size_of::<Kernel>()
            }
        }
    }
}
struct Retired {
    value: Option<RetiredValue>,
    bytes: usize,
    metrics: Arc<ReaderMetrics>,
}
impl Drop for Retired {
    fn drop(&mut self) {
        match self.value.take() {
            Some(RetiredValue::Prepared(prepared)) => drop(prepared),
            Some(RetiredValue::Persisted(persisted)) => drop(persisted),
            Some(RetiredValue::Document(document, _)) => drop(document),
            Some(RetiredValue::Kernel(kernel)) => drop(kernel),
            None => {}
        }
        self.metrics
            .retired
            .fetch_sub(self.bytes, Ordering::Relaxed);
        let count = if std::thread::current().name() == Some("pm-native-kernel-reader") {
            &self.metrics.retired_on_reader
        } else {
            &self.metrics.retired_inline
        };
        count.fetch_add(1, Ordering::Relaxed);
    }
}
enum Job {
    Read(Work),
    Refresh(RefreshWork),
    Persist(PersistWork),
    Retire(Retired),
}

struct RefreshWork {
    owner: u32,
    generation: u64,
    document: Document,
    source_bytes: usize,
    cancelled: Arc<AtomicBool>,
}

struct PersistWork {
    owner: u32,
    generation: u64,
    path: String,
    document: Document,
    cancelled: Arc<AtomicBool>,
}
struct PersistFile {
    path: std::path::PathBuf,
    keep: bool,
}
impl Drop for PersistFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
struct Persisted {
    prepared: PreparedDocument,
    file: PersistFile,
    bytes: usize,
    chunk_peak: usize,
}
struct PersistCompleted {
    owner: u32,
    generation: u64,
    result: Result<Persisted, String>,
}
struct Persist {
    generation: u64,
    epoch: u64,
    document: DocumentId,
    revision: u64,
    root: RootIdentity,
    path: String,
    cancelled: Arc<AtomicBool>,
    completed: Option<Result<Persisted, String>>,
}
impl Persist {
    fn current(&self, slot: &Slot) -> bool {
        !self.cancelled.load(Ordering::Relaxed)
            && self.epoch == slot.document_epoch
            && slot.document.as_ref() == Some(&self.document)
            && slot
                .kernel
                .as_ref()
                .is_some_and(|kernel| kernel.engine.revision() >= self.revision)
    }
    fn receipt(&self) -> Result<Option<String>, String> {
        let Some(result) = self.completed.as_ref() else {
            return Ok(None);
        };
        let persisted = result.as_ref().map_err(Clone::clone)?;
        if persisted.prepared.document.revision() != self.revision
            || !self.root.matches(&persisted.prepared.document)
        {
            return Err(PERSIST_STALE.into());
        }
        Ok(Some(serde_json::json!({"pending":false,"revision":self.revision,"hash":persisted.prepared.hash,"integrity":persisted.prepared.integrity,"bytes":persisted.bytes,"writeChunkPeak":persisted.chunk_peak}).to_string()))
    }
}
impl Drop for Persist {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

// Weak identities fence container roots without pinning another full tree.
// Scalars also carry their node stamp; all document replacement paths cancel
// refreshes even if a legacy resync reuses the same revision.
enum RootIdentity {
    Missing,
    Array(Weak<Vec<pm_model::Node>>),
    Object(Weak<pm_model::json::ObjectMap<pm_model::Node>>),
    Scalar {
        revision: u64,
        kind: u8,
        bits: u64,
    },
    String {
        revision: u64,
        address: usize,
        len: usize,
    },
    SmallString {
        revision: u64,
        len: usize,
        bytes: [u8; 2 * std::mem::size_of::<usize>() - 1],
    },
}
impl RootIdentity {
    fn capture(document: &Document) -> Self {
        let Some(root) = document.root() else {
            return Self::Missing;
        };
        match root.kind() {
            NodeKind::Array(items) => Self::Array(Arc::downgrade(items)),
            NodeKind::Object(map) => Self::Object(Arc::downgrade(map)),
            NodeKind::String(text)
                if text.as_bytes().len() < 2 * std::mem::size_of::<usize>() =>
            {
                let mut bytes = [0; 2 * std::mem::size_of::<usize>() - 1];
                bytes[..text.as_bytes().len()].copy_from_slice(text.as_bytes());
                Self::SmallString {
                    revision: root.rev(),
                    len: text.as_bytes().len(),
                    bytes,
                }
            }
            NodeKind::String(text) => Self::String {
                revision: root.rev(),
                address: text.as_bytes().as_ptr() as usize,
                len: text.as_bytes().len(),
            },
            kind => Self::Scalar {
                revision: root.rev(),
                kind: match kind {
                    NodeKind::Null => 0,
                    NodeKind::Bool(_) => 1,
                    _ => 2,
                },
                bits: match kind {
                    NodeKind::Bool(value) => u64::from(*value),
                    NodeKind::Number(value) => value.to_bits(),
                    _ => 0,
                },
            },
        }
    }
    fn matches(&self, document: &Document) -> bool {
        match (self, Self::capture(document)) {
            (Self::Missing, Self::Missing) => true,
            (
                Self::SmallString {
                    revision,
                    len,
                    bytes,
                },
                Self::SmallString {
                    revision: now,
                    len: current_len,
                    bytes: current_bytes,
                },
            ) => *revision == now && *len == current_len && *bytes == current_bytes,
            (Self::Array(before), Self::Array(now)) => {
                before.ptr_eq(&now) && before.strong_count() > 0
            }
            (Self::Object(before), Self::Object(now)) => {
                before.ptr_eq(&now) && before.strong_count() > 0
            }
            (
                Self::Scalar {
                    revision,
                    kind,
                    bits,
                },
                Self::Scalar {
                    revision: now,
                    kind: current_kind,
                    bits: current_bits,
                },
            ) => *revision == now && *kind == current_kind && *bits == current_bits,
            (
                Self::String {
                    revision,
                    address,
                    len,
                },
                Self::String {
                    revision: now,
                    address: current_address,
                    len: current_len,
                },
            ) => *revision == now && *address == current_address && *len == current_len,
            _ => false,
        }
    }
}

struct Refresh {
    generation: u64,
    document: DocumentId,
    revision: u64,
    root: RootIdentity,
    cancelled: Arc<AtomicBool>,
    completed: Option<Result<PreparedDocument, String>>,
    cached: Option<(String, String)>,
}
impl Refresh {
    fn current(&self, slot: &Slot) -> bool {
        !self.cancelled.load(Ordering::Relaxed)
            && slot.document.as_ref() == Some(&self.document)
            && slot.kernel.as_ref().is_some_and(|kernel| {
                kernel.engine.revision() == self.revision && self.root.matches(kernel.document())
            })
    }
    fn receipt(&self) -> Result<Option<String>, String> {
        let digests = if let Some(result) = self.completed.as_ref() {
            let prepared = result.as_ref().map_err(Clone::clone)?;
            Some((prepared.hash.as_str(), prepared.integrity.as_str()))
        } else {
            self.cached
                .as_ref()
                .map(|(hash, integrity)| (hash.as_str(), integrity.as_str()))
        };
        Ok(digests.map(|(hash, integrity)| serde_json::json!({"pending":false,"revision":self.revision,"hash":hash,"integrity":integrity}).to_string()))
    }
}
impl Drop for Refresh {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

struct Work {
    owner: u32,
    generation: u64,
    path: String,
    revision: u64,
    /// The canonical SHA-256 of the tree, or, for `text`, of the file's bytes.
    hash: String,
    /// The file is JSON text in any member order (OPENWRITE-1 round): its
    /// bytes are checked against `hash` and the canonical hash is derived
    /// from the parsed tree, instead of requiring canonical bytes.
    text: bool,
    cancelled: Arc<AtomicBool>,
}
struct Completed {
    owner: u32,
    generation: u64,
    result: Result<PreparedDocument, String>,
}
struct Pending {
    generation: u64,
    document: DocumentId,
    cancelled: Arc<AtomicBool>,
    completed: Option<Result<PreparedDocument, String>>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Slot {
    kernel: Option<Kernel>,
    document: Option<DocumentId>,
    pending: Option<Pending>,
    refresh: Option<Refresh>,
    persist: Option<Persist>,
    document_epoch: u64,
    checkpoint_batches: Option<usize>,
    checkpoint_bytes: usize,
}

struct Reader {
    input: Option<SyncSender<Job>>,
    output: Receiver<Completed>,
    persisted: Receiver<PersistCompleted>,
    thread: Option<JoinHandle<()>>,
}
impl Reader {
    fn new(process: EngineProcess, metrics: Arc<ReaderMetrics>) -> Result<Self, String> {
        let (input, jobs) = mpsc::sync_channel::<Job>(2);
        // Every call drains results before enqueueing. At most the one active
        // job and two queued jobs can complete between calls, bounding replies
        // without making shutdown wait on a blocked result sender.
        let (answers, output) = mpsc::channel();
        let (persist_answers, persisted) = mpsc::channel();
        let thread =
            pm_engine::qos::spawn_engine_thread(process, "pm-native-kernel-reader", move || {
                while let Ok(job) = jobs.recv() {
                    let work = match job {
                        Job::Read(work) => work,
                        Job::Persist(work) => {
                            if work.cancelled.load(Ordering::Relaxed) {
                                continue;
                            }
                            metrics.owner.store(work.owner as usize, Ordering::Relaxed);
                            let _reading = Reading(metrics.clone());
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    persist_to_file(&work.path, work.document, &work.cancelled)
                                }))
                                .unwrap_or_else(|_| {
                                    Err("The native document checkpoint writer failed".into())
                                });
                            if work.cancelled.load(Ordering::Relaxed) {
                                continue;
                            }
                            if persist_answers
                                .send(PersistCompleted {
                                    owner: work.owner,
                                    generation: work.generation,
                                    result,
                                })
                                .is_err()
                            {
                                break;
                            }
                            continue;
                        }
                        Job::Refresh(work) => {
                            if work.cancelled.load(Ordering::Relaxed) {
                                continue;
                            }
                            metrics.owner.store(work.owner as usize, Ordering::Relaxed);
                            let _reading = Reading(metrics.clone());
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    PreparedDocument::from_document(
                                        work.document,
                                        work.source_bytes,
                                    )
                                }))
                                .map_err(|_| {
                                    "The native document checkpoint refresh failed".to_owned()
                                });
                            if work.cancelled.load(Ordering::Relaxed) {
                                continue;
                            }
                            if answers
                                .send(Completed {
                                    owner: work.owner,
                                    generation: work.generation,
                                    result,
                                })
                                .is_err()
                            {
                                break;
                            }
                            continue;
                        }
                        Job::Retire(retired) => {
                            drop(retired);
                            continue;
                        }
                    };
                    if work.cancelled.load(Ordering::Relaxed) {
                        continue;
                    }
                    metrics.owner.store(work.owner as usize, Ordering::Relaxed);
                    let _reading = Reading(metrics.clone());
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        read(&work, &metrics)
                    }))
                    .unwrap_or_else(|_| Err("The native document reader failed".into()));
                    if work.cancelled.load(Ordering::Relaxed) {
                        continue;
                    }
                    if answers
                        .send(Completed {
                            owner: work.owner,
                            generation: work.generation,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|e| format!("The native document reader could not start ({})", e.kind()))?;
        Ok(Self {
            input: Some(input),
            output,
            persisted,
            thread: Some(thread),
        })
    }
    fn submit(&self, work: Work) -> Result<(), String> {
        self.submit_job(Job::Read(work))
    }
    fn submit_job(&self, job: Job) -> Result<(), String> {
        self.input
            .as_ref()
            .ok_or("The native document reader stopped")?
            .try_send(job)
            .map_err(|e| match e {
                TrySendError::Full(_) => READER_BUSY.into(),
                TrySendError::Disconnected(_) => "The native document reader stopped".into(),
            })
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.input.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct CanonicalFileWriter<'a> {
    output: BufWriter<std::fs::File>,
    cancelled: &'a AtomicBool,
    error: Option<String>,
    bytes: usize,
    chunk_peak: usize,
}
impl pm_model::json::write::Sink for CanonicalFileWriter<'_> {
    fn push_str(&mut self, text: &str) {
        if self.error.is_some() {
            return;
        }
        if self.cancelled.load(Ordering::Relaxed) {
            self.error = Some("The native checkpoint persistence was cancelled".into());
            return;
        }
        if self.bytes.saturating_add(text.len()) > MAX_PERSIST_BYTES {
            self.error = Some("The native checkpoint exceeds 512 MiB".into());
            return;
        }
        self.chunk_peak = self.chunk_peak.max(text.len());
        if let Err(error) = self.output.write_all(text.as_bytes()) {
            self.error = Some(format!(
                "The native checkpoint could not be written ({})",
                error.kind()
            ));
        } else {
            self.bytes += text.len();
        }
    }
}

fn persist_to_file(
    path: &str,
    document: Document,
    cancelled: &AtomicBool,
) -> Result<Persisted, String> {
    let root = document
        .root()
        .ok_or("An undefined native document has no canonical checkpoint")?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("The native checkpoint persistence was cancelled".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|error| {
        format!(
            "The native checkpoint could not be created ({})",
            error.kind()
        )
    })?;
    // The guard is created only after create_new succeeds, so an existing
    // checkpoint is never removed on an unsuccessful request.
    let artifact = PersistFile {
        path: path.into(),
        keep: false,
    };
    let mut sink = CanonicalFileWriter {
        output: BufWriter::with_capacity(64 * 1024, file),
        cancelled,
        error: None,
        bytes: 0,
        chunk_peak: 0,
    };
    pm_model::json::write::write_canonical(root, &mut sink);
    if let Some(error) = sink.error.take() {
        return Err(error);
    }
    sink.output.flush().map_err(|error| {
        format!(
            "The native checkpoint could not be flushed ({})",
            error.kind()
        )
    })?;
    sink.output.get_ref().sync_all().map_err(|error| {
        format!(
            "The native checkpoint could not be synced ({})",
            error.kind()
        )
    })?;
    let bytes = sink.bytes;
    let chunk_peak = sink.chunk_peak;
    drop(sink);
    if cancelled.load(Ordering::Relaxed) {
        return Err("The native checkpoint persistence was cancelled".into());
    }
    let prepared = PreparedDocument::from_document(document, bytes);
    if cancelled.load(Ordering::Relaxed) {
        return Err("The native checkpoint persistence was cancelled".into());
    }
    Ok(Persisted {
        prepared,
        file: artifact,
        bytes,
        chunk_peak,
    })
}

fn read(work: &Work, metrics: &ReaderMetrics) -> Result<PreparedDocument, String> {
    let mut file = std::fs::File::open(&work.path).map_err(|e| {
        format!(
            "The native document file could not be opened ({})",
            e.kind()
        )
    })?;
    let metadata = file.metadata().map_err(|e| {
        format!(
            "The native document file could not be inspected ({})",
            e.kind()
        )
    })?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err("The native document cache file is invalid or exceeds 1 GiB".into());
    }
    // Main owns this private path. Bound reads even if a file changes after
    // metadata validation, and notice superseded work between I/O chunks.
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(metadata.len() as usize)
        .map_err(|_| "The native document file exceeds available memory")?;
    let mut chunk = vec![0; 1024 * 1024];
    metrics.input(bytes.capacity() + chunk.capacity());
    loop {
        if work.cancelled.load(Ordering::Relaxed) {
            return Err("The native document load was cancelled".into());
        }
        let count = file
            .read(&mut chunk)
            .map_err(|e| format!("The native document file could not be read ({})", e.kind()))?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) as u64 > MAX_FILE_BYTES {
            return Err("The native document cache file exceeds 1 GiB".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        metrics.input(bytes.capacity() + chunk.capacity());
    }
    drop(chunk);
    metrics.input(bytes.capacity());
    // The stream's checksum and the parse each read every byte: side by side.
    let (sha256, document) = pm_wasm::kernel::join(
        || work.text.then(|| pm_model::hash::sha256_hex(&bytes)),
        || PreparedDocument::parse_tree(&bytes, work.revision),
    );
    if sha256.is_some_and(|sha256| sha256 != work.hash) {
        return Err("The native document file does not match the hash of its stream".into());
    }
    let document = document?;
    let source_bytes = bytes.len();
    drop(bytes);
    metrics.input(0);
    let prepared = PreparedDocument::from_document(document, source_bytes);
    if !work.text && prepared.hash != work.hash {
        return Err("The native document does not match its canonical hash".into());
    }
    Ok(prepared)
}

pub struct NativeKernels {
    slots: BTreeMap<u32, Slot>,
    reader: Option<Reader>,
    process: EngineProcess,
    generation: u64,
    reply_special: bool,
    metrics: Arc<ReaderMetrics>,
    retired: Vec<Retired>,
}
impl NativeKernels {
    pub fn new(process: EngineProcess) -> Self {
        Self {
            slots: BTreeMap::new(),
            reader: None,
            process,
            generation: 0,
            reply_special: false,
            metrics: Arc::new(ReaderMetrics::default()),
            retired: Vec::new(),
        }
    }

    fn flush_retired(&mut self) {
        let Some(sender) = self.reader.as_ref().and_then(|r| r.input.as_ref()) else {
            return;
        };
        while let Some(retired) = self.retired.pop() {
            match sender.try_send(Job::Retire(retired)) {
                Ok(()) => {}
                Err(TrySendError::Full(Job::Retire(retired))) => {
                    self.retired.push(retired);
                    break;
                }
                Err(_) => break,
            }
        }
    }

    fn retire(&mut self, value: RetiredValue) {
        if self.reader.is_none() {
            self.reader = Reader::new(self.process, self.metrics.clone()).ok();
        }
        let bytes = value.bytes();
        self.metrics.retired.fetch_add(bytes, Ordering::Relaxed);
        let retired = Retired {
            value: Some(value),
            bytes,
            metrics: self.metrics.clone(),
        };
        self.flush_retired();
        if let Some(sender) = self.reader.as_ref().and_then(|r| r.input.as_ref()) {
            match sender.try_send(Job::Retire(retired)) {
                Ok(()) => {}
                Err(TrySendError::Full(Job::Retire(retired))) => {
                    if self.retired.len() < MAX_DEFERRED_RETIREMENTS {
                        self.retired.push(retired);
                    }
                    // Bound memory even under repeated legacy synchronous
                    // replacements while ingestion occupies the reader. The
                    // overflow is freed here rather than accumulating trees.
                }
                Err(_) => {}
            }
        }
    }

    fn cancel_pending(&mut self, mut pending: Pending) {
        pending.cancelled.store(true, Ordering::Relaxed);
        if let Some(Ok(prepared)) = pending.completed.take() {
            self.retire(RetiredValue::Prepared(prepared));
        }
    }

    fn cancel_refresh(&mut self, mut refresh: Refresh) {
        refresh.cancelled.store(true, Ordering::Relaxed);
        if let Some(Ok(prepared)) = refresh.completed.take() {
            self.retire(RetiredValue::Prepared(prepared));
        }
    }

    fn cancel_persist(&mut self, mut persist: Persist) {
        persist.cancelled.store(true, Ordering::Relaxed);
        if let Some(Ok(persisted)) = persist.completed.take() {
            self.retire(RetiredValue::Persisted(persisted));
        }
    }

    fn drain(&mut self) {
        loop {
            let completed = self.reader.as_ref().and_then(|r| r.output.try_recv().ok());
            let Some(completed) = completed else {
                break;
            };
            if let Some(pending) = self
                .slots
                .get_mut(&completed.owner)
                .and_then(|s| s.pending.as_mut())
                && pending.generation == completed.generation
                && !pending.cancelled.load(Ordering::Relaxed)
            {
                let old = pending.completed.replace(completed.result);
                if let Some(Ok(prepared)) = old {
                    self.retire(RetiredValue::Prepared(prepared));
                }
            } else if let Some(refresh) = self
                .slots
                .get_mut(&completed.owner)
                .and_then(|s| s.refresh.as_mut())
                && refresh.generation == completed.generation
                && !refresh.cancelled.load(Ordering::Relaxed)
            {
                let old = refresh.completed.replace(completed.result);
                if let Some(Ok(prepared)) = old {
                    self.retire(RetiredValue::Prepared(prepared));
                }
            } else if let Ok(prepared) = completed.result {
                self.retire(RetiredValue::Prepared(prepared));
            }
        }
        loop {
            let completed = self
                .reader
                .as_ref()
                .and_then(|reader| reader.persisted.try_recv().ok());
            let Some(completed) = completed else {
                break;
            };
            if let Some(persist) = self
                .slots
                .get_mut(&completed.owner)
                .and_then(|slot| slot.persist.as_mut())
                && persist.generation == completed.generation
                && !persist.cancelled.load(Ordering::Relaxed)
            {
                let old = persist.completed.replace(completed.result);
                if let Some(Ok(persisted)) = old {
                    self.retire(RetiredValue::Persisted(persisted));
                }
            } else if let Ok(persisted) = completed.result {
                self.retire(RetiredValue::Persisted(persisted));
            }
        }
        self.flush_retired();
    }

    /// The only entry point used by Node. Main supplies `owner`; it is never
    /// read from the untrusted renderer's JSON request.
    pub fn call(&mut self, engine: &mut pm_engine::Engine, owner: u32, request: &str) -> String {
        self.call_bytes(engine, owner, request, None)
    }

    pub fn call_bytes(
        &mut self,
        engine: &mut pm_engine::Engine,
        owner: u32,
        request: &str,
        data: Option<&[u8]>,
    ) -> String {
        self.drain();
        self.reply_special = false;
        let result = self.dispatch(engine, owner, request, data);
        match result {
            Ok(value) => {
                let special = if self.reply_special {
                    ",\"special\":true"
                } else {
                    ""
                };
                format!("{{\"ok\":true,\"value\":{value}{special}}}")
            }
            Err(error) => {
                let mut reply = pm_wasm::reply::Reply::new();
                reply.raw(if matches!(error.as_str(), READER_BUSY | RETIREMENT_BUSY) {
                    "{\"ok\":false,\"code\":\"busy\",\"error\":"
                } else if matches!(error.as_str(), REFRESH_STALE | PERSIST_STALE) {
                    "{\"ok\":false,\"code\":\"stale\",\"error\":"
                } else {
                    "{\"ok\":false,\"error\":"
                });
                reply.str(&error);
                reply.raw("}");
                reply.text
            }
        }
    }

    fn register(engine: &mut pm_engine::Engine, owner: u32, slot: &Slot, checkpoint: bool) {
        if let (Some(kernel), Some(document)) = (&slot.kernel, &slot.document) {
            let mirror = kernel.document().clone();
            let hash = kernel.cached_hash().map(str::to_owned);
            if checkpoint {
                engine.register_document_kernel(owner, document.clone(), mirror, hash);
            } else {
                engine.update_document_kernel(owner, document.clone(), mirror, hash);
            }
        }
    }

    fn dispatch(
        &mut self,
        engine: &mut pm_engine::Engine,
        owner: u32,
        request: &str,
        data: Option<&[u8]>,
    ) -> Result<String, String> {
        if owner == 0 {
            return Err("The document kernel owner must be nonzero".into());
        }
        let request = pm_model::json::parse(request)
            .map_err(|e| format!("Invalid document kernel request: {e}"))?;
        let op = text(&request, "op")?;
        let args = member(&request, "args").unwrap_or(&Value::Null);
        if !matches!(args, Value::Null | Value::Object(_)) {
            return Err("args must be an object".into());
        }
        if op == "close" {
            let checkpoint = engine.document_kernel_checkpoint(owner);
            let slot = self.slots.remove(&owner);
            // Keep the kernel alive until the registry drops its old clone,
            // then retire the kernel's tree reference on the worker. Viewer
            // sessions may still retain their own shared reference.
            engine.unregister_document_kernel(owner);
            if let Some(mut slot) = slot {
                if let Some(pending) = slot.pending.take() {
                    self.cancel_pending(pending);
                }
                if let Some(refresh) = slot.refresh.take() {
                    self.cancel_refresh(refresh);
                }
                if let Some(persist) = slot.persist.take() {
                    self.cancel_persist(persist);
                }
                if let Some(kernel) = slot.kernel.take() {
                    if let Some(checkpoint) = checkpoint {
                        self.retire(RetiredValue::Document(
                            checkpoint,
                            kernel.tree_bytes_estimate.max(kernel.source_bytes),
                        ));
                    }
                    self.retire(RetiredValue::Kernel(Box::new(kernel)));
                }
            }
            return Ok("null".into());
        }
        if op == "cancel" {
            if let Some(pending) = self.slots.get_mut(&owner).and_then(|s| s.pending.take()) {
                self.cancel_pending(pending);
            }
            return Ok("null".into());
        }
        if !self.slots.contains_key(&owner) && self.slots.len() >= MAX_OWNERS {
            return Err("The document kernel owner limit (32) was reached".into());
        }
        if op == "adoptSession" {
            let session = SessionId::new(integer(args, "session")?).map_err(|e| e.to_string())?;
            let document = DocumentId::new(text(args, "document")?);
            let revision = integer(args, "revision")?;
            let slot = self
                .slots
                .get(&owner)
                .ok_or("The document kernel owner is not open")?;
            if slot.document.as_ref() != Some(&document)
                || slot
                    .kernel
                    .as_ref()
                    .is_none_or(|kernel| kernel.engine.revision() != revision)
            {
                return Err("The native capture document or revision changed".into());
            }
            engine.adopt_document_kernel_session(owner, session, &document, revision)?;
            return Ok("null".into());
        }
        if op == "cancelPersist" {
            let revision = integer(args, "revision")?;
            if let Some(slot) = self.slots.get_mut(&owner) {
                if slot
                    .persist
                    .as_ref()
                    .is_some_and(|persist| persist.revision != revision)
                {
                    return Err(PERSIST_STALE.into());
                }
                if let Some(persist) = slot.persist.take() {
                    self.cancel_persist(persist);
                }
            }
            return Ok("null".into());
        }
        if op == "persist" {
            let revision = integer(args, "revision")?;
            let path = text(args, "path")?.to_owned();
            let slot = self.slots.get(&owner).ok_or(PERSIST_STALE)?;
            let kernel = slot.kernel.as_ref().ok_or(PERSIST_STALE)?;
            if let Some(persist) = slot.persist.as_ref()
                && persist.revision == revision
                && persist.path == path
                && persist.current(slot)
            {
                return Ok(persist.receipt()?.unwrap_or_else(|| {
                    serde_json::json!({"pending":true,"revision":revision}).to_string()
                }));
            }
            if kernel.engine.revision() != revision {
                return Err(PERSIST_STALE.into());
            }
            if !self.retired.is_empty() {
                return Err(RETIREMENT_BUSY.into());
            }
            let snapshot = kernel.document().clone();
            let root = RootIdentity::capture(&snapshot);
            let epoch = slot.document_epoch;
            let document = slot.document.clone().ok_or(PERSIST_STALE)?;
            let generation = self
                .generation
                .checked_add(1)
                .ok_or("The native document generation is exhausted")?;
            let cancelled = Arc::new(AtomicBool::new(false));
            if self.reader.is_none() {
                self.reader = Some(Reader::new(self.process, self.metrics.clone())?);
            }
            self.reader
                .as_ref()
                .expect("created")
                .submit_job(Job::Persist(PersistWork {
                    owner,
                    generation,
                    path: path.clone(),
                    document: snapshot,
                    cancelled: cancelled.clone(),
                }))?;
            self.generation = generation;
            let persist = Persist {
                generation,
                epoch,
                document,
                revision,
                root,
                path,
                cancelled,
                completed: None,
            };
            let old = self
                .slots
                .get_mut(&owner)
                .expect("checked")
                .persist
                .replace(persist);
            if let Some(old) = old {
                self.cancel_persist(old);
            }
            return Ok(serde_json::json!({"pending":true,"revision":revision}).to_string());
        }
        if op == "pollPersist" || op == "acceptPersist" {
            let revision = integer(args, "revision")?;
            let slot = self.slots.get_mut(&owner).ok_or(PERSIST_STALE)?;
            let persist = slot.persist.as_ref().ok_or(PERSIST_STALE)?;
            if persist.revision != revision {
                return Err(PERSIST_STALE.into());
            }
            if !persist.current(slot) {
                let old = slot.persist.take().expect("checked");
                self.cancel_persist(old);
                return Err(PERSIST_STALE.into());
            }
            let receipt = persist.receipt()?;
            if op == "pollPersist" {
                return Ok(receipt.unwrap_or_else(|| {
                    serde_json::json!({"pending":true,"revision":revision}).to_string()
                }));
            }
            let receipt = receipt.ok_or("The native checkpoint persistence is still pending")?;
            let hash = text(args, "hash")?;
            let persisted = persist
                .completed
                .as_ref()
                .expect("ready")
                .as_ref()
                .map_err(Clone::clone)?;
            if persisted.prepared.hash != hash {
                return Err(PERSIST_STALE.into());
            }
            let mut persist = slot.persist.take().expect("checked");
            let mut persisted = persist.completed.take().expect("ready")?;
            persisted.file.keep = true;
            self.retire(RetiredValue::Persisted(persisted));
            return Ok(receipt);
        }
        if op == "refresh" {
            let revision = integer(args, "revision")?;
            let slot = self.slots.get(&owner).ok_or(REFRESH_STALE)?;
            let kernel = slot.kernel.as_ref().ok_or(REFRESH_STALE)?;
            if kernel.engine.revision() != revision {
                return Err(REFRESH_STALE.into());
            }
            if let Some(refresh) = slot.refresh.as_ref()
                && refresh.revision == revision
                && refresh.current(slot)
            {
                return Ok(refresh.receipt()?.unwrap_or_else(|| {
                    serde_json::json!({"pending":true,"revision":revision}).to_string()
                }));
            }
            let cached = kernel
                .cached_hash()
                .zip(kernel.cached_integrity())
                .map(|(hash, integrity)| (hash.to_owned(), integrity.to_owned()));
            let document = cached.is_none().then(|| kernel.document().clone());
            let document_id = slot.document.clone().ok_or(REFRESH_STALE)?;
            let root = RootIdentity::capture(kernel.document());
            let source_bytes = kernel.source_bytes;
            let generation = self
                .generation
                .checked_add(1)
                .ok_or("The native document generation is exhausted")?;
            let cancelled = Arc::new(AtomicBool::new(false));
            if let Some(document) = document {
                if !self.retired.is_empty() {
                    return Err(RETIREMENT_BUSY.into());
                }
                if self.reader.is_none() {
                    self.reader = Some(Reader::new(self.process, self.metrics.clone())?);
                }
                self.reader
                    .as_ref()
                    .expect("created")
                    .submit_job(Job::Refresh(RefreshWork {
                        owner,
                        generation,
                        document,
                        source_bytes,
                        cancelled: cancelled.clone(),
                    }))?;
            }
            self.generation = generation;
            let refresh = Refresh {
                generation,
                document: document_id,
                revision,
                root,
                cancelled,
                completed: None,
                cached,
            };
            let answer = refresh.receipt()?.unwrap_or_else(|| {
                serde_json::json!({"pending":true,"revision":revision}).to_string()
            });
            let old = self
                .slots
                .get_mut(&owner)
                .expect("checked")
                .refresh
                .replace(refresh);
            if let Some(old) = old {
                self.cancel_refresh(old);
            }
            return Ok(answer);
        }
        if op == "pollRefresh" || op == "checkpoint" {
            let revision = integer(args, "revision")?;
            let slot = self.slots.get_mut(&owner).ok_or(REFRESH_STALE)?;
            let refresh = slot.refresh.as_ref().ok_or(REFRESH_STALE)?;
            if refresh.revision != revision {
                return Err(REFRESH_STALE.into());
            }
            if !refresh.current(slot) {
                let old = slot.refresh.take().expect("checked");
                self.cancel_refresh(old);
                return Err(REFRESH_STALE.into());
            }
            let receipt = refresh.receipt()?;
            if op == "pollRefresh" {
                return Ok(receipt.unwrap_or_else(|| {
                    serde_json::json!({"pending":true,"revision":revision}).to_string()
                }));
            }
            let hash = text(args, "hash")?;
            let expected_hash = match refresh.completed.as_ref() {
                Some(Ok(prepared)) => Some(prepared.hash.as_str()),
                Some(Err(error)) => return Err(error.clone()),
                None => refresh.cached.as_ref().map(|(hash, _)| hash.as_str()),
            };
            if expected_hash != Some(hash) {
                return Err(REFRESH_STALE.into());
            }
            let mut refresh = slot.refresh.take().expect("checked");
            let kernel = slot.kernel.as_mut().expect("current");
            let old_checkpoint = engine.document_kernel_checkpoint(owner);
            let mut prepared = refresh.completed.take().transpose()?;
            if let Some(prepared) = prepared.as_mut() {
                kernel.cache_refresh(prepared);
            }
            slot.checkpoint_batches = Some(0);
            slot.checkpoint_bytes = 0;
            engine.register_document_kernel(
                owner,
                refresh.document.clone(),
                kernel.document().clone(),
                Some(hash.to_owned()),
            );
            let estimate = kernel.tree_bytes_estimate.max(kernel.source_bytes);
            if let Some(old) = old_checkpoint {
                self.retire(RetiredValue::Document(old, estimate));
            }
            if let Some(prepared) = prepared {
                self.retire(RetiredValue::Prepared(prepared));
            }
            return receipt
                .ok_or_else(|| "The native document checkpoint refresh is still pending".into());
        }
        if op == "load" {
            // Do not retain unbounded obsolete trees behind a long parse.
            // Caller retries after the existing worker drains retirement.
            if !self.retired.is_empty() {
                return Err(RETIREMENT_BUSY.into());
            }
            let path = text(args, "path")?.to_owned();
            let document = DocumentId::new(text(args, "document")?);
            if !document.is_valid() {
                return Err("The document id is invalid".into());
            }
            let revision = integer(args, "revision")?;
            let hash = text(args, "hash")?.to_owned();
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("hash must be a lowercase canonical SHA-256".into());
            }
            let source_text = match member(args, "source") {
                None => false,
                Some(source) if source.as_str() == Some("text") => true,
                Some(_) => return Err("source must be \"text\" when given".into()),
            };
            if self.reader.is_none() {
                self.reader = Some(Reader::new(self.process, self.metrics.clone())?);
            }
            let generation = self
                .generation
                .checked_add(1)
                .ok_or("The native document generation is exhausted")?;
            let cancelled = Arc::new(AtomicBool::new(false));
            self.reader.as_ref().expect("created").submit(Work {
                owner,
                generation,
                path,
                revision,
                hash,
                text: source_text,
                cancelled: cancelled.clone(),
            })?;
            self.generation = generation;
            let slot = self.slots.entry(owner).or_default();
            let old = slot.pending.replace(Pending {
                generation,
                document,
                cancelled,
                completed: None,
            });
            if let Some(pending) = old {
                self.cancel_pending(pending);
            }
            return Ok("{\"pending\":true}".into());
        }
        if op == "poll" {
            let slot = self
                .slots
                .get_mut(&owner)
                .ok_or("The document kernel owner is not open")?;
            if let Some(pending) = slot.pending.as_mut() {
                let Some(result) = pending.completed.as_ref() else {
                    return Ok("{\"pending\":true}".into());
                };
                match result {
                    Ok(prepared) => return Ok(serde_json::json!({"pending":false,"revision":prepared.document.revision(),"hash":prepared.hash,"integrity":prepared.integrity}).to_string()),
                    Err(error) => { let error = error.clone(); slot.pending = None; return Err(error); }
                }
            }
            let kernel = slot
                .kernel
                .as_mut()
                .ok_or("The document kernel has no live document")?;
            let hash = kernel
                .cached_hash()
                .ok_or("The document changed; refresh its hash before polling")?;
            let integrity = kernel
                .cached_integrity()
                .ok_or("The document changed; refresh its integrity before polling")?;
            return Ok(serde_json::json!({"pending":false,"revision":kernel.engine.revision(),"hash":hash,"integrity":integrity}).to_string());
        }
        if op == "activate" {
            let revision = integer(args, "revision")?;
            let prepared_revision = integer(args, "preparedRevision")?;
            let hash = text(args, "hash")?;
            let slot = self
                .slots
                .get_mut(&owner)
                .ok_or("The document kernel owner is not open")?;
            let pending = slot
                .pending
                .as_ref()
                .ok_or("The document kernel has no prepared replacement")?;
            let prepared = pending
                .completed
                .as_ref()
                .ok_or("The document kernel replacement is still pending")?
                .as_ref()
                .map_err(Clone::clone)?;
            if prepared.document.revision() != prepared_revision || prepared.hash != hash {
                return Err("The document kernel replacement revision or hash changed".into());
            }
            if revision < prepared_revision
                || slot
                    .kernel
                    .as_ref()
                    .is_some_and(|kernel| revision <= kernel.engine.revision())
            {
                return Err("The document kernel activation must follow the live revision".into());
            }
            if let Some(document) = member(args, "document")
                && document.as_str() != Some(pending.document.as_str())
            {
                return Err("The document kernel replacement document changed".into());
            }
            let epoch = slot
                .document_epoch
                .checked_add(1)
                .ok_or("The native document epoch is exhausted")?;
            let mut pending = slot.pending.take().expect("checked");
            let mut prepared = pending.completed.take().expect("checked")?;
            let refresh = slot.refresh.take();
            let persist = slot.persist.take();
            let old_checkpoint = engine.document_kernel_checkpoint(owner);
            let old_document = slot.kernel.as_ref().map(|kernel| {
                (
                    kernel.document().clone(),
                    kernel.tree_bytes_estimate.max(kernel.source_bytes),
                )
            });
            // Rebase only the document's revision. The worker's container
            // identities and warmed consistency entries remain unchanged.
            prepared.document = prepared.document.shared_at_revision(revision);
            slot.kernel
                .get_or_insert_with(Kernel::default)
                .adopt(prepared);
            slot.document = Some(pending.document.clone());
            slot.document_epoch = epoch;
            slot.checkpoint_batches = Some(0);
            slot.checkpoint_bytes = 0;
            Self::register(engine, owner, slot, true);
            if let Some(refresh) = refresh {
                self.cancel_refresh(refresh);
            }
            if let Some(persist) = persist {
                self.cancel_persist(persist);
            }
            if let Some(checkpoint) = old_checkpoint {
                self.retire(RetiredValue::Document(
                    checkpoint,
                    old_document.as_ref().map_or(0, |(_, bytes)| *bytes),
                ));
            }
            if let Some((document, bytes)) = old_document {
                self.retire(RetiredValue::Document(document, bytes));
            }
            return Ok("null".into());
        }
        if op == "resync" {
            // Reject before creating a slot or cancelling a valid pending load.
            let json = text(args, "json")?;
            if json.len() > pm_wasm::kernel::MAX_SYNC_RESYNC {
                return Err("resync exceeds 512 MiB; use load".into());
            }
            integer(args, "revision")?;
            let document = member(args, "document")
                .map(|_| text(args, "document").map(DocumentId::new))
                .transpose()?;
            if document.as_ref().is_some_and(|d| !d.is_valid()) {
                return Err("The document id is invalid".into());
            }
            let slot = self.slots.entry(owner).or_default();
            let epoch = slot
                .document_epoch
                .checked_add(1)
                .ok_or("The native document epoch is exhausted")?;
            let old_checkpoint = engine.document_kernel_checkpoint(owner);
            let old_document = slot.kernel.as_ref().map(|kernel| {
                (
                    kernel.document().clone(),
                    kernel.tree_bytes_estimate.max(kernel.source_bytes),
                )
            });
            let answer = slot
                .kernel
                .get_or_insert_with(Kernel::default)
                .dispatch(op, args)?;
            let pending = slot.pending.take();
            let refresh = slot.refresh.take();
            let persist = slot.persist.take();
            slot.document = document
                .or_else(|| slot.document.clone())
                .or_else(|| Some(DocumentId::new(format!("kernel-{owner}"))));
            slot.document_epoch = epoch;
            slot.checkpoint_batches = Some(0);
            slot.checkpoint_bytes = 0;
            Self::register(engine, owner, slot, true);
            if let Some(refresh) = refresh {
                self.cancel_refresh(refresh);
            }
            if let Some(persist) = persist {
                self.cancel_persist(persist);
            }
            if let Some(checkpoint) = old_checkpoint {
                self.retire(RetiredValue::Document(
                    checkpoint,
                    old_document.as_ref().map_or(0, |(_, bytes)| *bytes),
                ));
            }
            if let Some((document, bytes)) = old_document {
                self.retire(RetiredValue::Document(document, bytes));
            }
            if let Some(pending) = pending {
                self.cancel_pending(pending);
            }
            return Ok(answer);
        }
        let slot = self
            .slots
            .get_mut(&owner)
            .ok_or("The document kernel owner is not open")?;
        let kernel = slot
            .kernel
            .as_mut()
            .ok_or("The document kernel has no live document")?;
        let mut answer = kernel.dispatch_bytes(op, args, data)?;
        self.reply_special = kernel.reply_special;
        if op == "heap" {
            let mut heap: serde_json::Value =
                serde_json::from_str(&answer).map_err(|e| e.to_string())?;
            let prepared = slot
                .pending
                .as_ref()
                .and_then(|p| p.completed.as_ref())
                .and_then(|r| r.as_ref().ok());
            let prepared_bytes = prepared.map_or(0, |p| p.tree_bytes_estimate.max(p.source_bytes));
            let prepared_source = prepared.map_or(0, |p| p.source_bytes);
            let reader_owner = self.metrics.owner.load(Ordering::Relaxed);
            let reader_input = self.metrics.input.load(Ordering::Relaxed);
            let live = heap["live"]
                .as_u64()
                .unwrap_or(0)
                .saturating_add(prepared_bytes as u64)
                .saturating_add(if reader_owner == owner as usize {
                    reader_input as u64
                } else {
                    0
                });
            heap["live"] = live.into();
            heap["peak"] = heap["peak"].as_u64().unwrap_or(0).max(live).into();
            heap["preparedTreeBytesEstimate"] = prepared_bytes.into();
            heap["preparedSourceBytes"] = prepared_source.into();
            heap["readerOwner"] = reader_owner.into();
            heap["readerInputBytes"] = reader_input.into();
            heap["readerInputPeak"] = self.metrics.input_peak.load(Ordering::Relaxed).into();
            heap["retiredTreeBytesEstimate"] = self.metrics.retired.load(Ordering::Relaxed).into();
            heap["retiredOnReader"] = self
                .metrics
                .retired_on_reader
                .load(Ordering::Relaxed)
                .into();
            heap["retiredInline"] = self.metrics.retired_inline.load(Ordering::Relaxed).into();
            answer = heap.to_string();
        }
        if matches!(op, "apply" | "hash") {
            Self::register(engine, owner, slot, false);
        }
        let mut expired_checkpoint = None;
        if op == "apply"
            && serde_json::from_str::<serde_json::Value>(&answer)
                .ok()
                .is_some_and(|reply| reply["code"] == 0)
            && let Some(batches) = slot.checkpoint_batches
        {
            let bytes = data.map_or_else(
                || {
                    member(args, "body")
                        .and_then(Value::as_array)
                        .map_or(0, |body| body.len())
                },
                |body| body.len(),
            );
            slot.checkpoint_bytes = slot.checkpoint_bytes.saturating_add(bytes);
            // Match barrier.emit exactly: 128 batches and 8 MiB are usable;
            // the next batch (or first byte above the limit) abandons them.
            if batches >= MAX_CHECKPOINT_BATCHES || slot.checkpoint_bytes > MAX_CHECKPOINT_BYTES {
                slot.checkpoint_batches = None;
                slot.checkpoint_bytes = 0;
                let estimate = slot.kernel.as_ref().map_or(0, |kernel| {
                    kernel.tree_bytes_estimate.max(kernel.source_bytes)
                });
                expired_checkpoint = engine
                    .drop_document_kernel_checkpoint(owner)
                    .map(|document| (document, estimate));
            } else {
                slot.checkpoint_batches = Some(batches + 1);
            }
        }
        if slot
            .refresh
            .as_ref()
            .is_some_and(|refresh| !refresh.current(slot))
        {
            let refresh = slot.refresh.take().expect("checked");
            self.cancel_refresh(refresh);
        }
        if let Some((document, estimate)) = expired_checkpoint {
            self.retire(RetiredValue::Document(document, estimate));
        }
        Ok(answer)
    }
}
impl Drop for NativeKernels {
    fn drop(&mut self) {
        // Cancel running/queued work before joining the sole reader thread.
        self.slots.clear();
        self.reader.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_engine::{Engine, EngineConfig};
    use serde_json::{Value as Json, json};
    use std::time::{Duration, Instant};

    struct CacheFile(std::path::PathBuf);
    impl CacheFile {
        fn new(bytes: &[u8]) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "pm-kernel-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
        fn fresh() -> Self {
            let file = Self::new(b"");
            std::fs::remove_file(&file.0).unwrap();
            file
        }
    }
    impl Drop for CacheFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    fn engine() -> Engine {
        Engine::new(EngineConfig::new(EngineProcess::Interactive))
    }
    fn call(k: &mut NativeKernels, e: &mut Engine, owner: u32, op: &str, args: Json) -> Json {
        serde_json::from_str(&k.call(e, owner, &json!({"op":op,"args":args}).to_string())).unwrap()
    }
    fn value(k: &mut NativeKernels, e: &mut Engine, owner: u32, op: &str, args: Json) -> Json {
        let reply = call(k, e, owner, op, args);
        assert_eq!(reply["ok"], true, "{reply}");
        reply["value"].clone()
    }
    fn load(
        k: &mut NativeKernels,
        e: &mut Engine,
        owner: u32,
        file: &CacheFile,
        revision: u64,
        hash: &str,
    ) {
        assert_eq!(
            value(
                k,
                e,
                owner,
                "load",
                json!({"path":file.0,"document":"doc","revision":revision,"hash":hash})
            ),
            json!({"pending":true})
        );
    }
    fn poll(k: &mut NativeKernels, e: &mut Engine, owner: u32) -> Json {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let reply = call(k, e, owner, "poll", json!({}));
            if reply["ok"] == false || reply["value"]["pending"] == false {
                return reply;
            }
            assert!(
                Instant::now() < deadline,
                "native ingestion did not complete"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn poll_refresh(k: &mut NativeKernels, e: &mut Engine, owner: u32, revision: u64) -> Json {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let reply = call(k, e, owner, "pollRefresh", json!({"revision":revision}));
            if reply["ok"] == false || reply["value"]["pending"] == false {
                return reply;
            }
            assert!(
                Instant::now() < deadline,
                "checkpoint refresh did not complete"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn poll_persist(k: &mut NativeKernels, e: &mut Engine, revision: u64) -> Json {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let reply = call(k, e, 1, "pollPersist", json!({"revision":revision}));
            if reply["ok"] == false || reply["value"]["pending"] == false {
                return reply;
            }
            assert!(
                Instant::now() < deadline,
                "checkpoint persistence did not complete"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn wait_removed(file: &CacheFile) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while file.0.exists() {
            assert!(
                Instant::now() < deadline,
                "unaccepted checkpoint file was retained"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn kernel_ingests_json_text_in_any_member_order_against_its_byte_hash() {
        // JSON.stringify's order, escapes and a lone surrogate; the receipt is
        // the canonical hash of the parsed tree, as for a canonical stream.
        let text = format!(
            r#"{{"z":0,"s":"\ud800","layers":[{{"v":1.5,"t":0,"id":"L"}}],"big":"{}","a":true}}"#,
            "x".repeat(70_000)
        );
        let canonical = pm_model::json::to_canonical_string(&pm_model::json::parse(&text).unwrap());
        assert_ne!(canonical, text);
        let file = CacheFile::new(text.as_bytes());
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        let source = pm_model::hash::sha256_hex(text.as_bytes());
        assert_eq!(
            value(
                &mut k,
                &mut e,
                1,
                "load",
                json!({"path":file.0,"document":"doc","revision":3,"hash":source,"source":"text"})
            ),
            json!({"pending":true})
        );
        let receipt = poll(&mut k, &mut e, 1);
        assert_eq!(receipt["ok"], true, "{receipt}");
        assert_eq!(
            receipt["value"]["hash"],
            pm_model::hash::sha256_hex(canonical.as_bytes())
        );
        assert_eq!(receipt["value"]["revision"], 3);
        let canonical_hash = receipt["value"]["hash"].clone();
        value(
            &mut k,
            &mut e,
            1,
            "activate",
            json!({"revision":3,"preparedRevision":3,"hash":canonical_hash}),
        );
        assert_eq!(
            k.slots[&1].kernel.as_ref().unwrap().engine.to_json(),
            canonical
        );

        // Text whose bytes differ from the hash main computed is refused; a
        // canonical-mode load of the same non-canonical file is refused too.
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        value(
            &mut k,
            &mut e,
            1,
            "load",
            json!({"path":file.0,"document":"doc","revision":3,"hash":pm_model::hash::sha256_hex(b"other"),"source":"text"}),
        );
        let refused = poll(&mut k, &mut e, 1);
        assert_eq!(refused["ok"], false, "{refused}");
        load(&mut k, &mut e, 1, &file, 3, &source);
        let refused = poll(&mut k, &mut e, 1);
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "load",
                json!({"path":file.0,"document":"doc","revision":3,"hash":source,"source":"binary"})
            )["ok"],
            false
        );
    }

    #[test]
    fn kernel_persists_historical_canonical_bytes_and_restores_with_ordered_suffix() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        let initial = format!(
            r#"{{"z":0,"s":"\ud800","big":"{}","a":true,"value":1}}"#,
            "x".repeat(2 * 1024 * 1024)
        );
        let expected =
            pm_model::json::to_canonical_string(&pm_model::json::parse(&initial).unwrap());
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":initial,"revision":1,"document":"doc"}),
        );
        let file = CacheFile::fresh();
        assert_eq!(
            value(
                &mut k,
                &mut e,
                1,
                "persist",
                json!({"revision":1,"path":file.0})
            )["pending"],
            true
        );
        edit(&mut k, &mut e, 1, 2);
        let receipt = poll_persist(&mut k, &mut e, 1);
        assert_eq!(receipt["ok"], true, "{receipt}");
        assert_eq!(receipt["value"]["revision"], 1);
        assert_eq!(receipt["value"]["bytes"], expected.len());
        assert_eq!(
            receipt["value"]["hash"],
            pm_model::hash::sha256_hex(expected.as_bytes())
        );
        assert!(receipt["value"]["writeChunkPeak"].as_u64().unwrap() <= 8192 + 32);
        assert_eq!(std::fs::read(&file.0).unwrap(), expected.as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file.0).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(e.document_kernel_checkpoint(1).unwrap().revision(), 1);
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "acceptPersist",
                json!({"revision":1,"hash":"wrong"})
            )["code"],
            "stale"
        );
        value(
            &mut k,
            &mut e,
            1,
            "acceptPersist",
            json!({"revision":1,"hash":receipt["value"]["hash"]}),
        );
        assert_eq!(
            k.slots[&1].checkpoint_batches,
            Some(1),
            "disk compaction must preserve the renderer journal"
        );
        value(&mut k, &mut e, 1, "close", json!({}));
        assert!(file.0.exists(), "accepted cache files belong to main");
        let mut restored = NativeKernels::new(EngineProcess::Interactive);
        let mut restored_engine = engine();
        load(
            &mut restored,
            &mut restored_engine,
            1,
            &file,
            1,
            receipt["value"]["hash"].as_str().unwrap(),
        );
        assert_eq!(poll(&mut restored, &mut restored_engine, 1)["ok"], true);
        value(
            &mut restored,
            &mut restored_engine,
            1,
            "activate",
            json!({"revision":1,"preparedRevision":1,"hash":receipt["value"]["hash"]}),
        );
        edit(&mut restored, &mut restored_engine, 1, 2);
        let final_json = restored.slots[&1].kernel.as_ref().unwrap().engine.to_json();
        assert_eq!(final_json, expected.replace("\"value\":1", "\"value\":2"));
    }

    #[test]
    fn kernel_persist_cleanup_fences_cancel_replacement_and_owner_reuse() {
        for transition in ["cancelPersist", "resync", "close"] {
            let mut k = NativeKernels::new(EngineProcess::Interactive);
            let mut e = engine();
            value(
                &mut k,
                &mut e,
                1,
                "resync",
                json!({"json":"{\"value\":1}","revision":1,"document":"doc"}),
            );
            let file = CacheFile::fresh();
            value(
                &mut k,
                &mut e,
                1,
                "persist",
                json!({"revision":1,"path":file.0}),
            );
            assert_eq!(poll_persist(&mut k, &mut e, 1)["ok"], true);
            assert!(file.0.exists());
            assert_eq!(
                call(&mut k, &mut e, 1, "pollPersist", json!({"revision":0}))["code"],
                "stale"
            );
            assert_eq!(
                poll_persist(&mut k, &mut e, 1)["ok"],
                true,
                "stale callers must not cancel current persistence"
            );
            value(
                &mut k,
                &mut e,
                1,
                transition,
                match transition {
                    "cancelPersist" => json!({"revision":1}),
                    "resync" => {
                        json!({"json":"{\"replacement\":true}","revision":1,"document":"doc"})
                    }
                    _ => json!({}),
                },
            );
            wait_removed(&file);
            assert_eq!(
                call(
                    &mut k,
                    &mut e,
                    1,
                    "acceptPersist",
                    json!({"revision":1,"hash":pm_model::hash::sha256_hex(br#"{"value":1}"#)})
                )["code"],
                "stale"
            );
            assert_eq!(k.metrics.retired_inline.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn kernel_persist_create_new_never_clobbers_an_existing_checkpoint() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":"{}","revision":1,"document":"doc"}),
        );
        let file = CacheFile::new(b"retained checkpoint");
        value(
            &mut k,
            &mut e,
            1,
            "persist",
            json!({"revision":1,"path":file.0}),
        );
        assert_eq!(poll_persist(&mut k, &mut e, 1)["ok"], false);
        assert_eq!(std::fs::read(&file.0).unwrap(), b"retained checkpoint");
    }

    fn edit(k: &mut NativeKernels, e: &mut Engine, base: u64, revision: u64) {
        let op = pm_model::ops::Op::from_value(
            &pm_model::json::parse(
                &json!({"path":["value"],"exists":true,"value":revision}).to_string(),
            )
            .unwrap(),
        )
        .unwrap();
        let body = pm_model::binary::encode(&[op]).unwrap();
        let reply: Json = serde_json::from_str(&k.call_bytes(
            e,
            1,
            &json!({"op":"apply","args":{"base":base,"revision":revision}}).to_string(),
            Some(&body),
        ))
        .unwrap();
        assert_eq!(reply["value"]["code"], 0);
    }

    #[test]
    fn kernel_refresh_warms_off_thread_and_publishes_only_an_accepted_checkpoint() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        let initial = r#"{"comps":{"c":{"layers":[{"id":"a","p":{"x":{"v":7}}}]}},"value":1}"#;
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":initial,"revision":1,"document":"doc"}),
        );
        let cached = value(&mut k, &mut e, 1, "refresh", json!({"revision":1}));
        assert_eq!(cached["pending"], false);
        assert!(
            k.reader.is_none(),
            "warm receipts must not queue worker work"
        );
        assert_eq!(
            cached["hash"],
            pm_model::hash::sha256_hex(initial.as_bytes())
        );
        edit(&mut k, &mut e, 1, 2);
        let id = value(
            &mut k,
            &mut e,
            1,
            "intern",
            json!({"comp":"c","layer":"a","key":"x","via":"channel"}),
        );
        assert_eq!(
            value(&mut k, &mut e, 1, "ev", json!({"id":id,"t":0}))["number"],
            7
        );
        let tree = k.slots[&1].kernel.as_ref().unwrap().document().clone();
        let resyncs = k.slots[&1].kernel.as_ref().unwrap().engine.stats.resyncs;
        let plans = k.slots[&1]
            .kernel
            .as_ref()
            .unwrap()
            .engine
            .evaluation_cache_stats()
            .plans;
        assert_eq!(
            value(&mut k, &mut e, 1, "refresh", json!({"revision":2}))["pending"],
            true
        );
        let receipt = poll_refresh(&mut k, &mut e, 1, 2);
        assert_eq!(receipt["ok"], true);
        assert_eq!(
            receipt["value"]["hash"],
            pm_model::hash::sha256_hex(initial.replace("\"value\":1", "\"value\":2").as_bytes())
        );
        assert_eq!(
            e.document_kernel_checkpoint(1).unwrap().revision(),
            1,
            "worker completion must preserve the renderer's old checkpoint"
        );
        assert!(k.slots[&1].kernel.as_ref().unwrap().cached_hash().is_none());
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "checkpoint",
                json!({"revision":2,"hash":"wrong"})
            )["code"],
            "stale"
        );
        assert_eq!(e.document_kernel_checkpoint(1).unwrap().revision(), 1);
        let accepted = value(
            &mut k,
            &mut e,
            1,
            "checkpoint",
            json!({"revision":2,"hash":receipt["value"]["hash"]}),
        );
        assert_eq!(accepted, receipt["value"]);
        assert_eq!(e.document_kernel_checkpoint(1).unwrap().revision(), 2);
        let kernel = k.slots[&1].kernel.as_ref().unwrap();
        assert_eq!(
            kernel.engine.stats.resyncs, resyncs,
            "refresh must preserve evaluator state"
        );
        assert_eq!(kernel.engine.evaluation_cache_stats().plans, plans);
        let (NodeKind::Object(before), NodeKind::Object(after)) = (
            tree.root().unwrap().kind(),
            kernel.document().root().unwrap().kind(),
        ) else {
            panic!("objects");
        };
        assert!(
            Arc::ptr_eq(before, after),
            "refresh must not parse or rebuild the tree"
        );
        assert_eq!(
            kernel.cached_integrity(),
            receipt["value"]["integrity"].as_str()
        );
        assert_eq!(
            value(&mut k, &mut e, 1, "ev", json!({"id":id,"t":0}))["number"],
            7
        );
        assert_eq!(
            value(&mut k, &mut e, 1, "hash", json!({})),
            receipt["value"]["hash"]
        );
        assert_eq!(
            value(&mut k, &mut e, 1, "consistencyHash", json!({})),
            receipt["value"]["integrity"]
        );
        assert_eq!(
            value(&mut k, &mut e, 1, "refresh", json!({"revision":2}))["pending"],
            false
        );
        assert_eq!(
            call(&mut k, &mut e, 1, "pollRefresh", json!({"revision":1}))["code"],
            "stale"
        );
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "checkpoint",
                json!({"revision":1,"hash":cached["hash"]})
            )["code"],
            "stale"
        );
        assert_eq!(
            value(&mut k, &mut e, 1, "pollRefresh", json!({"revision":2}))["hash"],
            receipt["value"]["hash"],
            "obsolete callers must not cancel a newer refresh"
        );
    }

    #[test]
    fn kernel_refresh_rejects_edits_replacements_and_owner_reuse_even_at_the_same_revision() {
        for transition in ["edit", "resync", "close"] {
            let mut k = NativeKernels::new(EngineProcess::Interactive);
            let mut e = engine();
            value(
                &mut k,
                &mut e,
                1,
                "resync",
                json!({"json":"{}","revision":1,"document":"doc"}),
            );
            edit(&mut k, &mut e, 1, 2);
            let (input, jobs) = mpsc::sync_channel(2);
            let (answers, output) = mpsc::channel();
            k.reader = Some(Reader {
                input: Some(input),
                output,
                persisted: mpsc::channel().1,
                thread: None,
            });
            value(&mut k, &mut e, 1, "refresh", json!({"revision":2}));
            let Job::Refresh(work) = jobs.recv().unwrap() else {
                panic!("refresh job");
            };
            if transition == "edit" {
                edit(&mut k, &mut e, 2, 3);
            } else {
                if transition == "close" {
                    value(&mut k, &mut e, 1, "close", json!({}));
                }
                value(
                    &mut k,
                    &mut e,
                    1,
                    "resync",
                    json!({"json":"{\"replacement\":true}","revision":2,"document":"doc"}),
                );
            }
            assert!(work.cancelled.load(Ordering::Relaxed));
            // Deliver a completion that raced cancellation. The generation
            // and root fence must reject it without touching the live tree.
            answers
                .send(Completed {
                    owner: 1,
                    generation: work.generation,
                    result: Ok(PreparedDocument::from_document(
                        work.document,
                        work.source_bytes,
                    )),
                })
                .unwrap();
            assert_eq!(
                call(&mut k, &mut e, 1, "pollRefresh", json!({"revision":2}))["code"],
                "stale",
                "{transition}"
            );
            assert_eq!(
                call(
                    &mut k,
                    &mut e,
                    1,
                    "checkpoint",
                    json!({"revision":2,"hash":pm_model::hash::sha256_hex(br#"{"value":2}"#)})
                )["code"],
                "stale",
                "{transition}"
            );
            let current = k.slots[&1].kernel.as_ref().unwrap();
            assert_eq!(
                current.engine.to_json(),
                if transition == "edit" {
                    r#"{"value":3}"#
                } else {
                    r#"{"replacement":true}"#
                }
            );
            assert_eq!(
                e.document_kernel_checkpoint(1).unwrap().revision(),
                if transition == "edit" { 1 } else { 2 }
            );
        }
    }

    #[test]
    fn kernel_worker_prepares_without_changing_live_reads_then_activation_is_fenced() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":"{\"value\":\"old\"}","revision":1,"document":"doc"}),
        );
        let bytes = pm_model::json::to_canonical_string(
            &pm_model::json::parse(&json!({"value":"new","data":vec!["abcd"; 32768]}).to_string())
                .unwrap(),
        );
        let file = CacheFile::new(bytes.as_bytes());
        let hash = pm_model::hash::sha256_hex(bytes.as_bytes());
        load(&mut k, &mut e, 1, &file, 2, &hash);
        // These execute on the host while parsing and cold hashing happen on
        // its bounded worker. They must keep observing the published project.
        let start = Instant::now();
        assert_eq!(value(&mut k, &mut e, 1, "revision", json!({})), 1);
        assert_eq!(
            value(
                &mut k,
                &mut e,
                1,
                "query",
                json!({"op":"mul","a":[1,0,0,1,0,0],"b":[1,0,0,1,2,3]})
            )["ok"],
            json!([1, 0, 0, 1, 2, 3])
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        let prepared = poll(&mut k, &mut e, 1);
        assert_eq!(prepared["ok"], true);
        assert_eq!(prepared["value"]["hash"], hash);
        assert_eq!(value(&mut k, &mut e, 1, "revision", json!({})), 1);
        assert_eq!(
            value(&mut k, &mut e, 1, "toJson", json!({})),
            "{\"value\":\"old\"}"
        );
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "activate",
                json!({"revision":1,"preparedRevision":2,"hash":hash})
            )["ok"],
            false
        );
        value(
            &mut k,
            &mut e,
            1,
            "activate",
            json!({"revision":2,"preparedRevision":2,"hash":hash}),
        );
        assert_eq!(value(&mut k, &mut e, 1, "revision", json!({})), 2);
        assert_eq!(
            value(&mut k, &mut e, 1, "consistencyHash", json!({})),
            prepared["value"]["integrity"]
        );
        assert_eq!(value(&mut k, &mut e, 1, "toJson", json!({})), bytes);
        let heap = value(&mut k, &mut e, 1, "heap", json!({}));
        assert!(heap["live"].as_u64().unwrap() >= bytes.len() as u64);
        assert_eq!(heap["estimated"], true);
    }

    #[test]
    fn kernel_bad_hash_and_cancel_preserve_the_old_live_project() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":"{\"value\":1}","revision":1}),
        );
        let file = CacheFile::new(br#"{"value":2}"#);
        load(&mut k, &mut e, 1, &file, 2, &"0".repeat(64));
        assert_eq!(poll(&mut k, &mut e, 1)["ok"], false);
        assert_eq!(value(&mut k, &mut e, 1, "revision", json!({})), 1);
        let hash = pm_model::hash::sha256_hex(br#"{"value":2}"#);
        load(&mut k, &mut e, 1, &file, 2, &hash);
        assert_eq!(poll(&mut k, &mut e, 1)["ok"], true);
        value(&mut k, &mut e, 1, "cancel", json!({}));
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "activate",
                json!({"revision":2,"preparedRevision":2,"hash":hash})
            )["ok"],
            false
        );
        assert_eq!(value(&mut k, &mut e, 1, "revision", json!({})), 1);
    }

    #[test]
    fn kernel_reader_releases_staging_before_ready_and_retires_cancelled_trees() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":"{}","revision":1}),
        );
        let bytes = json!({"data":vec!["a".repeat(1024);8192]}).to_string();
        let file = CacheFile::new(bytes.as_bytes());
        let hash = pm_model::hash::sha256_hex(bytes.as_bytes());
        load(&mut k, &mut e, 1, &file, 2, &hash);
        assert_eq!(poll(&mut k, &mut e, 1)["ok"], true);
        let heap = value(&mut k, &mut e, 1, "heap", json!({}));
        assert_eq!(heap["preparedSourceBytes"], bytes.len());
        assert_eq!(heap["readerInputBytes"], 0);
        assert_eq!(heap["readerInputPeak"], bytes.len() + 1024 * 1024);
        assert!(heap["preparedTreeBytesEstimate"].as_u64().unwrap() >= bytes.len() as u64);
        eprintln!(
            "native synthetic allocation attribution: source={}, reader_input_peak={}, ready_input={}, prepared_tree_estimate={}",
            bytes.len(),
            heap["readerInputPeak"],
            heap["readerInputBytes"],
            heap["preparedTreeBytesEstimate"]
        );
        let retired = k.metrics.retired_on_reader.load(Ordering::Relaxed);
        value(&mut k, &mut e, 1, "cancel", json!({}));
        let deadline = Instant::now() + Duration::from_secs(5);
        while k.metrics.retired_on_reader.load(Ordering::Relaxed) == retired {
            assert!(
                Instant::now() < deadline,
                "cancelled tree was not retired on reader"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(k.metrics.retired.load(Ordering::Relaxed), 0);
        assert_eq!(k.metrics.retired_inline.load(Ordering::Relaxed), 0);
        let heap = value(&mut k, &mut e, 1, "heap", json!({}));
        assert_eq!(heap["preparedTreeBytesEstimate"], 0);
        assert_eq!(heap["preparedSourceBytes"], 0);
        assert_eq!(value(&mut k, &mut e, 1, "revision", json!({})), 1);
    }

    #[test]
    fn kernel_retirement_backlog_is_bounded_and_blocks_more_ingestion() {
        let (input, jobs) = mpsc::sync_channel(2);
        let (_answers, output) = mpsc::channel();
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        k.reader = Some(Reader {
            input: Some(input),
            output,
            persisted: mpsc::channel().1,
            thread: None,
        });
        let file = CacheFile::new(b"{}");
        let hash = pm_model::hash::sha256_hex(b"{}");
        load(&mut k, &mut e, 1, &file, 1, &hash);
        load(&mut k, &mut e, 1, &file, 2, &hash);
        for _ in 0..MAX_DEFERRED_RETIREMENTS {
            k.retire(RetiredValue::Document(
                pm_model::Document::from_root(None, 0),
                100,
            ));
        }
        assert_eq!(k.retired.len(), MAX_DEFERRED_RETIREMENTS);
        assert_eq!(
            k.metrics.retired.load(Ordering::Relaxed),
            MAX_DEFERRED_RETIREMENTS * 100
        );
        assert_eq!(
            k.metrics.retired_inline.load(Ordering::Relaxed),
            0,
            "one owner teardown burst must stay off the utility thread"
        );
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "load",
                json!({"path":file.0,"document":"doc","revision":3,"hash":hash})
            )["code"],
            "busy"
        );
        assert_eq!(k.slots[&1].pending.as_ref().unwrap().generation, 2);
        assert!(matches!(jobs.recv().unwrap(), Job::Read(_)));
        assert!(matches!(jobs.recv().unwrap(), Job::Read(_)));
        while !k.retired.is_empty() {
            k.flush_retired();
            while let Ok(job) = jobs.try_recv() {
                assert!(matches!(job, Job::Retire(_)));
            }
        }
        assert_eq!(k.metrics.retired.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn kernel_expires_checkpoint_after_129_batches_and_releases_its_old_tree_on_reader() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        let initial = json!({"payload":["x".repeat(1024*1024)],"value":0}).to_string();
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":initial,"revision":1,"document":"doc"}),
        );
        let checkpoint = e.document_kernel_checkpoint(1).unwrap();
        let NodeKind::Object(root) = checkpoint.root().unwrap().kind() else {
            panic!("object");
        };
        let old_root = Arc::downgrade(root);
        drop(checkpoint);
        let remove = pm_model::ops::Op::from_value(
            &pm_model::json::parse(r#"{"path":["payload"],"exists":false}"#).unwrap(),
        )
        .unwrap();
        let body = pm_model::binary::encode(&[remove]).unwrap();
        let first: Json = serde_json::from_str(&k.call_bytes(
            &mut e,
            1,
            &json!({"op":"apply","args":{"base":1,"revision":2}}).to_string(),
            Some(&body),
        ))
        .unwrap();
        assert_eq!(first["value"]["code"], 0);
        for base in 2..=128 {
            edit(&mut k, &mut e, base, base + 1);
        }
        assert_eq!(k.slots[&1].checkpoint_batches, Some(128));
        assert!(e.document_kernel_checkpoint(1).is_some());
        assert!(old_root.strong_count() > 0);
        edit(&mut k, &mut e, 129, 130);
        assert!(e.document_kernel_checkpoint(1).is_none());
        assert_eq!(k.slots[&1].checkpoint_batches, None);
        let deadline = Instant::now() + Duration::from_secs(5);
        while old_root.strong_count() != 0 {
            assert!(
                Instant::now() < deadline,
                "expired checkpoint still pins the old tree"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(k.metrics.retired_inline.load(Ordering::Relaxed), 0);
        value(&mut k, &mut e, 1, "refresh", json!({"revision":130}));
        let refreshed = poll_refresh(&mut k, &mut e, 1, 130);
        assert_eq!(
            refreshed["value"]["hash"],
            pm_model::hash::sha256_hex(br#"{"value":130}"#)
        );
        value(
            &mut k,
            &mut e,
            1,
            "checkpoint",
            json!({"revision":130,"hash":refreshed["value"]["hash"]}),
        );
        assert_eq!(k.slots[&1].checkpoint_batches, Some(0));
        edit(&mut k, &mut e, 130, 131);
        assert_eq!(k.slots[&1].checkpoint_batches, Some(1));
        assert_eq!(e.document_kernel_checkpoint(1).unwrap().revision(), 130);
    }

    #[test]
    fn kernel_keeps_exact_8mib_checkpoint_suffix_then_expires_and_resets_on_resync() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":"{}","revision":1,"document":"doc"}),
        );
        let blob = |len| {
            pm_model::ops::Op::from_value(
                &pm_model::json::parse(
                    &json!({"path":["blob"],"exists":true,"value":"x".repeat(len)}).to_string(),
                )
                .unwrap(),
            )
            .unwrap()
        };
        let overhead = pm_model::binary::encode(&[blob(0)]).unwrap().len();
        let body = pm_model::binary::encode(&[blob(MAX_CHECKPOINT_BYTES - overhead)]).unwrap();
        assert_eq!(body.len(), MAX_CHECKPOINT_BYTES);
        let applied: Json = serde_json::from_str(&k.call_bytes(
            &mut e,
            1,
            &json!({"op":"apply","args":{"base":1,"revision":2}}).to_string(),
            Some(&body),
        ))
        .unwrap();
        assert_eq!(applied["value"]["code"], 0);
        assert_eq!(k.slots[&1].checkpoint_bytes, MAX_CHECKPOINT_BYTES);
        assert!(e.document_kernel_checkpoint(1).is_some());
        let next = pm_model::binary::encode(&[]).unwrap();
        let applied: Json = serde_json::from_str(&k.call_bytes(
            &mut e,
            1,
            &json!({"op":"apply","args":{"base":2,"revision":3}}).to_string(),
            Some(&next),
        ))
        .unwrap();
        assert_eq!(applied["value"]["code"], 0);
        assert!(e.document_kernel_checkpoint(1).is_none());
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":"{}","revision":4,"document":"doc"}),
        );
        assert_eq!(k.slots[&1].checkpoint_bytes, 0);
        assert_eq!(k.slots[&1].checkpoint_batches, Some(0));
        assert!(e.document_kernel_checkpoint(1).is_some());
    }

    #[test]
    fn kernel_activation_rebases_after_edits_to_the_live_project_without_reusing_old_plans() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        let old = r#"{"comps":{"c":{"layers":[{"id":"a","p":{"x":{"v":1}}}]}}}"#;
        let next = r#"{"comps":{"c":{"layers":[{"id":"a","p":{"x":{"v":99}}}]}}}"#;
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":old,"revision":3,"document":"doc"}),
        );
        let id = value(
            &mut k,
            &mut e,
            1,
            "intern",
            json!({"comp":"c","layer":"a","key":"x","via":"channel"}),
        );
        assert_eq!(
            value(&mut k, &mut e, 1, "ev", json!({"id":id,"t":0}))["number"],
            1
        );
        let file = CacheFile::new(next.as_bytes());
        let hash = pm_model::hash::sha256_hex(next.as_bytes());
        load(&mut k, &mut e, 1, &file, 4, &hash);
        assert_eq!(poll(&mut k, &mut e, 1)["value"]["revision"], 4);
        let op = pm_model::ops::Op::from_value(
            &pm_model::json::parse(
                r#"{"path":["comps","c","layers",0,"p","x","v"],"exists":true,"value":2}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let body = pm_model::binary::encode(&[op]).unwrap();
        assert_eq!(
            value(
                &mut k,
                &mut e,
                1,
                "apply",
                json!({"body":body,"base":3,"revision":6})
            )["code"],
            0
        );
        assert_eq!(
            value(&mut k, &mut e, 1, "ev", json!({"id":id,"t":0}))["number"],
            2
        );
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "activate",
                json!({"revision":4,"preparedRevision":4,"hash":hash})
            )["ok"],
            false
        );
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "activate",
                json!({"revision":7,"preparedRevision":5,"hash":hash})
            )["ok"],
            false
        );
        value(
            &mut k,
            &mut e,
            1,
            "activate",
            json!({"revision":7,"preparedRevision":4,"hash":hash}),
        );
        assert_eq!(value(&mut k, &mut e, 1, "revision", json!({})), 7);
        assert_eq!(
            k.slots[&1]
                .kernel
                .as_ref()
                .unwrap()
                .document()
                .root()
                .unwrap()
                .rev(),
            4,
            "rebasing must preserve the worker tree and its warmed identity cache"
        );
        assert_eq!(
            value(&mut k, &mut e, 1, "ev", json!({"id":id,"t":0}))["number"],
            99
        );
    }

    #[test]
    fn kernel_queue_backpressure_supersession_and_close_ignore_stale_completions() {
        let (input, jobs) = mpsc::sync_channel(2);
        let (answers, output) = mpsc::channel();
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        k.reader = Some(Reader {
            input: Some(input),
            output,
            persisted: mpsc::channel().1,
            thread: None,
        });
        let file = CacheFile::new(b"{}");
        let hash = pm_model::hash::sha256_hex(b"{}");
        load(&mut k, &mut e, 1, &file, 1, &hash);
        load(&mut k, &mut e, 1, &file, 2, &hash);
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "load",
                json!({"path":file.0,"document":"doc","revision":3,"hash":hash})
            )["code"],
            "busy"
        );
        let Job::Read(first) = jobs.recv().unwrap() else {
            panic!("first read");
        };
        let Job::Read(second) = jobs.recv().unwrap() else {
            panic!("second read");
        };
        assert!(first.cancelled.load(Ordering::Relaxed));
        assert!(!second.cancelled.load(Ordering::Relaxed));
        answers
            .send(Completed {
                owner: 1,
                generation: first.generation,
                result: PreparedDocument::parse(b"{}", 1),
            })
            .unwrap();
        assert_eq!(
            value(&mut k, &mut e, 1, "poll", json!({})),
            json!({"pending":true})
        );
        answers
            .send(Completed {
                owner: 1,
                generation: second.generation,
                result: PreparedDocument::parse(b"{}", 2),
            })
            .unwrap();
        assert_eq!(value(&mut k, &mut e, 1, "poll", json!({}))["revision"], 2);
        value(&mut k, &mut e, 1, "close", json!({}));
        assert!(second.cancelled.load(Ordering::Relaxed));
        answers
            .send(Completed {
                owner: 1,
                generation: second.generation,
                result: PreparedDocument::parse(b"{}", 2),
            })
            .unwrap();
        assert_eq!(call(&mut k, &mut e, 1, "poll", json!({}))["ok"], false);
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":"{}","revision":9}),
        );
        assert_eq!(value(&mut k, &mut e, 1, "revision", json!({})), 9);
    }

    #[test]
    fn kernel_owner_limits_and_invalid_inputs_do_not_create_live_state() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        assert_eq!(
            call(
                &mut k,
                &mut e,
                0,
                "resync",
                json!({"json":"{}","revision":0})
            )["ok"],
            false
        );
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "load",
                json!({"path":"unused","document":"","revision":0,"hash":"invalid"})
            )["ok"],
            false
        );
        assert!(k.slots.is_empty());
        for owner in 1..=32 {
            value(
                &mut k,
                &mut e,
                owner,
                "resync",
                json!({"json":"{}","revision":0}),
            );
        }
        assert_eq!(
            call(
                &mut k,
                &mut e,
                33,
                "resync",
                json!({"json":"{}","revision":0})
            )["ok"],
            false
        );
        assert_eq!(
            call(
                &mut k,
                &mut e,
                1,
                "apply",
                json!({"body":[-1],"base":0,"revision":1})
            )["ok"],
            false
        );
        value(&mut k, &mut e, 1, "close", json!({}));
        value(
            &mut k,
            &mut e,
            33,
            "resync",
            json!({"json":"{}","revision":0}),
        );
        assert_eq!(
            serde_json::from_str::<Json>(&k.call(&mut e, 33, "invalid")).unwrap()["ok"],
            false
        );
    }

    #[test]
    fn kernel_envelopes_flag_only_replies_that_require_number_revival() {
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":"{}","revision":1}),
        );
        let special = call(
            &mut k,
            &mut e,
            1,
            "query",
            json!({"op":"projectPoint","m":[0,0,0,0,0,0,0,0,0],"x":0,"y":0}),
        );
        assert_eq!(special["special"], true);
        assert_eq!(special["value"]["ok"]["x"], json!({"\0":"NaN"}));
        assert!(
            call(&mut k, &mut e, 1, "revision", json!({}))
                .get("special")
                .is_none()
        );
    }

    #[test]
    fn kernel_main_capture_rpc_uses_live_revision_without_refresh_or_json() {
        use pm_engine::{
            PeerKind,
            engine::testing::{hello, sent},
        };
        use pm_protocol::{
            codec,
            color::ProjectColor,
            common::{HostMicros, RequestId},
            envelope::{Envelope, Message, OpenSession, Request, Response, SessionRole},
            host::PortId,
            version::PeerRole,
        };
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        value(
            &mut k,
            &mut e,
            1,
            "resync",
            json!({"json":"{}","revision":1,"document":"doc"}),
        );
        edit(&mut k, &mut e, 1, 2);
        let port = PortId(50);
        e.attach(port, PeerKind::Host);
        e.receive(port, &hello(PeerRole::Host), HostMicros::ZERO);
        let request = Envelope::new(Message::Request {
            id: RequestId::new(1).unwrap(),
            body: Request::OpenSession(OpenSession {
                document: "doc".into(),
                role: SessionRole::Interactive,
                process: EngineProcess::Interactive,
                color: ProjectColor::DEFAULT,
            }),
        });
        let out = e.receive(port, &codec::encode(&request).unwrap(), HostMicros::ZERO);
        let session = sent(&out, port)
            .into_iter()
            .find_map(|answer| match answer.message {
                Message::Response {
                    body: Response::SessionOpened { session },
                    ..
                } => Some(session),
                _ => None,
            })
            .unwrap();
        for args in [
            json!({"session":session.get(),"document":"other","revision":2}),
            json!({"session":session.get(),"document":"doc","revision":1}),
        ] {
            assert_eq!(call(&mut k, &mut e, 1, "adoptSession", args)["ok"], false);
        }
        assert_eq!(
            value(
                &mut k,
                &mut e,
                1,
                "adoptSession",
                json!({"session":session.get(),"document":"doc","revision":2})
            ),
            Json::Null
        );
        assert!(k.slots[&1].kernel.as_ref().unwrap().cached_hash().is_none());
        assert!(
            k.reader.is_none(),
            "capture must not hash or stage a document"
        );
        assert_eq!(
            k.slots[&1].checkpoint_batches,
            Some(1),
            "capture must leave the renderer checkpoint journal intact"
        );
    }

    #[test]
    fn kernel_registration_serves_a_protocol_session_without_reading_its_resync_body() {
        use pm_engine::{
            PeerKind,
            engine::testing::{hello, sent},
        };
        use pm_protocol::{
            codec,
            color::ProjectColor,
            common::{Bytes, ContentHash, HostMicros, RequestId, Revision},
            envelope::{Envelope, Event, Message, OpenSession, Request, Response, SessionRole},
            host::PortId,
            patch::{
                DocumentEncoding, PatchBatch, PatchEncoding, PatchOrigin, Resync, ResyncSource,
            },
            version::PeerRole,
        };
        let mut k = NativeKernels::new(EngineProcess::Interactive);
        let mut e = engine();
        value(
            &mut k,
            &mut e,
            7,
            "resync",
            json!({"json":"{\"value\":7}","document":"doc","revision":3}),
        );
        let port = PortId(2);
        let now = HostMicros::new(0).unwrap();
        e.attach(port, PeerKind::Renderer);
        e.set_document_owner(port, Some(7));
        e.receive(port, &hello(PeerRole::Renderer), now);
        let open = Envelope::new(Message::Request {
            id: RequestId::new(1).unwrap(),
            body: Request::OpenSession(OpenSession {
                document: "doc".into(),
                role: SessionRole::Interactive,
                process: EngineProcess::Interactive,
                color: ProjectColor::DEFAULT,
            }),
        });
        let opened = e.receive(port, &codec::encode(&open).unwrap(), now);
        let sid = sent(&opened, port)
            .into_iter()
            .find_map(|answer| match answer.message {
                Message::Response {
                    body: Response::SessionOpened { session },
                    ..
                } => Some(session),
                _ => None,
            })
            .unwrap();
        let hash = ContentHash(
            k.slots[&7]
                .kernel
                .as_ref()
                .unwrap()
                .document()
                .canonical_digest()
                .unwrap(),
        );
        // The renderer still holds revision 3 plus this bounded journal when
        // a new viewer session opens. The native evaluator has already edited.
        let op = pm_model::ops::Op::from_value(
            &pm_model::json::parse(r#"{"path":["value"],"exists":true,"value":8}"#).unwrap(),
        )
        .unwrap();
        let body = pm_model::binary::encode(&[op]).unwrap();
        let applied: Json = serde_json::from_str(&k.call_bytes(
            &mut e,
            7,
            &json!({"op":"apply","args":{"base":3,"revision":4}}).to_string(),
            Some(&body),
        ))
        .unwrap();
        assert_eq!(applied["value"]["code"], 0);
        assert_eq!(k.slots[&7].kernel.as_ref().unwrap().cached_hash(), None);
        let mut resync = Envelope::new(Message::Request {
            id: RequestId::new(2).unwrap(),
            body: Request::Resync(Resync {
                document: "doc".into(),
                revision: Revision::new(3).unwrap(),
                encoding: DocumentEncoding::CanonicalJsonV1,
                source: ResyncSource::Inline {
                    body: Bytes(b"invalid body must not be parsed".to_vec()),
                },
                hash,
            }),
        });
        resync.session = Some(sid);
        let out = e.receive(port, &codec::encode(&resync).unwrap(), now);
        assert!(
            sent(&out, port).iter().any(|answer| matches!(
                answer.message,
                Message::Response {
                    body: Response::PatchesApplied(_),
                    ..
                }
            )),
            "{out:?}"
        );
        let status = e.status();
        assert_eq!(status.sessions[0].revision, 3);
        assert_eq!(
            status.sessions[0].hash.as_deref(),
            Some(pm_model::hash::hex(&hash.0).as_str())
        );
        let journal = Envelope::new(Message::Event {
            body: Event::ApplyPatches(PatchBatch {
                document: "doc".into(),
                base_revision: Revision::new(3).unwrap(),
                revision: Revision::new(4).unwrap(),
                origin: PatchOrigin::Edit,
                encoding: PatchEncoding::CanonicalV1,
                patch_count: 1,
                body: Bytes(body),
                provisional: false,
            }),
        })
        .with_session(sid);
        e.receive(port, &codec::encode(&journal).unwrap(), now);
        let status = e.status();
        assert_eq!(status.sessions[0].revision, 4);
        assert_eq!(
            status.sessions[0].hash.as_deref(),
            Some(pm_model::hash::sha256_hex(br#"{"value":8}"#).as_str())
        );
    }
}
