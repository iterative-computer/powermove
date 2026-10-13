//! Decoded sources: program samples in fixed blocks, decoded on demand and
//! ahead of playback, kept under a byte budget.
//!
//! A block always decodes from its own anchor ([`AudioDecoder::seek_anchored`]),
//! so its samples depend only on the file and the block index: a render is
//! the same whatever was rendered before it, and the preview equals the
//! export. Nothing holds a whole file; there is no length limit.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

use pm_media::audio::{AudioDecoder, AudioStreamInfo};

use crate::error::{AudioError, Result};

/// Samples per channel in a block (about 1.4 s at 48 kHz).
pub const BLOCK: usize = 1 << 16;

/// One decodable stream of one file.
pub struct Source {
    id: u64,
    pub info: AudioStreamInfo,
    decoder: Mutex<AudioDecoder>,
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Source")
            .field("id", &self.id)
            .field("info", &self.info)
            .finish()
    }
}

impl Source {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn channels(&self) -> usize {
        self.info.channels()
    }
    pub fn sample_rate(&self) -> u32 {
        self.info.sample_rate
    }
    /// Program length in samples (metadata; see `AudioStreamInfo::length`).
    pub fn length(&self) -> Option<u64> {
        self.info.length
    }
}

/// Decoded samples `[index × BLOCK, index × BLOCK + len)`, per channel.
#[derive(Debug)]
pub struct Block {
    pub channels: Vec<Vec<f32>>,
}

impl Block {
    pub fn len(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn bytes(&self) -> usize {
        self.channels
            .iter()
            .map(|c| c.capacity() * 4)
            .sum::<usize>()
            + 64
    }
}

type BlockKey = (u64, u64);

struct Entry {
    block: Arc<Block>,
    used: u64,
}

#[derive(Default)]
struct Blocks {
    map: HashMap<BlockKey, Entry>,
    bytes: usize,
    clock: u64,
    in_flight: HashSet<BlockKey>,
    queue: VecDeque<(Arc<Source>, u64)>,
    queued: HashSet<BlockKey>,
    errors: HashMap<BlockKey, String>,
}

/// Counters for diagnostics and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub decoded_blocks: u64,
    pub hits: u64,
    pub misses: u64,
    pub evicted_blocks: u64,
    pub bytes: usize,
}

/// Shared by every mixer and render in one process.
pub struct SourceCache {
    sources: Mutex<HashMap<(PathBuf, Option<usize>), Arc<Source>>>,
    blocks: Mutex<Blocks>,
    ready: Condvar,
    work: Condvar,
    budget: usize,
    next_id: AtomicU64,
    decoded: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    evicted: AtomicU64,
    stop: AtomicBool,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl SourceCache {
    /// A cache holding at most about `budget_bytes` of decoded samples, with
    /// `workers` background decode threads for prefetch (0 for none: every
    /// block then decodes on the thread that asks for it).
    pub fn new(budget_bytes: usize, workers: usize) -> Arc<SourceCache> {
        let cache = Arc::new(SourceCache {
            sources: Mutex::default(),
            blocks: Mutex::default(),
            ready: Condvar::new(),
            work: Condvar::new(),
            budget: budget_bytes.max(BLOCK * 8 * 4),
            next_id: AtomicU64::new(1),
            decoded: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evicted: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            workers: Mutex::default(),
        });
        let mut handles = lock(&cache.workers);
        for i in 0..workers {
            let weak = Arc::downgrade(&cache);
            handles.push(
                std::thread::Builder::new()
                    .name(format!("pm-audio-decode-{i}"))
                    .spawn(move || worker(weak))
                    .expect("spawn audio decode worker"),
            );
        }
        drop(handles);
        cache
    }

    /// Stops the decode workers (they also stop when the cache is dropped).
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.work.notify_all();
        for h in lock(&self.workers).drain(..) {
            let _ = h.join();
        }
    }

    /// Whether background decode workers run (without them, playback
    /// decodes on its render thread).
    pub fn has_workers(&self) -> bool {
        !lock(&self.workers).is_empty()
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            decoded_blocks: self.decoded.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evicted_blocks: self.evicted.load(Ordering::Relaxed),
            bytes: lock(&self.blocks).bytes,
        }
    }

    /// Opens (or returns the open) audio stream of a file; `stream` is the
    /// FFmpeg stream index, the best audio stream when `None`.
    pub fn open(&self, path: &Path, stream: Option<usize>) -> Result<Arc<Source>> {
        let key = (path.to_owned(), stream);
        if let Some(s) = lock(&self.sources).get(&key) {
            return Ok(s.clone());
        }
        let decoder = AudioDecoder::open(path, stream)?;
        let source = Arc::new(Source {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            info: decoder.info().clone(),
            decoder: Mutex::new(decoder),
        });
        Ok(lock(&self.sources).entry(key).or_insert(source).clone())
    }

    /// Forgets open sources whose files changed or that no mix uses.
    pub fn close(&self, path: &Path) {
        let mut sources = lock(&self.sources);
        let gone: Vec<u64> = sources
            .iter()
            .filter(|((p, _), _)| p == path)
            .map(|(_, s)| s.id)
            .collect();
        sources.retain(|(p, _), _| p != path);
        drop(sources);
        let mut blocks = lock(&self.blocks);
        let keys: Vec<BlockKey> = blocks
            .map
            .keys()
            .filter(|k| gone.contains(&k.0))
            .copied()
            .collect();
        for k in keys {
            if let Some(e) = blocks.map.remove(&k) {
                blocks.bytes -= e.block.bytes();
            }
        }
    }

    fn decode(source: &Source, index: u64) -> Result<Block> {
        let mut decoder = lock(&source.decoder);
        let start = index * BLOCK as u64;
        decoder.seek_anchored(start)?;
        let mut channels = vec![Vec::with_capacity(BLOCK); source.channels()];
        decoder.read(&mut channels, BLOCK)?;
        Ok(Block { channels })
    }

    fn insert(&self, blocks: &mut Blocks, key: BlockKey, block: Arc<Block>) {
        blocks.clock += 1;
        let used = blocks.clock;
        blocks.bytes += block.bytes();
        if let Some(old) = blocks.map.insert(key, Entry { block, used }) {
            blocks.bytes -= old.block.bytes();
        }
        while blocks.bytes > self.budget && blocks.map.len() > 1 {
            let Some((&oldest, _)) = blocks
                .map
                .iter()
                .filter(|(k, _)| **k != key)
                .min_by_key(|(_, e)| e.used)
            else {
                break;
            };
            if let Some(e) = blocks.map.remove(&oldest) {
                blocks.bytes -= e.block.bytes();
                self.evicted.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Block `index` of `source`. With `wait`, decodes it on this thread (or
    /// waits for the worker decoding it); without, returns `None` when it is
    /// not decoded yet and queues it.
    pub fn block(
        &self,
        source: &Arc<Source>,
        index: u64,
        wait: bool,
    ) -> Result<Option<Arc<Block>>> {
        let key = (source.id, index);
        let mut blocks = lock(&self.blocks);
        loop {
            blocks.clock += 1;
            let now = blocks.clock;
            if let Some(e) = blocks.map.get_mut(&key) {
                e.used = now;
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Ok(Some(e.block.clone()));
            }
            if let Some(err) = blocks.errors.get(&key) {
                return Err(AudioError::Media(pm_media::MediaError::decode(err.clone())));
            }
            if !wait {
                self.misses.fetch_add(1, Ordering::Relaxed);
                if !blocks.in_flight.contains(&key) && blocks.queued.insert(key) {
                    blocks.queue.push_front((source.clone(), index));
                    self.work.notify_one();
                }
                return Ok(None);
            }
            if blocks.in_flight.contains(&key) {
                blocks = self.ready.wait(blocks).unwrap_or_else(|e| e.into_inner());
                continue;
            }
            self.misses.fetch_add(1, Ordering::Relaxed);
            blocks.in_flight.insert(key);
            drop(blocks);
            let decoded = Self::decode(source, index);
            let mut b = lock(&self.blocks);
            b.in_flight.remove(&key);
            let out = match decoded {
                Ok(block) => {
                    self.decoded.fetch_add(1, Ordering::Relaxed);
                    let block = Arc::new(block);
                    self.insert(&mut b, key, block.clone());
                    Ok(Some(block))
                }
                Err(e) => {
                    b.errors.insert(key, e.to_string());
                    Err(e)
                }
            };
            self.ready.notify_all();
            return out;
        }
    }

    /// Queues blocks for the decode workers (in order of need).
    pub fn prefetch(&self, source: &Arc<Source>, indexes: impl IntoIterator<Item = u64>) {
        let mut blocks = lock(&self.blocks);
        let mut added = false;
        for index in indexes {
            let key = (source.id, index);
            if blocks.map.contains_key(&key)
                || blocks.in_flight.contains(&key)
                || blocks.errors.contains_key(&key)
                || !blocks.queued.insert(key)
            {
                continue;
            }
            blocks.queue.push_back((source.clone(), index));
            added = true;
        }
        if added {
            self.work.notify_all();
        }
    }

    /// Whether every block covering samples `[start, start + len)` is decoded.
    pub fn is_ready(&self, source: &Source, start: i64, len: usize) -> bool {
        let Some((first, last)) = block_span(source, start, len) else {
            return true;
        };
        let blocks = lock(&self.blocks);
        (first..=last).all(|i| {
            blocks.map.contains_key(&(source.id, i)) || blocks.errors.contains_key(&(source.id, i))
        })
    }

    /// Copies program samples `[start, start + len)` into `out` (one slice
    /// per channel, `len` long). Samples outside the program are zero, and
    /// so are samples of blocks not yet decoded when `wait` is false.
    /// Returns false when something was missing.
    pub fn read(
        &self,
        source: &Arc<Source>,
        start: i64,
        len: usize,
        out: &mut [&mut [f32]],
        wait: bool,
    ) -> Result<bool> {
        for c in out.iter_mut() {
            c[..len].fill(0.0);
        }
        let Some((first, last)) = block_span(source, start, len) else {
            return Ok(true);
        };
        let mut complete = true;
        for index in first..=last {
            let Some(block) = self.block(source, index, wait)? else {
                complete = false;
                continue;
            };
            let block_start = (index * BLOCK as u64) as i64;
            let from = start.max(block_start);
            let to = (start + len as i64).min(block_start + block.len() as i64);
            if to <= from {
                continue;
            }
            let (src_off, dst_off, n) = (
                (from - block_start) as usize,
                (from - start) as usize,
                (to - from) as usize,
            );
            for (dst, src) in out.iter_mut().zip(&block.channels) {
                dst[dst_off..dst_off + n].copy_from_slice(&src[src_off..src_off + n]);
            }
        }
        Ok(complete)
    }
}

/// The blocks covering `[start, start + len)` clipped to the program.
fn block_span(source: &Source, start: i64, len: usize) -> Option<(u64, u64)> {
    let end = start + len as i64;
    let lo = start.max(0);
    let hi = match source.length() {
        Some(n) => end.min(n as i64),
        None => end,
    };
    if hi <= lo {
        return None;
    }
    Some((lo as u64 / BLOCK as u64, (hi - 1) as u64 / BLOCK as u64))
}

fn worker(cache: std::sync::Weak<SourceCache>) {
    loop {
        let Some(c) = cache.upgrade() else { return };
        let job = {
            let mut blocks = lock(&c.blocks);
            loop {
                if c.stop.load(Ordering::SeqCst) {
                    return;
                }
                if let Some((source, index)) = blocks.queue.pop_front() {
                    let key = (source.id, index);
                    blocks.queued.remove(&key);
                    if blocks.map.contains_key(&key) || blocks.in_flight.contains(&key) {
                        continue;
                    }
                    blocks.in_flight.insert(key);
                    break Some((source, index));
                }
                let (b, timeout) = c
                    .work
                    .wait_timeout(blocks, std::time::Duration::from_millis(200))
                    .unwrap_or_else(|e| e.into_inner());
                blocks = b;
                if timeout.timed_out() && blocks.queue.is_empty() {
                    break None;
                }
            }
        };
        let Some((source, index)) = job else {
            // Re-check the weak reference so a dropped cache ends the thread.
            drop(c);
            continue;
        };
        let key = (source.id, index);
        let decoded = SourceCache::decode(&source, index);
        let mut blocks = lock(&c.blocks);
        blocks.in_flight.remove(&key);
        match decoded {
            Ok(block) => {
                c.decoded.fetch_add(1, Ordering::Relaxed);
                c.insert(&mut blocks, key, Arc::new(block));
            }
            Err(e) => {
                blocks.errors.insert(key, e.to_string());
            }
        }
        c.ready.notify_all();
    }
}

impl Drop for SourceCache {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.work.notify_all();
    }
}
