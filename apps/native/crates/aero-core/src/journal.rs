//! A durable session folder: the host's side of libfx's journal (header,
//! "Durable sessions"). Appends are stored in order, bound to the head they
//! expect, and acknowledged only once on disk; one writer holds the folder at
//! a time (an exclusive lock that a killed process gives up); checkpoints are
//! stored only at the head the core was told is durable; images live under
//! their names.
//!
//! Layout: `events` (one line per append: `first_seq last_seq json`),
//! `checkpoint` (8-byte little-endian seq, then the snapshot), `images/`,
//! `writer.lock`.
//!
//! Recovery (Sol's m2 items 1 and 3). Opening for writing reads the
//! checkpoint and the events under the writer lock, checks every whole
//! record (a JSON array of the events `first_seq..=last_seq`, consecutive
//! with the record before) and cuts a torn last line (a crash mid-write
//! never acknowledged it) before anything is appended. A whole record that
//! fails the check is corruption: the folder is refused, never repaired. A
//! write that may have reached the disk in part (a failed write or sync)
//! puts the folder out of use until it is opened again, so nothing is ever
//! appended after an uncertain record. Every head the store acknowledges
//! reopens: an append that would make the journal after the checkpoint
//! larger than a reopen loads ([`MAX_TAIL_BYTES`]) is refused.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::{Error, JournalAppend, Outcome, Remote};

/// The most journal after the checkpoint `fx_session_open` loads (the
/// header's 4 MiB snapshot bound), as [`Stored::journal_json`] spells it.
pub const MAX_TAIL_BYTES: usize = 4 << 20;

/// Why an append was not stored.
#[derive(Debug)]
pub enum StoreError {
    /// Another writer holds the folder, or the append is not the next one.
    Fenced,
    /// The journal after the checkpoint would outgrow what a reopen loads:
    /// take a checkpoint (or reopen) first.
    TooLarge,
    /// A whole record on disk is not a valid one: the folder is not opened.
    Corrupt(String),
    Io(io::Error),
}

impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        StoreError::Io(error)
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Fenced => f.write_str("another writer holds this session's journal"),
            StoreError::TooLarge => write!(
                f,
                "the session's journal after its checkpoint would exceed the {} MiB a reopen loads",
                MAX_TAIL_BYTES >> 20
            ),
            StoreError::Corrupt(why) => write!(f, "the session's journal is corrupt: {why}"),
            StoreError::Io(error) => write!(f, "{error}"),
        }
    }
}

/// What a folder holds, for [`crate::OpenArgs`].
#[derive(Debug, Default, Clone)]
pub struct Stored {
    /// The events after the checkpoint, a JSON array.
    pub journal_json: String,
    pub checkpoint: Vec<u8>,
    /// The seq of the last event stored.
    pub last_seq: u64,
}

/// One session's folder, held for writing.
pub struct SessionFolder {
    dir: PathBuf,
    _lock: File,
    head: Mutex<Head>,
    stored: Mutex<Option<Stored>>,
}

/// The writer's view of `events`, guarded by the folder's mutex.
struct Head {
    events: File,
    /// Bytes of `events` that hold whole, checked records.
    len: u64,
    /// The seq of the last event on disk.
    disk_seq: u64,
    /// The seq of the last event the core was told is durable.
    acked_seq: u64,
    /// The journal after the checkpoint, as [`Stored::journal_json`] would
    /// spell it: its length and its record count.
    tail_bytes: usize,
    tail_records: usize,
    /// A write that may have reached the disk in part: no append follows it.
    broken: Option<String>,
}

/// What a scan of a folder found.
struct Scan {
    stored: Stored,
    /// Bytes of `events` up to the end of the last whole record.
    valid_len: u64,
    /// Whether bytes after it (a torn line) were found.
    torn: bool,
    tail_records: usize,
}

impl SessionFolder {
    /// Opens (or makes) `dir` for writing. `Fenced` if another writer holds
    /// it; `Corrupt` if a whole record is not valid. Under the writer lock it
    /// reads what the folder holds ([`SessionFolder::take_stored`]) and cuts
    /// a torn last line before any append.
    pub fn open(dir: &Path) -> Result<SessionFolder, StoreError> {
        std::fs::create_dir_all(dir.join("images"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("writer.lock"))?;
        // SAFETY: a valid descriptor; flock has no other preconditions.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(StoreError::Fenced);
        }
        let scan = scan(dir).map_err(|error| match error.kind() {
            io::ErrorKind::InvalidData => StoreError::Corrupt(error.to_string()),
            _ => StoreError::Io(error),
        })?;
        let events = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(dir.join("events"))?;
        if scan.torn {
            events.set_len(scan.valid_len)?;
            events.sync_all()?;
        }
        let tail_bytes = scan.stored.journal_json.len();
        let head = Head {
            events,
            len: scan.valid_len,
            disk_seq: scan.stored.last_seq,
            acked_seq: scan.stored.last_seq,
            tail_bytes,
            tail_records: scan.tail_records,
            broken: None,
        };
        Ok(SessionFolder {
            dir: dir.to_path_buf(),
            _lock: lock,
            head: Mutex::new(head),
            stored: Mutex::new(Some(scan.stored)),
        })
    }

    /// What the folder held when it was opened (read under the writer
    /// lock), for [`crate::OpenArgs`]; empty after the first call.
    pub fn take_stored(&self) -> Stored {
        self.stored
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_default()
    }

    /// Stores `append` durably, after the last one stored. Any error but
    /// `Fenced` and `TooLarge` means the folder is out of use: reopen it.
    pub fn append(
        &self,
        first_seq: u64,
        last_seq: u64,
        events_json: &str,
    ) -> Result<(), StoreError> {
        let mut head = self.head.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(why) = &head.broken {
            return Err(StoreError::Io(io::Error::other(format!(
                "an earlier write may have reached the disk in part ({why}): reopen the folder"
            ))));
        }
        if first_seq != head.disk_seq + 1 || last_seq < first_seq {
            return Err(StoreError::Fenced);
        }
        let json = events_json.trim();
        // What this record adds to the journal a reopen loads.
        let (tail_bytes, tail_records) = match tail_part(json) {
            None => (head.tail_bytes, head.tail_records),
            Some(inner) if head.tail_records == 0 => (inner.len() + 2, 1),
            Some(inner) => (head.tail_bytes + inner.len() + 1, head.tail_records + 1),
        };
        if tail_bytes > MAX_TAIL_BYTES {
            return Err(StoreError::TooLarge);
        }
        let line = format!("{first_seq} {last_seq} {json}\n");
        let written = head
            .events
            .write_all(line.as_bytes())
            .and_then(|()| head.events.sync_data());
        if let Err(error) = written {
            // The record may be on disk in part: cut it back if the disk
            // lets us, and append nothing more either way.
            let len = head.len;
            let _ = head
                .events
                .set_len(len)
                .and_then(|()| head.events.sync_data());
            head.broken = Some(error.to_string());
            return Err(StoreError::Io(error));
        }
        head.len += line.len() as u64;
        head.disk_seq = last_seq;
        head.tail_bytes = tail_bytes;
        head.tail_records = tail_records;
        Ok(())
    }

    /// The appends through `last_seq` are acknowledged to the core: that
    /// head may now carry a checkpoint. Returns the head before, for
    /// [`SessionFolder::unmark_acked`] if the core did not take the ack.
    pub fn mark_acked(&self, last_seq: u64) -> u64 {
        let mut head = self.head.lock().unwrap_or_else(|e| e.into_inner());
        let before = head.acked_seq;
        head.acked_seq = head.acked_seq.max(last_seq.min(head.disk_seq));
        before
    }

    /// The core never took the ack [`SessionFolder::mark_acked`] marked.
    pub fn unmark_acked(&self, before: u64) {
        let mut head = self.head.lock().unwrap_or_else(|e| e.into_inner());
        head.acked_seq = head.acked_seq.min(before);
    }

    /// Stores a checkpoint covering every event through `at_seq`, only if
    /// that is the last event stored and the core has been told it is
    /// durable (header, "Checkpoints"). True if stored.
    pub fn save_checkpoint(&self, at_seq: u64, bytes: &[u8]) -> io::Result<bool> {
        let mut head = self.head.lock().unwrap_or_else(|e| e.into_inner());
        if at_seq != head.disk_seq || at_seq != head.acked_seq || head.broken.is_some() {
            return Ok(false);
        }
        let temp = self.dir.join("checkpoint.new");
        let mut file = File::create(&temp)?;
        file.write_all(&at_seq.to_le_bytes())?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, self.dir.join("checkpoint"))?;
        File::open(&self.dir)?.sync_all()?;
        head.tail_bytes = 0;
        head.tail_records = 0;
        Ok(true)
    }

    /// Stores an image under its name before it is handed in. Each call
    /// writes its own temporary file, so a concurrent or interrupted writer
    /// of the same image never touches a file once it is published; the
    /// name is durable (the folder synced) when this returns.
    pub fn put_image(&self, bytes: &[u8]) -> io::Result<String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = crate::image_name(bytes);
        let images = self.dir.join("images");
        let path = images.join(&name);
        if !path.exists() {
            let temp = images.join(format!(
                ".{name}.{}.{}.new",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let written = (|| {
                let mut file = File::create(&temp)?;
                file.write_all(bytes)?;
                file.sync_all()?;
                std::fs::rename(&temp, &path)
            })();
            if let Err(error) = written {
                let _ = std::fs::remove_file(&temp);
                return Err(error);
            }
        }
        // Published by this call or another: its directory entry is on disk
        // before the journal names it.
        File::open(&images)?.sync_all()?;
        Ok(name)
    }

    /// An image the folder holds.
    pub fn image(dir: &Path, name: &str) -> io::Result<Vec<u8>> {
        if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not an image name",
            ));
        }
        std::fs::read(dir.join("images").join(name))
    }

    /// What `dir` holds, read without the writer lock (for inspection; a
    /// writer opens with [`SessionFolder::open`] and
    /// [`SessionFolder::take_stored`]): the newest checkpoint and the events
    /// after it. A torn last line (a crash mid-write) was never acknowledged
    /// and is left out; a whole record that is not valid is `InvalidData`.
    pub fn load(dir: &Path) -> io::Result<Stored> {
        Ok(scan(dir)?.stored)
    }
}

/// A record's events without the array's brackets, or None for an empty
/// array (it adds nothing to the journal a reopen loads).
fn tail_part(json: &str) -> Option<&str> {
    let inner = json
        .trim()
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or("");
    (!inner.trim().is_empty()).then_some(inner)
}

fn corrupt(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

/// Reads `dir`'s checkpoint and events and checks every whole record.
fn scan(dir: &Path) -> io::Result<Scan> {
    let mut stored = Stored::default();
    let mut at_seq = 0u64;
    match File::open(dir.join("checkpoint")) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            if bytes.len() <= 8 {
                return Err(corrupt("the checkpoint is shorter than its header".into()));
            }
            at_seq = u64::from_le_bytes(bytes[..8].try_into().unwrap());
            stored.checkpoint = bytes[8..].to_vec();
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let bytes = match std::fs::read(dir.join("events")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let mut parts: Vec<&str> = Vec::new();
    let mut last: Option<u64> = None;
    let mut valid_len = 0usize;
    let mut number = 0usize;
    while let Some(end) = bytes[valid_len..].iter().position(|&b| b == b'\n') {
        number += 1;
        let raw = &bytes[valid_len..valid_len + end];
        let (first, last_seq, json) =
            record(raw, last).map_err(|why| corrupt(format!("record {number}: {why}")))?;
        last = Some(last_seq);
        if first > at_seq {
            if let Some(inner) = tail_part(json) {
                parts.push(inner);
            }
        } else if last_seq > at_seq {
            return Err(corrupt(format!(
                "record {number} straddles the checkpoint at seq {at_seq}"
            )));
        }
        valid_len += end + 1;
    }
    let disk_seq = last.unwrap_or(0);
    if last.is_some() && at_seq > disk_seq {
        return Err(corrupt(format!(
            "the checkpoint covers seq {at_seq}, past the journal's last seq {disk_seq}"
        )));
    }
    stored.last_seq = disk_seq.max(at_seq);
    let tail_records = parts.len();
    stored.journal_json = if parts.is_empty() {
        String::new()
    } else {
        format!("[{}]", parts.join(","))
    };
    Ok(Scan {
        stored,
        valid_len: valid_len as u64,
        torn: valid_len < bytes.len(),
        tail_records,
    })
}

/// Checks one whole record: `first last json`, UTF-8, consecutive with the
/// record before (`previous`'s last seq), and its JSON an array of exactly
/// the events `first..=last`, each with its `seq`.
fn record(raw: &[u8], previous: Option<u64>) -> Result<(u64, u64, &str), String> {
    let line = std::str::from_utf8(raw).map_err(|_| "not UTF-8".to_string())?;
    let mut fields = line.splitn(3, ' ');
    let (Some(first), Some(last), Some(json)) = (fields.next(), fields.next(), fields.next())
    else {
        return Err("not `first last json`".into());
    };
    let (Ok(first), Ok(last)) = (first.parse::<u64>(), last.parse::<u64>()) else {
        return Err("its seqs are not numbers".into());
    };
    if first == 0 || last < first {
        return Err(format!("seqs {first}..{last}"));
    }
    if let Some(previous) = previous
        && first != previous + 1
    {
        return Err(format!("starts at seq {first} after seq {previous}"));
    }
    let events: serde_json::Value =
        serde_json::from_str(json).map_err(|error| format!("its events are not JSON ({error})"))?;
    let events = events
        .as_array()
        .ok_or_else(|| "its events are not an array".to_string())?;
    if events.len() as u64 != last - first + 1 {
        return Err(format!("{} events for seqs {first}..{last}", events.len()));
    }
    for (offset, event) in events.iter().enumerate() {
        let seq = event.get("seq").and_then(serde_json::Value::as_u64);
        if seq != Some(first + offset as u64) {
            return Err(format!("event {offset} has seq {seq:?}"));
        }
    }
    Ok((first, last, json))
}

/// An append copied for the writer thread: id, first and last seq, events.
type Queued = (u64, u64, u64, String);

/// Stores a runtime's appends on a thread of its own and acknowledges each
/// once durable (or fails it): [`crate::Host::journal_append`] only enqueues.
/// An ack or failure the core could not take now (FX_BUSY, FX_E_OOM) is kept
/// and sent again, never re-stored (Sol's m2 item 2).
pub struct JournalWriter {
    queue: Mutex<Option<Sender<Queued>>>,
    remote: Arc<OnceLock<Remote>>,
    folder: Arc<SessionFolder>,
}

/// Sends `call` until the core takes it or can never take it: a retryable
/// answer (busy, out of memory) is retried after a short wait. Returns the
/// last answer.
pub(crate) fn deliver(mut call: impl FnMut() -> Result<Outcome, Error>) -> Result<Outcome, Error> {
    let mut wait = Duration::from_micros(200);
    loop {
        match call() {
            Ok(Outcome::Busy) | Err(Error::Busy) | Err(Error::OutOfMemory) => {
                std::thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_millis(50));
            }
            other => return other,
        }
    }
}

impl JournalWriter {
    pub fn start(folder: SessionFolder) -> io::Result<Arc<JournalWriter>> {
        let (send, receive) = channel::<Queued>();
        let remote: Arc<OnceLock<Remote>> = Arc::new(OnceLock::new());
        let folder = Arc::new(folder);
        let (bound, store) = (remote.clone(), folder.clone());
        std::thread::Builder::new()
            .name("aero-journal".into())
            .spawn(move || {
                for (append, first, last, json) in receive {
                    // Appends start after the session opens, after bind.
                    let remote = bound.wait();
                    match store.append(first, last, &json) {
                        Ok(()) => {
                            // Marked first, so a checkpoint the host asks for
                            // as soon as the core has the ack finds its head.
                            let before = store.mark_acked(last);
                            let taken = deliver(|| remote.journal_ack(append));
                            if !matches!(taken, Ok(Outcome::Applied) | Ok(Outcome::Stale)) {
                                store.unmark_acked(before);
                            }
                        }
                        Err(StoreError::Fenced) => {
                            let _ = deliver(|| remote.journal_fail(append, "", true));
                        }
                        Err(error) => {
                            let reason = error.to_string();
                            let _ = deliver(|| remote.journal_fail(append, &reason, false));
                        }
                    }
                }
            })?;
        Ok(Arc::new(JournalWriter {
            queue: Mutex::new(Some(send)),
            remote,
            folder,
        }))
    }

    /// Names the runtime the acks go to; call right after create, before the
    /// session opens (no append comes earlier).
    pub fn bind(&self, remote: Remote) {
        let _ = self.remote.set(remote);
    }

    /// Copies the append and queues it for the writer thread.
    pub fn enqueue(&self, append: &JournalAppend<'_>) {
        if let Some(queue) = self
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = queue.send((
                append.append,
                append.first_seq,
                append.last_seq,
                append.events_json.to_string(),
            ));
        }
    }

    pub fn folder(&self) -> &SessionFolder {
        &self.folder
    }

    /// Ends the writer thread once the queue is stored.
    pub fn stop(&self) {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}
