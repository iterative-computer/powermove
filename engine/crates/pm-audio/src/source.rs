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
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread::JoinHandle;

use pm_media::audio::{AudioDecoder, AudioStreamInfo};

use crate::error::{AudioError, Result};

/// Samples per channel in a block (about 1.4 s at 48 kHz).
pub const BLOCK: usize = 1 << 16;

/// One decodable stream of one file.
pub struct Source {
    id: u64,
    pub info: AudioStreamInfo,
    decoder: Mutex<Decoder>,
}

/// FFmpeg, or Symphonia for MP3 as today decodes it ([`crate::mp3`]).
enum Decoder {
    Ffmpeg(Box<AudioDecoder>),
    Mp3(Box<crate::mp3::Mp3Decoder>, u64),
    /// Electron's own FFmpeg ([`crate::chromium`]).
    Chromium(Box<crate::chromium::ChromiumDecoder>, u64),
}

impl Decoder {
    fn packets_decoded(&self) -> u64 {
        match self {
            Decoder::Ffmpeg(d) => d.packets_decoded(),
            Decoder::Mp3(_, n) | Decoder::Chromium(_, n) => *n,
        }
    }

    fn seek_anchored(&mut self, index: u64) -> Result<()> {
        match self {
            Decoder::Ffmpeg(d) => Ok(d.seek_anchored(index)?),
            Decoder::Mp3(d, _) => d.seek_anchored(index),
            Decoder::Chromium(d, _) => d.seek_anchored(index),
        }
    }

    fn read(&mut self, out: &mut [Vec<f32>], frames: usize) -> Result<usize> {
        match self {
            Decoder::Ffmpeg(d) => Ok(d.read(out, frames)?),
            Decoder::Mp3(d, n) => {
                let got = d.read(out, frames)?;
                *n += got.div_ceil(1152) as u64;
                Ok(got)
            }
            Decoder::Chromium(d, n) => {
                let got = d.read(out, frames)?;
                *n += got.div_ceil(1024) as u64;
                Ok(got)
            }
        }
    }
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
    /// Packets its decoder has sent to the codec (diagnostics: what the
    /// blocks decoded so far cost).
    pub fn packets_decoded(&self) -> u64 {
        lock(&self.decoder).packets_decoded()
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
/// A file and its FFmpeg stream index (the best audio stream when `None`).
type SourceKey = (PathBuf, Option<usize>);
type SourceIdentity = (u64, Option<(u64, Option<std::time::SystemTime>)>);

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
    /// Allocated only for opt-in diagnostics; follows a block through an
    /// ordinary worker or an offline synchronous decode.
    traces: HashMap<BlockKey, BlockTrace>,
}

#[derive(Default)]
struct BlockTrace {
    enqueued_us: u64,
    enqueue_thread: Option<String>,
    urgent: bool,
    queued_duplicates: u64,
    in_flight_duplicates: u64,
    last_duplicate_us: u64,
}

impl BlockTrace {
    fn enqueued(&mut self, urgent: bool) {
        self.enqueued_us = crate::clock::host_now_us();
        let thread = std::thread::current();
        self.enqueue_thread = thread
            .name()
            .filter(|name| name.starts_with("pm-audio-"))
            .map(str::to_owned);
        self.urgent = urgent;
    }

    fn queued_duplicate(&mut self) {
        self.queued_duplicates += 1;
        self.last_duplicate_us = crate::clock::host_now_us();
    }

    fn in_flight_duplicate(&mut self) {
        self.in_flight_duplicates += 1;
        self.last_duplicate_us = crate::clock::host_now_us();
    }
}

#[derive(Default)]
struct DecodeTrace {
    claim_us: u64,
    decoder_lock_us: u64,
    seek_done_us: u64,
    read_done_us: u64,
    cpu_start_ns: u64,
}

impl DecodeTrace {
    fn claimed(profile: bool, claim_us: u64) -> Self {
        if profile {
            Self {
                claim_us,
                cpu_start_ns: crate::realtime::thread_cpu_ns(),
                ..Self::default()
            }
        } else {
            Self::default()
        }
    }
}

fn log_decode(
    source: &Source,
    index: u64,
    origin: &'static str,
    queued: BlockTrace,
    trace: DecodeTrace,
    success: bool,
) {
    if trace.claim_us == 0 {
        return;
    }
    // Called after the cache mutex is released. This is an upper bound on
    // publication, including a worker descheduled after insertion.
    let published_us = crate::clock::host_now_us();
    let cpu_ns = crate::realtime::thread_cpu_ns().saturating_sub(trace.cpu_start_ns);
    let thread = std::thread::current();
    crate::profile::worker_event(format!(
        "pm-audio-source-block source={} block={} origin={} success={} enqueued_us={} enqueue_thread={} urgent={} claim_us={} decode_thread={} decoder_lock_us={} seek_done_us={} read_done_us={} published_us={} cpu_ns={} queued_duplicates={} in_flight_duplicates={} last_duplicate_us={}",
        source.id,
        index,
        origin,
        success,
        queued.enqueued_us,
        queued.enqueue_thread.as_deref().unwrap_or("external"),
        queued.urgent,
        trace.claim_us,
        thread
            .name()
            .filter(|name| name.starts_with("pm-audio-"))
            .unwrap_or("external"),
        trace.decoder_lock_us,
        trace.seek_done_us,
        trace.read_done_us,
        published_us,
        cpu_ns,
        queued.queued_duplicates,
        queued.in_flight_duplicates,
        queued.last_duplicate_us,
    ));
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
    sources: Mutex<HashMap<SourceKey, Weak<Source>>>,
    /// Sample identity survives a metadata-only Source being dropped. No file
    /// handles or PCM live here; a changed file receives a fresh block key.
    source_ids: Mutex<HashMap<SourceKey, SourceIdentity>>,

    blocks: Mutex<Blocks>,
    ready: Condvar,
    work: Condvar,
    budget: usize,
    next_id: AtomicU64,
    decoded: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    evicted: AtomicU64,
    /// Advances when a decoded block or a memoized decode error becomes
    /// visible. Missing-input preparation waits for progress, not a timer.
    publication_epoch: AtomicU64,
    stop: AtomicBool,
    workers: Mutex<Vec<JoinHandle<()>>>,
    profile: bool,
    #[cfg(feature = "test-hooks")]
    decode_delay_us: AtomicU64,
    #[cfg(feature = "test-hooks")]
    decode_gate: Mutex<HashSet<u64>>,
    #[cfg(feature = "test-hooks")]
    decode_gate_changed: Condvar,
    #[cfg(feature = "test-hooks")]
    gated_decoders: AtomicU64,
    /// Count today's decoded lengths before a source opens (an export's
    /// cache; [`SourceCache::set_blocking_lengths`]).
    blocking_lengths: AtomicBool,
    #[cfg(feature = "test-hooks")]
    decode_failing: Mutex<HashSet<u64>>,
}

/// A failed block's message as kept for later reads, which return it as a
/// decode error: a decode error keeps its own text, so every read of the
/// block reports the same message as the first.
fn memo(e: &AudioError) -> String {
    match e {
        AudioError::Media(pm_media::MediaError::Decode(m)) => m.clone(),
        other => other.to_string(),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl SourceCache {
    /// A cache holding at most about `budget_bytes` of decoded samples, with
    /// `workers` background decode threads for prefetch (0 for none: every
    /// block then decodes on the thread that asks for it).
    pub fn new(budget_bytes: usize, workers: usize) -> Arc<SourceCache> {
        let profile = crate::profile::enabled();
        if profile {
            crate::profile::init_worker_events();
        }
        let cache = Arc::new(SourceCache {
            sources: Mutex::default(),
            source_ids: Mutex::default(),
            blocks: Mutex::default(),
            ready: Condvar::new(),
            work: Condvar::new(),
            budget: budget_bytes
                .min(crate::chromium::TODAYS_BUDGET as usize)
                .max(BLOCK * 8 * 4),
            next_id: AtomicU64::new(1),
            decoded: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evicted: AtomicU64::new(0),
            publication_epoch: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            workers: Mutex::default(),
            profile,
            #[cfg(feature = "test-hooks")]
            decode_delay_us: AtomicU64::new(0),
            #[cfg(feature = "test-hooks")]
            decode_gate: Mutex::default(),
            #[cfg(feature = "test-hooks")]
            decode_gate_changed: Condvar::new(),
            #[cfg(feature = "test-hooks")]
            gated_decoders: AtomicU64::new(0),
            blocking_lengths: AtomicBool::new(false),
            #[cfg(feature = "test-hooks")]
            decode_failing: Mutex::default(),
        });
        cache.ensure_workers(workers);
        cache
    }

    /// Playback always reads nonblocking, even when its cache was created
    /// for offline use with no workers. Decoding must never own its thread.
    pub(crate) fn ensure_workers(self: &Arc<Self>, count: usize) {
        let mut handles = lock(&self.workers);
        for i in handles.len()..count {
            let weak = Arc::downgrade(self);
            handles.push(
                std::thread::Builder::new()
                    .name(format!("pm-audio-decode-{i}"))
                    .spawn(move || worker(weak))
                    .expect("spawn audio decode worker"),
            );
        }
    }

    /// Stops the decode workers (they also stop when the cache is dropped).
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.work.notify_all();
        #[cfg(feature = "test-hooks")]
        {
            let _gate = lock(&self.decode_gate);
            self.decode_gate_changed.notify_all();
        }
        for h in lock(&self.workers).drain(..) {
            let _ = h.join();
        }
    }

    /// Whether background decode workers run. Playback starts one if the
    /// cache was created without any; offline reads may decode inline.
    pub fn has_workers(&self) -> bool {
        !lock(&self.workers).is_empty()
    }

    /// Injects decoder latency for loaded-machine diagnostics. Applies to
    /// cache misses only, on both the calling thread and decode workers.
    #[cfg(feature = "test-hooks")]
    pub fn set_decode_delay(&self, delay: std::time::Duration) {
        self.decode_delay_us.store(
            delay.as_micros().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
    }

    /// Holds a source's decoder until explicitly released (a stuck-worker
    /// regression hook). Cache hits and other sources are unaffected.
    #[cfg(feature = "test-hooks")]
    pub fn set_decode_blocked(&self, source: &Source, blocked: bool) {
        let mut gate = lock(&self.decode_gate);
        if blocked {
            gate.insert(source.id);
        } else {
            gate.remove(&source.id);
        }
        self.decode_gate_changed.notify_all();
    }

    /// Makes every block of a source that is not decoded yet fail to
    /// decode (a corrupt-media regression hook). Failures are memoized like
    /// real decode errors.
    #[cfg(feature = "test-hooks")]
    pub fn set_decode_failing(&self, source: &Source, failing: bool) {
        let mut failing_ids = lock(&self.decode_failing);
        if failing {
            failing_ids.insert(source.id);
        } else {
            failing_ids.remove(&source.id);
        }
    }

    #[cfg(feature = "test-hooks")]
    pub fn blocked_decoders(&self) -> u64 {
        self.gated_decoders.load(Ordering::Acquire)
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

    pub(crate) fn publication_epoch(&self) -> u64 {
        self.publication_epoch.load(Ordering::Acquire)
    }

    /// Exports wait for today's decoded length of an AAC or Opus file when
    /// they open it (a job, off any interactive path); playback counts it in
    /// the background ([`crate::chromium::count_length`]).
    pub fn set_blocking_lengths(&self, blocking: bool) {
        self.blocking_lengths.store(blocking, Ordering::Relaxed);
    }

    /// Opens (or returns the open) audio stream of a file; `stream` is the
    /// FFmpeg stream index, the best audio stream when `None`.
    pub fn open(&self, path: &Path, stream: Option<usize>) -> Result<Arc<Source>> {
        let key = (path.to_owned(), stream);
        let fingerprint = std::fs::metadata(path)
            .ok()
            .map(|m| (m.len(), m.modified().ok()));
        let id = {
            let mut ids = lock(&self.source_ids);
            let entry = ids
                .entry(key.clone())
                .or_insert_with(|| (self.next_id.fetch_add(1, Ordering::Relaxed), fingerprint));
            if entry.1 != fingerprint {
                *entry = (self.next_id.fetch_add(1, Ordering::Relaxed), fingerprint);
            }
            entry.0
        };
        if let Some(s) = lock(&self.sources).get(&key).and_then(Weak::upgrade) {
            // A length counted since this source opened replaces it (the
            // next mix build takes the exact length).
            let stale = matches!(&*lock(&s.decoder), Decoder::Chromium(..))
                && crate::chromium::known_length(path).is_some_and(|n| s.info.length != Some(n));
            if s.id == id && !stale {
                return Ok(s.clone());
            }
        }
        let decoder = AudioDecoder::open(path, stream)?;
        let mut info = decoder.info().clone();
        // The FFmpeg probe gives the stream's facts (layout, length, priming);
        // MP3 samples come from today's decoder.
        let decoder = if info.codec == "mp3" {
            match crate::mp3::Mp3Decoder::open(path, info.channels()) {
                Ok(mp3) => Decoder::Mp3(Box::new(mp3), 0),
                Err(_) => Decoder::Ffmpeg(Box::new(decoder)),
            }
        } else if crate::chromium::decodes(&info.codec) && crate::chromium::plays_today(&info) {
            // Today's own FFmpeg where this process has it (the app's
            // engine processes); our FFmpeg elsewhere (the CLI, tests) and
            // for files longer than today plays at all.
            match crate::chromium::ChromiumDecoder::open(path, info.channels()) {
                Ok(Some(mut d)) => {
                    // Today's buffer length (its own priming and padding):
                    // counted once per file, off this thread unless the
                    // cache is an export's. Until then the container's.
                    let blocking = self.blocking_lengths.load(Ordering::Relaxed);
                    if let Some(frames) = crate::chromium::known_length(path)
                        .or_else(|| blocking.then(|| d.count_frames().ok()).flatten())
                    {
                        info.length = Some(frames);
                        info.priming = 0;
                        info.extra_priming = 0;
                    }
                    Decoder::Chromium(Box::new(d), 0)
                }
                _ => Decoder::Ffmpeg(Box::new(decoder)),
            }
        } else {
            Decoder::Ffmpeg(Box::new(decoder))
        };
        let source = Arc::new(Source {
            id,
            info,
            decoder: Mutex::new(decoder),
        });
        let mut sources = lock(&self.sources);
        sources.retain(|_, source| source.strong_count() > 0);
        if let Some(existing) = sources.get(&key).and_then(Weak::upgrade)
            && existing.id == source.id
            && existing.info.length == source.info.length
        {
            return Ok(existing.clone());
        }
        sources.insert(key, Arc::downgrade(&source));
        Ok(source)
    }

    /// Forgets open sources whose files changed or that no mix uses.
    pub fn close(&self, path: &Path) {
        let mut sources = lock(&self.sources);
        let gone: Vec<u64> = sources
            .iter()
            .filter(|((p, _), _)| p == path)
            .filter_map(|(_, s)| s.upgrade().map(|s| s.id))
            .collect();
        sources.retain(|(p, _), _| p != path);
        lock(&self.source_ids).retain(|(p, _), _| p != path);
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

    /// Release inactive decoded samples on host memory pressure. Blocks in
    /// an in-flight render remain pinned until that render releases them.
    /// Sources are weakly indexed: the last mix/read also closes its PCM spool.
    pub fn memory_pressure(&self) {
        let mut sources = lock(&self.sources);
        sources.retain(|_, source| source.strong_count() > 0);
        lock(&self.source_ids).retain(|key, _| sources.contains_key(key));
        drop(sources);
        let mut blocks = lock(&self.blocks);
        blocks
            .map
            .retain(|_, entry| Arc::strong_count(&entry.block) > 1);
        blocks.bytes = blocks.map.values().map(|entry| entry.block.bytes()).sum();
    }

    fn decode(&self, source: &Source, index: u64, trace: &mut DecodeTrace) -> Result<Block> {
        #[cfg(feature = "test-hooks")]
        {
            let mut gate = lock(&self.decode_gate);
            if gate.contains(&source.id) {
                self.gated_decoders.fetch_add(1, Ordering::Release);
                while gate.contains(&source.id) && !self.stop.load(Ordering::SeqCst) {
                    gate = self
                        .decode_gate_changed
                        .wait(gate)
                        .unwrap_or_else(|e| e.into_inner());
                }
                self.gated_decoders.fetch_sub(1, Ordering::Release);
            }
            drop(gate);
            let delay = self.decode_delay_us.load(Ordering::Relaxed);
            if delay != 0 {
                std::thread::sleep(std::time::Duration::from_micros(delay));
            }
            if lock(&self.decode_failing).contains(&source.id) {
                return Err(AudioError::Media(pm_media::MediaError::decode(format!(
                    "block {index}: injected decode failure"
                ))));
            }
        }
        let mut decoder = lock(&source.decoder);
        if self.profile {
            trace.decoder_lock_us = crate::clock::host_now_us();
        }
        let start = index * BLOCK as u64;
        decoder.seek_anchored(start)?;
        if self.profile {
            trace.seek_done_us = crate::clock::host_now_us();
        }
        let mut channels = vec![Vec::with_capacity(BLOCK); source.channels()];
        decoder.read(&mut channels, BLOCK)?;
        if self.profile {
            trace.read_done_us = crate::clock::host_now_us();
        }
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
                .filter(|(k, e)| **k != key && Arc::strong_count(&e.block) == 1)
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
        // A waiting caller can wake for unrelated blocks. Count its request
        // once, while still rechecking readiness after every notification.
        let mut counted_in_flight = false;
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
                if blocks.in_flight.contains(&key) {
                    if self.profile {
                        blocks.traces.entry(key).or_default().in_flight_duplicate();
                    }
                } else if blocks.queued.insert(key) {
                    blocks.queue.push_front((source.clone(), index));
                    if self.profile {
                        blocks.traces.entry(key).or_default().enqueued(true);
                    }
                    self.work.notify_one();
                } else if self.profile {
                    blocks.traces.entry(key).or_default().queued_duplicate();
                }
                return Ok(None);
            }
            if blocks.in_flight.contains(&key) {
                if self.profile && !counted_in_flight {
                    blocks.traces.entry(key).or_default().in_flight_duplicate();
                    counted_in_flight = true;
                }
                blocks = self.ready.wait(blocks).unwrap_or_else(|e| e.into_inner());
                continue;
            }
            self.misses.fetch_add(1, Ordering::Relaxed);
            blocks.in_flight.insert(key);
            drop(blocks);
            let mut trace = DecodeTrace::claimed(
                self.profile,
                self.profile.then(crate::clock::host_now_us).unwrap_or(0),
            );
            let decoded = self.decode(source, index, &mut trace);
            let mut b = lock(&self.blocks);
            b.in_flight.remove(&key);
            let queued = b.traces.remove(&key).unwrap_or_default();
            let success = decoded.is_ok();
            let out = match decoded {
                Ok(block) => {
                    self.decoded.fetch_add(1, Ordering::Relaxed);
                    let block = Arc::new(block);
                    self.insert(&mut b, key, block.clone());
                    Ok(Some(block))
                }
                Err(e) => {
                    b.errors.insert(key, memo(&e));
                    Err(e)
                }
            };
            self.publication_epoch.fetch_add(1, Ordering::Release);
            self.ready.notify_all();
            drop(b);
            if self.profile {
                let origin = if queued.enqueued_us != 0 {
                    "inline-stole-queued"
                } else {
                    "inline"
                };
                log_decode(source, index, origin, queued, trace, success);
            }
            return out;
        }
    }

    /// Queues blocks for the decode workers (in order of need).
    pub fn prefetch(&self, source: &Arc<Source>, indexes: impl IntoIterator<Item = u64>) {
        let mut blocks = lock(&self.blocks);
        let mut added = false;
        for index in indexes {
            let key = (source.id, index);
            if blocks.map.contains_key(&key) {
                continue;
            }
            if blocks.in_flight.contains(&key) {
                if self.profile {
                    blocks.traces.entry(key).or_default().in_flight_duplicate();
                }
                continue;
            }
            if blocks.errors.contains_key(&key) {
                continue;
            }
            if !blocks.queued.insert(key) {
                if self.profile {
                    blocks.traces.entry(key).or_default().queued_duplicate();
                }
                continue;
            }
            blocks.queue.push_back((source.clone(), index));
            if self.profile {
                blocks.traces.entry(key).or_default().enqueued(false);
            }
            added = true;
        }
        if added {
            self.work.notify_all();
        }
    }

    /// Queues the blocks covering program samples `[start, start + len)`
    /// (clamped to the program).
    pub fn prefetch_span(&self, source: &Arc<Source>, start: i64, len: usize) {
        if let Some((first, last)) = block_span(source, start, len) {
            self.prefetch(source, first..=last);
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
        self.read_valid(source, start, len, out, wait, None)
    }

    /// Like `read`, with validity from the blocks actually copied into
    /// scratch. Rechecking cache readiness afterwards races a decoder's
    /// arrival (or eviction) and can treat zero-filled scratch as decoded.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn read_valid(
        &self,
        source: &Arc<Source>,
        start: i64,
        len: usize,
        out: &mut [&mut [f32]],
        wait: bool,
        mut valid: Option<&mut [bool]>,
    ) -> Result<bool> {
        if let Some(valid) = valid.as_deref_mut() {
            valid[..len].fill(true); // Outside the program is valid silence.
        }
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
                if let Some(valid) = valid.as_deref_mut() {
                    let from = start.max((index * BLOCK as u64) as i64);
                    let to = (start + len as i64)
                        .min(((index + 1) * BLOCK as u64) as i64)
                        .min(source.length().unwrap_or(i64::MAX as u64) as i64);
                    valid[(from - start) as usize..(to - start) as usize].fill(false);
                }
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
                    let claim_us = if c.profile {
                        crate::clock::host_now_us()
                    } else {
                        0
                    };
                    break Some((source, index, claim_us));
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
        let Some((source, index, claim_us)) = job else {
            // Re-check the weak reference so a dropped cache ends the thread.
            drop(c);
            continue;
        };
        let key = (source.id, index);
        let mut trace = DecodeTrace::claimed(c.profile, claim_us);
        let decoded = c.decode(&source, index, &mut trace);
        let mut blocks = lock(&c.blocks);
        blocks.in_flight.remove(&key);
        let queued = blocks.traces.remove(&key).unwrap_or_default();
        let success = decoded.is_ok();
        match decoded {
            Ok(block) => {
                c.decoded.fetch_add(1, Ordering::Relaxed);
                c.insert(&mut blocks, key, Arc::new(block));
            }
            Err(e) => {
                blocks.errors.insert(key, memo(&e));
            }
        }
        c.publication_epoch.fetch_add(1, Ordering::Release);
        c.ready.notify_all();
        drop(blocks);
        if c.profile {
            log_decode(&source, index, "worker", queued, trace, success);
        }
    }
}

impl Drop for SourceCache {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.work.notify_all();
    }
}
