//! The decode service (section 13): sources opened once, sessions per source
//! and playback instance kept in a pool and reused across seeks, frames
//! selected by section 5 rule 7, a shared frame cache under a lease, and
//! read-ahead planned by direction, speed, and GOP.

use super::arbiter::{HardwareArbiter, LocalArbiter};
use super::session::Session;
use super::{Backend, DecoderInfo, StreamSpec, open};
use crate::cache::{CacheStats, FrameCache, FrameKey};
use crate::error::{MediaError, Result};
use crate::ff::{CodecInfo, Input, PacketReader};
use crate::frame::DecodedFrame;
use crate::index::FrameIndex;
use crate::pool::{FramePool, PoolStats};
use crate::probe::{MediaProbe, Probed};
use crate::time::{NominalRate, select_frame};
use crate::vt::DecodePurpose;
use pm_protocol::color::ColorDescription;
use pm_protocol::common::ByteSize;
use pm_protocol::lease::LeaseCategories;
use pm_time::{Rate, Tick};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(1);

/// A probed and indexed video source.
pub struct MediaSource {
    pub id: u64,
    pub probe: MediaProbe,
    pub stream: usize,
    pub index: Arc<FrameIndex>,
    pub info: CodecInfo,
    pub extradata: Vec<u8>,
    pub nominal: NominalRate,
    pub color: ColorDescription,
    /// Presented PTS in presentation order.
    pub pts: Vec<i64>,
}

impl MediaSource {
    /// Probes and indexes a file's best video stream.
    pub fn open(path: &Path) -> Result<Arc<MediaSource>> {
        let Probed { probe, index } = crate::probe::probe(path)?;
        let stream = probe
            .video_stream
            .ok_or_else(|| MediaError::invalid("no video stream"))?;
        let index = index.ok_or_else(|| MediaError::invalid("no video index"))?;
        let video = probe.video.as_ref().unwrap();
        let nominal = video.nominal_rate.clone().ok_or_else(|| {
            MediaError::invalid("the stream has no nominal rate (fewer than two frames)")
        })?;
        let input = Input::open(path)?;
        let pts = index.presented_pts();
        Ok(Arc::new(MediaSource {
            id: NEXT_SOURCE.fetch_add(1, Ordering::Relaxed),
            stream,
            info: video.codec.clone(),
            extradata: input.extradata(stream),
            nominal,
            color: video.color.interpreted,
            pts,
            index: Arc::new(index),
            probe,
        }))
    }

    pub fn rate(&self) -> Rate {
        self.nominal.rate
    }

    /// Section 5 rule 7: the presented frame showing at source time `t`
    /// (ticks, on the presentation timeline: PTS 0 is tick 0).
    pub fn select(&self, t: Tick) -> Option<usize> {
        select_frame(&self.pts, self.index.timebase, self.nominal.rate, t)
    }

    /// Source time of presented frame `n`, in ticks.
    pub fn time_of(&self, n: usize) -> Tick {
        crate::time::ticks_of(self.pts[n], self.index.timebase)
    }

    pub fn path(&self) -> &Path {
        &self.probe.path
    }
}

/// Frames around a presented frame, for deinterlacing.
#[derive(Clone, Debug)]
pub struct FieldFrames {
    pub prev: DecodedFrame,
    pub cur: DecodedFrame,
    pub next: DecodedFrame,
    /// No previous frame (the current one stands in).
    pub first: bool,
    /// No next frame (the current one stands in).
    pub last: bool,
}

/// Which playback or render instance a session serves (each viewer,
/// each export job). Sessions are pooled per source and instance.
pub type InstanceId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SessionKey {
    source: u64,
    instance: InstanceId,
    purpose: DecodePurpose,
}

/// Configuration.
#[derive(Clone, Debug)]
pub struct ServiceConfig {
    /// Frame cache budget (the lease's frame-cache share).
    pub cache_bytes: usize,
    /// Free pool memory kept for reuse.
    pub pool_free_bytes: usize,
    /// Software decoder threads.
    pub threads: i32,
    /// Idle sessions kept per source and instance.
    pub idle_sessions: usize,
    /// Frames each session keeps besides the cache.
    pub session_window: usize,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        ServiceConfig {
            cache_bytes: 512 << 20,
            pool_free_bytes: 256 << 20,
            threads: 4,
            idle_sessions: 2,
            session_window: super::session::DEFAULT_WINDOW,
        }
    }
}

/// The service's shares of the engine's lease (contract 8 categories).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaLease {
    /// The frame cache budget.
    pub frame_cache: usize,
    /// Decoded memory outside the frame cache: session windows, decoder
    /// references, frames renderers still hold, and free pool slots.
    pub decoders: usize,
}

impl MediaLease {
    pub const UNLIMITED: MediaLease = MediaLease {
        frame_cache: usize::MAX,
        decoders: usize::MAX,
    };
}

/// Memory the service holds, for the engine's lease report (contract 8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaUsage {
    /// Frames in the frame cache (pool and CoreVideo frames).
    pub frame_cache: usize,
    /// Every other decoded frame the process holds, each counted once: pool
    /// slots the cache does not hold (decoder references, session windows,
    /// frames a renderer still uses after eviction, free slots) and
    /// VideoToolbox's CoreVideo buffers the cache does not hold (session
    /// windows, frames renderers hold). VideoToolbox's internal buffer pools
    /// and session state are not visible here.
    pub decoders: usize,
    pub cache: CacheStats,
    pub pool: PoolStats,
    pub sessions: usize,
}

impl MediaUsage {
    /// The service's share of a lease usage report (contract 8): the frame
    /// cache and decoder categories. `None` beyond the protocol's safe
    /// integer range.
    pub fn lease_categories(&self) -> Option<LeaseCategories> {
        Some(LeaseCategories {
            frame_cache: ByteSize::new(self.frame_cache as u64).ok()?,
            decoders: ByteSize::new(self.decoders as u64).ok()?,
            ..LeaseCategories::default()
        })
    }
}

/// Playback direction for read-ahead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Reverse,
}

/// What playback is about to need.
#[derive(Clone, Copy, Debug)]
pub struct ReadAhead {
    pub direction: Direction,
    /// Source frames advanced per output frame (1 for real time, 2 for 2×,
    /// 0.5 for half speed).
    pub speed: f64,
    /// Output frames to prepare.
    pub frames: usize,
}

pub struct DecodeService {
    pool: FramePool,
    cache: Arc<FrameCache>,
    arbiter: Arc<dyn HardwareArbiter>,
    config: ServiceConfig,
    idle: Mutex<HashMap<SessionKey, Vec<Session>>>,
    open_sessions: AtomicU64,
    /// The lease's decoder share ([`MediaLease::decoders`]).
    decoder_budget: AtomicUsize,
}

/// A session checked out of the pool; it returns on drop.
pub struct SessionGuard<'a> {
    service: &'a DecodeService,
    key: SessionKey,
    source: Arc<MediaSource>,
    session: Option<Session>,
}

impl SessionGuard<'_> {
    pub fn info(&self) -> DecoderInfo {
        self.session.as_ref().unwrap().decoder_info()
    }

    pub fn stats(&self) -> super::session::SessionStats {
        self.session.as_ref().unwrap().stats.clone()
    }

    /// Presented frame `n`, through the cache.
    pub fn frame(&mut self, n: usize) -> Result<DecodedFrame> {
        let sample = self
            .source
            .index
            .frames
            .get(n)
            .ok_or_else(|| MediaError::invalid(format!("frame {n} out of range")))?
            .sample;
        let key = FrameKey {
            source: self.source.id,
            sample,
            purpose: self.key.purpose,
        };
        if let Some(f) = self.service.cache.get(&key) {
            return Ok(f);
        }
        let s = self.session.as_mut().unwrap();
        let f = s.sample(sample)?;
        // Everything the session decoded on the way is cached too.
        for (k, fr) in s.window_frames() {
            let key = FrameKey {
                source: self.source.id,
                sample: k,
                purpose: self.key.purpose,
            };
            if !self.service.cache.contains(&key) {
                self.service.cache.insert(key, fr.clone());
            }
        }
        // Over the decoder share (a shrunken lease), the window gives back
        // the frames the cache could not keep.
        if self.service.decoders_over() {
            s.clear_window();
        }
        Ok(f)
    }

    /// Rule 7 selection then decode; `None` before the first frame.
    pub fn frame_at(&mut self, t: Tick) -> Result<Option<DecodedFrame>> {
        match self.source.select(t) {
            Some(n) => self.frame(n).map(Some),
            None => Ok(None),
        }
    }

    /// The previous, current, and next presented frames around `n`, for
    /// deinterlacing (random access includes neighboring fields). At the
    /// ends the current frame stands in for the missing neighbor, and
    /// `first`/`last` say so (bwdif then interpolates that field spatially).
    pub fn fields(&mut self, n: usize) -> Result<FieldFrames> {
        let count = self.source.index.frames.len();
        if n >= count {
            return Err(MediaError::invalid(format!("frame {n} out of range")));
        }
        let prev = if n > 0 {
            Some(self.frame(n - 1)?)
        } else {
            None
        };
        let cur = self.frame(n)?;
        let next = if n + 1 < count {
            Some(self.frame(n + 1)?)
        } else {
            None
        };
        Ok(FieldFrames {
            first: prev.is_none(),
            last: next.is_none(),
            prev: prev.unwrap_or_else(|| cur.clone()),
            next: next.unwrap_or_else(|| cur.clone()),
            cur,
        })
    }

    /// Decodes the read-ahead plan from presented frame `from` into the
    /// cache. Returns the frames decoded (cache hits excluded).
    pub fn read_ahead(&mut self, from: usize, plan: ReadAhead) -> Result<usize> {
        let mut decoded = 0;
        for n in read_ahead_order(&self.source, from, plan) {
            let sample = self.source.index.frames[n].sample;
            let key = FrameKey {
                source: self.source.id,
                sample,
                purpose: self.key.purpose,
            };
            if !self.service.cache.contains(&key) {
                self.frame(n)?;
                decoded += 1;
            }
        }
        Ok(decoded)
    }
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        if let Some(mut s) = self.session.take() {
            let svc = self.service;
            // Over the decoder share, the session returns without its window,
            // and closes when that is still not enough (always for a zero
            // share, as in `fit_decoders`).
            let zero = svc.decoder_budget.load(Ordering::Relaxed) == 0;
            if zero || svc.decoders_over() {
                s.clear_window();
                if zero || svc.decoders_over() {
                    drop(s);
                    svc.open_sessions.fetch_sub(1, Ordering::Relaxed);
                    svc.fit_decoders();
                    return;
                }
            }
            let mut idle = svc.idle.lock().unwrap();
            let list = idle.entry(self.key).or_default();
            if list.len() < svc.config.idle_sessions {
                list.push(s);
            } else {
                svc.open_sessions.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}

impl DecodeService {
    pub fn new(config: ServiceConfig) -> DecodeService {
        Self::with_arbiter(config, Arc::new(LocalArbiter::unlimited()))
    }

    pub fn with_arbiter(config: ServiceConfig, arbiter: Arc<dyn HardwareArbiter>) -> DecodeService {
        DecodeService {
            pool: FramePool::new(config.pool_free_bytes),
            cache: Arc::new(FrameCache::new(config.cache_bytes)),
            arbiter,
            config,
            idle: Mutex::new(HashMap::new()),
            open_sessions: AtomicU64::new(0),
            decoder_budget: AtomicUsize::new(usize::MAX),
        }
    }

    pub fn pool(&self) -> &FramePool {
        &self.pool
    }

    pub fn cache(&self) -> &Arc<FrameCache> {
        &self.cache
    }

    /// A session for `source` serving `instance`: an idle one when the pool
    /// has it (reused across seeks), otherwise a new one.
    pub fn session(
        &self,
        source: &Arc<MediaSource>,
        instance: InstanceId,
        purpose: DecodePurpose,
    ) -> Result<SessionGuard<'_>> {
        self.session_with(source, instance, purpose, None)
    }

    /// As [`session`](DecodeService::session) with a backend preference.
    pub fn session_with(
        &self,
        source: &Arc<MediaSource>,
        instance: InstanceId,
        purpose: DecodePurpose,
        prefer: Option<Backend>,
    ) -> Result<SessionGuard<'_>> {
        let key = SessionKey {
            source: source.id,
            instance,
            purpose,
        };
        let reused = self.idle.lock().unwrap().get_mut(&key).and_then(|l| {
            let at = l
                .iter()
                .position(|s| prefer.is_none_or(|p| s.decoder_info().backend == p))?;
            Some(l.remove(at))
        });
        let session = match reused {
            Some(s) => s,
            None => {
                let input = Input::open(source.path())?;
                let spec = StreamSpec {
                    input: &input,
                    stream: source.stream,
                    info: &source.info,
                    index: &source.index,
                    extradata: &source.extradata,
                };
                let (decoder, _skipped) = open(
                    &spec,
                    purpose,
                    &self.pool,
                    self.arbiter.as_ref(),
                    self.config.threads,
                    prefer,
                )?;
                let reader = PacketReader::new(source.path(), source.stream)?;
                let mut s = Session::new(source.index.clone(), reader, decoder, Some(source.color));
                s.set_window(self.config.session_window);
                self.open_sessions.fetch_add(1, Ordering::Relaxed);
                s
            }
        };
        Ok(SessionGuard {
            service: self,
            key,
            source: source.clone(),
            session: Some(session),
        })
    }

    /// Closes every idle session of an instance (a viewer closed, an export
    /// finished) and of sources no longer open when `source` is given.
    pub fn release(&self, instance: Option<InstanceId>, source: Option<u64>) {
        let mut idle = self.idle.lock().unwrap();
        idle.retain(|k, v| {
            let drop =
                instance.is_none_or(|i| k.instance == i) && source.is_none_or(|s| k.source == s);
            if drop {
                self.open_sessions
                    .fetch_sub(v.len() as u64, Ordering::Relaxed);
            }
            !drop
        });
        if let Some(s) = source {
            self.cache.remove_source(s);
        }
    }

    /// Applies a lease change and enforces it: the frame cache evicts down
    /// to its share; over the decoder share, idle sessions drop their
    /// windows, free pool memory is released, and idle sessions close until
    /// usage fits (all of them for a zero share). Sessions in use give their windows back at their next
    /// request and close on return while usage is still over. Returns the
    /// usage afterwards (the acknowledgement; frames renderers still hold
    /// count until they let go).
    pub fn apply_lease(&self, lease: MediaLease) -> MediaUsage {
        self.cache.set_budget(lease.frame_cache);
        self.decoder_budget.store(lease.decoders, Ordering::Relaxed);
        self.fit_decoders();
        self.usage()
    }

    /// Whether decoder memory exceeds the lease's decoder share.
    fn decoders_over(&self) -> bool {
        let budget = self.decoder_budget.load(Ordering::Relaxed);
        budget != usize::MAX && self.usage().decoders > budget
    }

    /// Brings decoder memory within the decoder share, cheapest first.
    fn fit_decoders(&self) {
        let mut idle = self.idle.lock().unwrap();
        if self.decoders_over() {
            for s in idle.values_mut().flatten() {
                s.clear_window();
            }
        }
        self.trim_free();
        // A zero share closes every idle session: the state VideoToolbox
        // keeps per session is not measurable, so nothing else frees it.
        let zero = self.decoder_budget.load(Ordering::Relaxed) == 0;
        while zero || self.decoders_over() {
            let Some(s) = idle.values_mut().find_map(|l| l.pop()) else {
                break;
            };
            drop(s);
            self.open_sessions.fetch_sub(1, Ordering::Relaxed);
            self.trim_free();
        }
        idle.retain(|_, l| !l.is_empty());
    }

    /// Keeps free pool memory within the configured amount and what the
    /// decoder share leaves after live memory.
    fn trim_free(&self) {
        let budget = self.decoder_budget.load(Ordering::Relaxed);
        let u = self.usage();
        let live = u.decoders - u.pool.free_bytes.min(u.decoders);
        self.pool
            .trim(self.config.pool_free_bytes.min(budget.saturating_sub(live)));
    }

    pub fn usage(&self) -> MediaUsage {
        let cache = self.cache.stats();
        let pool = self.pool.stats();
        // Each frame counts once: in the cache when the cache holds it,
        // otherwise here (pool slots and CoreVideo buffers alike).
        let cache_corevideo = cache.bytes - cache.pool_bytes;
        MediaUsage {
            frame_cache: cache.bytes,
            decoders: pool.bytes.saturating_sub(cache.pool_bytes)
                + pool.external_bytes.saturating_sub(cache_corevideo),
            cache,
            pool,
            sessions: self.open_sessions.load(Ordering::Relaxed) as usize,
        }
    }
}

/// Presented frames read-ahead needs from `from`, in the order that decodes
/// each GOP once: forward in presentation order; reverse GOP by GOP from the
/// newest, each GOP forward (a reverse step then finds its frame cached).
pub fn read_ahead_order(source: &MediaSource, from: usize, plan: ReadAhead) -> Vec<usize> {
    let n = source.pts.len();
    if n == 0 || plan.frames == 0 {
        return vec![];
    }
    let mut wanted: Vec<usize> = Vec::with_capacity(plan.frames);
    let t0 = source.time_of(from.min(n - 1));
    let frame = source.rate().frame_start(1).unwrap_or(1) as f64;
    for i in 1..=plan.frames {
        let dt = (i as f64 * plan.speed.abs() * frame).round() as Tick;
        let t = match plan.direction {
            Direction::Forward => t0.saturating_add(dt),
            Direction::Reverse => t0.saturating_sub(dt),
        };
        if let Some(k) = source.select(t)
            && wanted.last() != Some(&k)
            && k != from
        {
            wanted.push(k);
        }
        if plan.direction == Direction::Reverse && source.select(t).is_none() {
            break;
        }
    }
    match plan.direction {
        Direction::Forward => wanted,
        Direction::Reverse => {
            // Group by the sync sample each frame decodes from.
            let gop_of = |k: usize| source.index.sync_for(source.index.frames[k].sample);
            let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
            for k in wanted {
                let g = gop_of(k);
                match groups.iter_mut().find(|(s, _)| *s == g) {
                    Some((_, v)) => v.push(k),
                    None => groups.push((g, vec![k])),
                }
            }
            groups.sort_by_key(|(g, _)| std::cmp::Reverse(*g));
            groups
                .into_iter()
                .flat_map(|(_, mut v)| {
                    v.sort_unstable();
                    v
                })
                .collect()
        }
    }
}
