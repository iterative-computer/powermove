//! A durable session folder: the host's side of libfx's journal (header,
//! "Durable sessions"). Appends are stored in order, bound to the head they
//! expect, and acknowledged only once on disk; one writer holds the folder at
//! a time (an exclusive lock that a killed process gives up); checkpoints are
//! stored only at the head they cover; images live under their names.
//!
//! Layout: `events` (one line per append: `first_seq last_seq json`),
//! `checkpoint` (8-byte little-endian seq, then the snapshot), `images/`,
//! `writer.lock`.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use crate::{JournalAppend, Remote};

/// Why an append was not stored.
#[derive(Debug)]
pub enum StoreError {
    /// Another writer holds the folder, or the append is not the next one.
    Fenced,
    Io(io::Error),
}

impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        StoreError::Io(error)
    }
}

/// What a folder holds, for [`crate::OpenArgs`].
#[derive(Debug, Default)]
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
    events: Mutex<(File, u64)>,
}

impl SessionFolder {
    /// Opens (or makes) `dir` for writing. `Fenced` if another writer holds it.
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
        let last_seq = Self::load(dir)?.last_seq;
        let events = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("events"))?;
        Ok(SessionFolder {
            dir: dir.to_path_buf(),
            _lock: lock,
            events: Mutex::new((events, last_seq)),
        })
    }

    /// Stores `append` durably, after the last one stored.
    pub fn append(
        &self,
        first_seq: u64,
        last_seq: u64,
        events_json: &str,
    ) -> Result<(), StoreError> {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if first_seq != events.1 + 1 || last_seq < first_seq {
            return Err(StoreError::Fenced);
        }
        let line = format!("{first_seq} {last_seq} {}\n", events_json.trim());
        events.0.write_all(line.as_bytes())?;
        events.0.sync_data()?;
        events.1 = last_seq;
        Ok(())
    }

    /// Stores a checkpoint covering every event through `at_seq`, only if
    /// that is the last event stored. True if stored.
    pub fn save_checkpoint(&self, at_seq: u64, bytes: &[u8]) -> io::Result<bool> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if at_seq != events.1 {
            return Ok(false);
        }
        let temp = self.dir.join("checkpoint.new");
        let mut file = File::create(&temp)?;
        file.write_all(&at_seq.to_le_bytes())?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, self.dir.join("checkpoint"))?;
        File::open(&self.dir)?.sync_all()?;
        Ok(true)
    }

    /// Stores an image under its name before it is handed in.
    pub fn put_image(&self, bytes: &[u8]) -> io::Result<String> {
        let name = crate::image_name(bytes);
        let path = self.dir.join("images").join(&name);
        if !path.exists() {
            let temp = path.with_extension("new");
            let mut file = File::create(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            std::fs::rename(&temp, &path)?;
        }
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

    /// What `dir` holds: the newest checkpoint and the events after it. A
    /// line a crash left torn (no newline) was never acknowledged; it is
    /// left out.
    pub fn load(dir: &Path) -> io::Result<Stored> {
        let mut stored = Stored::default();
        let mut at_seq = 0u64;
        if let Ok(mut file) = File::open(dir.join("checkpoint")) {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            if bytes.len() > 8 {
                at_seq = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                stored.checkpoint = bytes[8..].to_vec();
                stored.last_seq = at_seq;
            }
        }
        let mut parts = Vec::new();
        if let Ok(file) = File::open(dir.join("events")) {
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line)? == 0 || !line.ends_with('\n') {
                    break;
                }
                let mut fields = line.trim_end().splitn(3, ' ');
                let (Some(first), Some(last), Some(json)) =
                    (fields.next(), fields.next(), fields.next())
                else {
                    break;
                };
                let (Ok(first), Ok(last)) = (first.parse::<u64>(), last.parse::<u64>()) else {
                    break;
                };
                stored.last_seq = stored.last_seq.max(last);
                if first <= at_seq {
                    continue;
                }
                let inner = json
                    .trim()
                    .strip_prefix('[')
                    .and_then(|rest| rest.strip_suffix(']'))
                    .unwrap_or("");
                if !inner.trim().is_empty() {
                    parts.push(inner.to_string());
                }
            }
        }
        stored.journal_json = if parts.is_empty() {
            String::new()
        } else {
            format!("[{}]", parts.join(","))
        };
        Ok(stored)
    }
}

/// Stores a runtime's appends on a thread of its own and acknowledges each
/// once durable (or fails it): [`crate::Host::journal_append`] only enqueues.
pub struct JournalWriter {
    queue: Mutex<Option<Sender<(u64, u64, u64, String)>>>,
    remote: Arc<OnceLock<Remote>>,
    folder: Arc<SessionFolder>,
}

impl JournalWriter {
    pub fn start(folder: SessionFolder) -> io::Result<Arc<JournalWriter>> {
        let (send, receive) = channel::<(u64, u64, u64, String)>();
        let remote: Arc<OnceLock<Remote>> = Arc::new(OnceLock::new());
        let folder = Arc::new(folder);
        let (bound, store) = (remote.clone(), folder.clone());
        std::thread::Builder::new()
            .name("aero-journal".into())
            .spawn(move || {
                for (append, first, last, json) in receive {
                    // Appends start after the session opens, after bind.
                    let remote = bound.wait();
                    let _ = match store.append(first, last, &json) {
                        Ok(()) => remote.journal_ack(append),
                        Err(StoreError::Fenced) => remote.journal_fail(append, "", true),
                        Err(StoreError::Io(error)) => {
                            remote.journal_fail(append, &error.to_string(), false)
                        }
                    };
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
