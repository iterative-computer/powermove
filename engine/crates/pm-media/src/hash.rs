//! Content identity (section 13 probe item 6): a full BLAKE3 hash computed in
//! the background, with today's sampled fingerprint kept as a lookup hint.
//! Files still being written are hashed only once their size and
//! modification time stop changing; a file that changes while it is hashed
//! is hashed again.

use crate::error::{MediaError, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

const SAMPLE_BYTES: u64 = 64 * 1024;

/// The renderer's import fingerprint (`legacy/core/media.ts`,
/// `main/media-path-cache.ts`): SHA-256 over a header and three 64 KiB
/// samples (start, middle, end). It can collide (appendix A item 22), so it
/// is only a hint for finding a file's full hash.
pub fn sampled_fingerprint(path: &Path) -> Result<String> {
    let f = File::open(path)?;
    let size = f.metadata()?.len();
    let sample = SAMPLE_BYTES.min(size);
    let mut offsets: Vec<u64> = Vec::with_capacity(3);
    for o in [0, (size - sample) / 2, size - sample] {
        if !offsets.contains(&o) {
            offsets.push(o);
        }
    }
    let mut h = Sha256::new();
    h.update(format!("powermove-media-v2\n{size}\n").as_bytes());
    let mut buf = vec![0u8; sample as usize];
    for o in offsets {
        f.read_exact_at(&mut buf, o)?;
        h.update(&buf);
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("v2:{size}:{hex}"))
}

/// A file's full content hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentHash {
    #[serde(serialize_with = "hex32")]
    pub blake3: [u8; 32],
    pub size: u64,
}

fn hex32<S: serde::Serializer>(b: &[u8; 32], s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

impl ContentHash {
    pub fn hex(&self) -> String {
        self.blake3.iter().map(|x| format!("{x:02x}")).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "state")]
pub enum HashStatus {
    /// Waiting for the file to stop changing.
    Waiting,
    Hashing { done: u64, total: u64 },
    Done { hash: ContentHash },
    Failed { message: String },
    Cancelled,
}

/// How long the file's size and modification time must stay unchanged
/// before hashing starts, and how often to look.
#[derive(Clone, Copy, Debug)]
pub struct Settle {
    pub quiet: Duration,
    pub poll: Duration,
}

impl Default for Settle {
    fn default() -> Self {
        Settle { quiet: Duration::from_secs(2), poll: Duration::from_millis(250) }
    }
}

fn stamp(path: &Path) -> Result<(u64, SystemTime)> {
    let m = std::fs::metadata(path)?;
    Ok((m.len(), m.modified()?))
}

/// Hashes a file on the calling thread, waiting for it to settle first.
/// `progress` receives bytes hashed so far; `cancel` stops the work.
pub fn hash_file(path: &Path, settle: Settle, cancel: &AtomicBool, progress: &dyn Fn(HashStatus)) -> Result<ContentHash> {
    loop {
        // Settle: the same size and mtime for `quiet`.
        let mut last = stamp(path)?;
        let mut quiet_since = std::time::Instant::now();
        let mut waiting_reported = false;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
            let age = last.1.elapsed().unwrap_or(Duration::ZERO);
            if age >= settle.quiet || quiet_since.elapsed() >= settle.quiet {
                break;
            }
            if !waiting_reported {
                progress(HashStatus::Waiting);
                waiting_reported = true;
            }
            std::thread::sleep(settle.poll);
            let now = stamp(path)?;
            if now != last {
                last = now;
                quiet_since = std::time::Instant::now();
            }
        }
        let (size, _) = last;
        let mut f = File::open(path)?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut done = 0u64;
        let mut tick = 0u32;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            done += n as u64;
            tick += 1;
            if tick.is_multiple_of(64) {
                progress(HashStatus::Hashing { done, total: size });
            }
        }
        // Changed while hashing: start over.
        if stamp(path)? != last || done != size {
            continue;
        }
        return Ok(ContentHash { blake3: *hasher.finalize().as_bytes(), size });
    }
}

/// What a full hash says about a file whose fingerprint was looked up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum Resolution {
    /// The content was known under this fingerprint.
    Known,
    /// New content. `collided_with` lists different content that shares the
    /// fingerprint (appendix A item 22): matching on the fingerprint alone
    /// would have picked the wrong file.
    New { collided_with: Vec<ContentHash> },
}

/// Known content by sampled fingerprint. The fingerprint is cheap (three
/// 64 KiB reads), so it finds candidates while the full hash runs in the
/// background; only the full hash decides identity (relinking, deduplicating
/// imports, cache keys).
#[derive(Clone, Debug, Default)]
pub struct ContentHints {
    by_fingerprint: std::collections::HashMap<String, Vec<ContentHash>>,
}

impl ContentHints {
    /// Content previously seen with this fingerprint: a hint, not an identity.
    pub fn candidates(&self, fingerprint: &str) -> &[ContentHash] {
        self.by_fingerprint.get(fingerprint).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Records a file's full hash under its fingerprint and says whether the
    /// content was already known there.
    pub fn resolve(&mut self, fingerprint: &str, hash: ContentHash) -> Resolution {
        let known = self.by_fingerprint.entry(fingerprint.to_string()).or_default();
        if known.contains(&hash) {
            return Resolution::Known;
        }
        let collided_with = known.clone();
        known.push(hash);
        Resolution::New { collided_with }
    }
}

/// A background hash request.
#[derive(Clone)]
pub struct HashTicket {
    state: Arc<(Mutex<HashStatus>, Condvar)>,
    cancel: Arc<AtomicBool>,
    pub path: PathBuf,
}

impl HashTicket {
    pub fn status(&self) -> HashStatus {
        self.state.0.lock().unwrap().clone()
    }

    /// Waits for the result.
    pub fn wait(&self) -> Result<ContentHash> {
        let mut g = self.state.0.lock().unwrap();
        loop {
            match &*g {
                HashStatus::Done { hash } => return Ok(*hash),
                HashStatus::Failed { message } => return Err(MediaError::invalid(message)),
                HashStatus::Cancelled => return Err(MediaError::Cancelled),
                _ => g = self.state.1.wait(g).unwrap(),
            }
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Hashes files on one background thread at utility QoS, one at a time.
pub struct BackgroundHasher {
    queue: Arc<(Mutex<Vec<HashTicket>>, Condvar)>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    pub settle: Settle,
    hashed_bytes: Arc<AtomicU64>,
}

impl BackgroundHasher {
    pub fn new(settle: Settle) -> BackgroundHasher {
        let queue: Arc<(Mutex<Vec<HashTicket>>, Condvar)> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let hashed_bytes = Arc::new(AtomicU64::new(0));
        let (q, s, hb) = (queue.clone(), stop.clone(), hashed_bytes.clone());
        let worker = std::thread::Builder::new()
            .name("pm-media-hash".into())
            .spawn(move || {
                unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) };
                loop {
                    let job = {
                        let mut g = q.0.lock().unwrap();
                        loop {
                            if s.load(Ordering::Relaxed) {
                                return;
                            }
                            if !g.is_empty() {
                                break g.remove(0);
                            }
                            g = q.1.wait(g).unwrap();
                        }
                    };
                    let set = |st: HashStatus| {
                        *job.state.0.lock().unwrap() = st;
                        job.state.1.notify_all();
                    };
                    let r = hash_file(&job.path, settle, &job.cancel, &|st| set(st));
                    match r {
                        Ok(hash) => {
                            hb.fetch_add(hash.size, Ordering::Relaxed);
                            set(HashStatus::Done { hash })
                        }
                        Err(MediaError::Cancelled) => set(HashStatus::Cancelled),
                        Err(e) => set(HashStatus::Failed { message: e.to_string() }),
                    }
                }
            })
            .expect("spawn hash thread");
        BackgroundHasher { queue, stop, worker: Some(worker), settle, hashed_bytes }
    }

    /// Queues a file; the ticket reports progress and the result.
    pub fn request(&self, path: &Path) -> HashTicket {
        let t = HashTicket {
            state: Arc::new((Mutex::new(HashStatus::Waiting), Condvar::new())),
            cancel: Arc::new(AtomicBool::new(false)),
            path: path.to_path_buf(),
        };
        self.queue.0.lock().unwrap().push(t.clone());
        self.queue.1.notify_all();
        t
    }

    pub fn hashed_bytes(&self) -> u64 {
        self.hashed_bytes.load(Ordering::Relaxed)
    }
}

impl Drop for BackgroundHasher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.queue.0.lock().unwrap().drain(..) {
            t.cancel();
            *t.state.0.lock().unwrap() = HashStatus::Cancelled;
            t.state.1.notify_all();
        }
        self.queue.1.notify_all();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pm-media-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn fingerprint_matches_the_typescript_algorithm() {
        // Small file: one sample of the whole file, offsets deduplicated.
        let p = tmp("small.bin", b"hello");
        let mut h = Sha256::new();
        h.update(b"powermove-media-v2\n5\nhello");
        let want: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(sampled_fingerprint(&p).unwrap(), format!("v2:5:{want}"));
        // Large file: start, middle, end samples.
        let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let p = tmp("large.bin", &data);
        let mut h = Sha256::new();
        h.update(b"powermove-media-v2\n300000\n");
        let mid = (300_000 - 65_536) / 2;
        h.update(&data[..65_536]);
        h.update(&data[mid..mid + 65_536]);
        h.update(&data[300_000 - 65_536..]);
        let want: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(sampled_fingerprint(&p).unwrap(), format!("v2:300000:{want}"));
    }

    #[test]
    fn fingerprints_hint_and_full_hashes_decide() {
        // Same size and the same three sampled windows, different bytes
        // between them: the fingerprints collide.
        let mut a: Vec<u8> = (0..400_000u32).map(|i| (i % 249) as u8).collect();
        let pa = tmp("collide-a.bin", &a);
        a[100_000] ^= 0xff;
        let pb = tmp("collide-b.bin", &a);
        let (fa, fb) = (sampled_fingerprint(&pa).unwrap(), sampled_fingerprint(&pb).unwrap());
        assert_eq!(fa, fb);
        let never = AtomicBool::new(false);
        let settle = Settle { quiet: Duration::ZERO, poll: Duration::from_millis(1) };
        let ha = hash_file(&pa, settle, &never, &|_| {}).unwrap();
        let hb = hash_file(&pb, settle, &never, &|_| {}).unwrap();
        assert_ne!(ha, hb);
        let mut hints = ContentHints::default();
        assert!(hints.candidates(&fa).is_empty());
        assert_eq!(hints.resolve(&fa, ha), Resolution::New { collided_with: vec![] });
        // B looks like A by fingerprint; the full hash says it is not.
        assert_eq!(hints.candidates(&fb), &[ha]);
        assert_eq!(hints.resolve(&fb, hb), Resolution::New { collided_with: vec![ha] });
        assert_eq!(hints.resolve(&fa, ha), Resolution::Known);
        assert_eq!(hints.candidates(&fa), &[ha, hb]);
    }

    #[test]
    fn background_hash_waits_for_a_settled_file() {
        let data = vec![7u8; 3 << 20];
        let p = tmp("bg.bin", &data);
        let hasher = BackgroundHasher::new(Settle { quiet: Duration::from_millis(300), poll: Duration::from_millis(20) });
        let t = hasher.request(&p);
        // Keep writing for a moment: the hash must cover the final bytes.
        std::thread::sleep(Duration::from_millis(100));
        let mut more = data.clone();
        more.extend_from_slice(b"tail");
        std::fs::write(&p, &more).unwrap();
        let h = t.wait().unwrap();
        assert_eq!(h.size, more.len() as u64);
        assert_eq!(h.blake3, *blake3::hash(&more).as_bytes());
        let c = hasher.request(&p);
        c.cancel();
        assert!(matches!(c.wait(), Err(MediaError::Cancelled) | Ok(_)));
    }
}
