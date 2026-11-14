//! The decode service (section 13): sources opened once, sessions per source
//! and playback instance kept in a pool and reused across seeks, frames
//! selected by section 5 rule 7, a shared frame cache under a lease, and
//! read-ahead planned by direction, speed, and GOP.

use super::arbiter::{HardwareArbiter, LocalArbiter};
use super::session::Session;
use super::{Backend, DecoderInfo, StreamSpec, open};
use crate::cache::{CacheStats, FrameCache, FrameKey};
use crate::clip::{ClipTiming, LegacyMap, Span};
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
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// The probe's already-open FFmpeg context serves the first decoder.
    startup_input: Arc<Mutex<Option<Input>>>,
    pub nominal: NominalRate,
    pub color: ColorDescription,
    /// Presented timestamps in presentation order on the clip's timeline
    /// ([`ClipTiming::Presentation`]): 0 is the start of the file's
    /// presentation ([`Span`]). Index timebase units.
    pub pts: Vec<i64>,
    /// The container timestamp of clip time 0, in index timebase units.
    pub origin_pts: i64,
    /// Where the presentation ends on the clip's timeline, in ticks.
    end: Tick,
    /// Where the clip ends as an older asset reads the file, in ticks.
    legacy_end: Tick,
    /// The same frames as older projects address them
    /// ([`ClipTiming::Legacy`]).
    pub legacy: LegacyMap,
}

impl MediaSource {
    /// Prepare a stable file's content-hash-keyed probe and frame index at
    /// import time. The first [`open`](Self::open) can reuse that work; no
    /// decoded or converted frame is retained by this cache.
    pub fn prewarm(path: &Path) -> Result<()> {
        crate::probe::cache::prewarm(path)
    }

    /// Interruptible import preparation for a renderer background worker.
    pub fn prewarm_cancelled(path: &Path, cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
        crate::probe::cache::prewarm_cancelled(path, cancel)
    }

    /// Probes and indexes a file's best video stream.
    pub fn open(path: &Path) -> Result<Arc<MediaSource>> {
        let prepared = crate::probe::cache::lookup(path)?;
        let (probe, index, extradata, startup_input) = match prepared {
            Some(prepared) => (
                prepared.probe,
                prepared.index,
                prepared.extradata,
                prepared.input,
            ),
            None => {
                let (Probed { probe, index }, input) = crate::probe::probe_with_input(path)?;
                let index = Arc::new(index.ok_or_else(|| MediaError::invalid("no video index"))?);
                let stream = probe
                    .video_stream
                    .ok_or_else(|| MediaError::invalid("no video stream"))?;
                let extradata = input.extradata(stream);
                (probe, index, extradata, Arc::new(Mutex::new(Some(input))))
            }
        };
        let stream = probe
            .video_stream
            .ok_or_else(|| MediaError::invalid("no video stream"))?;
        let video = probe.video.as_ref().unwrap();
        let nominal = match video.nominal_rate.clone() {
            Some(nominal) => nominal,
            // One frame is shown whatever its rate. A file that states none
            // (a lone transport-stream picture without timing information)
            // takes 30 fps for cadence decisions only; the probe goes on
            // reporting that the file states no rate.
            None if index.frames.len() == 1 && !crate::probe::still_container(&probe.container) => {
                NominalRate {
                    rate: Rate::FPS_30,
                    method: crate::time::RateMethod::Declared,
                    candidates: vec![],
                    median_duration: 0,
                    window: 1,
                }
            }
            None => {
                return Err(MediaError::invalid(
                    "the stream has no frame rate (its frames share one timestamp)",
                ));
            }
        };
        let span = Span::of(&probe);
        let origin_pts = span.origin.in_units(index.timebase);
        let pts = index.frames.iter().map(|f| f.pts - origin_pts).collect();
        let end = match span.end {
            Some(end) => {
                crate::time::ticks_of(end.in_units(index.timebase) - origin_pts, index.timebase)
            }
            None => crate::time::ticks_of(index.end_pts() - origin_pts, index.timebase),
        };
        let legacy = LegacyMap::of(&index);
        let legacy_end = match Span::legacy_end(&probe) {
            Some(end) => crate::time::ticks_of(end.in_units(index.timebase), index.timebase),
            None => crate::time::ticks_of(index.end_pts() - index.edits.head_gap, index.timebase),
        }
        .max(0);
        Ok(Arc::new(MediaSource {
            id: NEXT_SOURCE.fetch_add(1, Ordering::Relaxed),
            stream,
            info: video.codec.clone(),
            extradata,
            startup_input,
            nominal,
            color: video.color.interpreted,
            pts,
            origin_pts,
            end,
            legacy_end,
            legacy,
            index,
            probe,
        }))
    }

    pub fn rate(&self) -> Rate {
        self.nominal.rate
    }

    /// Section 5 rule 7: the presented frame showing at clip time `t`
    /// (ticks; tick 0 is the first presented frame). `None` before it.
    pub fn select(&self, t: Tick) -> Option<usize> {
        select_frame(&self.pts, self.index.timebase, self.nominal.rate, t)
    }

    /// Presented frame times in index units as `timing` reads the file.
    pub fn clip_pts(&self, timing: ClipTiming) -> &[i64] {
        match timing {
            ClipTiming::Legacy => &self.legacy.pts,
            ClipTiming::Presentation => &self.pts,
        }
    }

    /// The presented frame an asset's source time `t` shows under its
    /// `timing`. The legacy reading always shows a frame (the first one
    /// holds before its own time); the presentation reading shows nothing
    /// before the picture starts.
    pub fn select_as(&self, timing: ClipTiming, t: Tick) -> Option<usize> {
        match timing {
            ClipTiming::Presentation => self.select(t),
            ClipTiming::Legacy if self.legacy.pts.is_empty() => None,
            ClipTiming::Legacy => Some(
                select_frame(&self.legacy.pts, self.index.timebase, self.nominal.rate, t)
                    .map_or(self.legacy.head, |n| n.max(self.legacy.head)),
            ),
        }
    }

    /// Clip time of presented frame `n`, in ticks.
    pub fn time_of(&self, n: usize) -> Tick {
        crate::time::ticks_of(self.pts[n], self.index.timebase)
    }

    /// The clip's length in ticks: from the start of the file's
    /// presentation to the latest end of any of its streams.
    pub fn duration(&self) -> Tick {
        self.end
    }

    /// The clip's length in ticks as an asset with `timing` reads the file
    /// ([`crate::clip::Span::legacy_end`] for older assets), so the last
    /// frame its source times reach is inside it.
    pub fn duration_as(&self, timing: ClipTiming) -> Tick {
        match timing {
            ClipTiming::Legacy => self.legacy_end,
            ClipTiming::Presentation => self.end,
        }
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
            cache_bytes: 24 << 20,
            pool_free_bytes: 16 << 20,
            threads: 4,
            idle_sessions: 1,
            session_window: super::session::DEFAULT_WINDOW,
        }
    }
}

/// The service's shares of the engine's lease (contract 8 categories).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaLease {
    /// The frame cache budget.
    pub frame_cache: usize,
    /// Base allowance outside the frame cache: session windows, decoder
    /// references, frames renderers still hold, and free pool slots. Warm
    /// references may borrow unused frame-cache capacity within the same
    /// combined allowance; zero still requests that every session close.
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
    /// When a drag frame last asked for video (ms since process start), for
    /// PM_FIX_VIDEO2's scrub-only decoder retention.
    scrubbed_ms: AtomicU64,
    open_sessions: AtomicU64,
    /// Decoders a failed session's recovery tried to open, in all.
    recovery_opens: AtomicU64,
    /// The two media shares are one bounded pool: warm decoder references
    /// can borrow only the frame-cache capacity that is not being kept.
    lease: Mutex<MediaLease>,
    /// Most recently displayed streams, not background preparation. Bounded
    /// metadata lets a clip being shown displace older idle decoders first.
    displayed: Mutex<VecDeque<SessionKey>>,
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

    /// See [`Session::set_skip_disposable`].
    pub fn set_skip_disposable(&mut self, on: bool) {
        self.session.as_mut().unwrap().set_skip_disposable(on);
    }

    /// Presented frame `n`, through the cache.
    pub fn frame(&mut self, n: usize) -> Result<DecodedFrame> {
        match self.frame_with(n, &mut |_| false)? {
            Some(f) => Ok(f),
            None => unreachable!("never stopped"),
        }
    }

    /// As [`frame`](SessionGuard::frame), asking `stop` between batches of
    /// samples whether to give up ([`Session::sample_with`]). Frames decoded
    /// on the way are cached whether or not the request finished.
    pub fn frame_with(
        &mut self,
        n: usize,
        stop: &mut dyn FnMut(super::session::Progress) -> bool,
    ) -> Result<Option<DecodedFrame>> {
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
            return Ok(Some(f));
        }
        let mut f = self.session.as_mut().unwrap().sample_with(sample, stop);
        if f.is_err() {
            f = self.recover(sample, stop, f);
        }
        let s = self.session.as_mut().unwrap();
        if f.is_ok() {
            s.replacement = false;
        }
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
        // Over the combined allowance, older idle state gives way before
        // the current window returns frames the cache could not keep.
        if self.service.decoders_over() {
            self.service.fit_older_than(self.key);
            if self.service.decoders_over() {
                s.clear_window();
            }
        }
        f
    }

    /// A VideoToolbox session that failed is not asked again: once the
    /// system invalidates it (a GPU reset, sleep and wake), every later
    /// sample fails. The session gives back its hardware and a new one
    /// opens, once; if the sample still fails and the stream has a software
    /// backend, that decodes it. A sample that fails everywhere is damaged,
    /// and a file no decoder opens any more is gone: the error stands, and
    /// the session that tried last stays, marked, so neither reopens
    /// decoders on every request.
    fn recover(
        &mut self,
        sample: usize,
        stop: &mut dyn FnMut(super::session::Progress) -> bool,
        failed: Result<Option<DecodedFrame>>,
    ) -> Result<Option<DecodedFrame>> {
        let old = self.session.as_mut().unwrap();
        if old.replacement || old.decoder_info().backend != Backend::VideoToolbox {
            return failed;
        }
        // Its hardware session is the replacement's to take.
        old.release_hardware();
        old.replacement = true;
        let software = super::backends(&self.source.info, &self.source.index)
            .into_iter()
            .find(|b| *b != Backend::VideoToolbox);
        let mut last = failed;
        for prefer in [None, software] {
            if prefer.is_none() && last.is_ok() {
                break;
            }
            self.service.recovery_opens.fetch_add(1, Ordering::Relaxed);
            let Ok(mut fresh) = self
                .service
                .open_session(&self.source, self.key.purpose, prefer)
            else {
                continue;
            };
            fresh.replacement = true;
            last = fresh.sample_with(sample, stop);
            // The failed session closes here; the new one takes its place.
            self.session = Some(fresh);
            self.service.open_sessions.fetch_sub(1, Ordering::Relaxed);
            if last.is_ok() || software.is_none() {
                break;
            }
        }
        last
    }

    /// See [`Session::invalidate_hardware`].
    #[doc(hidden)]
    pub fn invalidate_hardware(&mut self) -> bool {
        self.session.as_mut().unwrap().invalidate_hardware()
    }

    /// See [`Session::set_window`].
    pub fn set_window(&mut self, frames: usize) {
        self.session.as_mut().unwrap().set_window(frames);
    }

    /// See [`Session::set_keep_before`].
    pub fn set_keep_before(&mut self, frames: usize, stride: usize) {
        self.session
            .as_mut()
            .unwrap()
            .set_keep_before(frames, stride);
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
            // Over the combined media allowance, return the window first,
            // then close if needed. A zero decoder share always closes it.
            let zero = svc.lease.lock().unwrap().decoders == 0;
            // PM_FIX_KEEP_SESSION: over the share, a session gives back its
            // window but stays open as its key's one idle session; cold
            // sources and idle release close it.
            let keep = (super::diag::knob("PM_FIX_KEEP_SESSION") == Some(2)
                || super::diag::knob("PM_FIX_KEEP_SESSION") == Some(1)
                || svc.scrubbing())
                && svc
                    .idle
                    .lock()
                    .unwrap()
                    .get(&self.key)
                    .is_none_or(|l| l.is_empty());
            if !zero && svc.decoders_over() {
                svc.fit_older_than(self.key);
            }
            if zero || svc.decoders_over() {
                s.clear_window();
                // Returned window buffers are free pool storage now. Release
                // those before deciding the decoder itself cannot fit: closing
                // a warm decoder to reclaim reusable free buffers forces the
                // next playing frame to open and seek all over again.
                svc.trim_free();
                if !keep && (zero || svc.decoders_over()) {
                    drop(s);
                    svc.open_sessions.fetch_sub(1, Ordering::Relaxed);
                    super::diag::bump(&super::diag::SESSIONS_CLOSED_OVER_SHARE, 1);
                    svc.fit_decoders();
                    return;
                }
            }
            let mut idle = svc.idle.lock().unwrap();
            let list = idle.entry(self.key).or_default();
            let idle_cap = super::diag::knob("PM_ABL_IDLE_SESSIONS")
                .map_or(svc.config.idle_sessions, |n| n as usize);
            if list.len() < idle_cap {
                list.push(s);
            } else {
                super::diag::bump(&super::diag::SESSIONS_CLOSED_IDLE_FULL, 1);
                svc.open_sessions.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}

impl DecodeService {
    /// A drag frame is asking for video now (PM_FIX_VIDEO2).
    pub fn note_scrub(&self) {
        self.scrubbed_ms.store(scrub_clock_ms().max(1), Ordering::Relaxed);
    }

    /// Whether a drag asked for video in the last half second, with
    /// PM_FIX_VIDEO2 on: decoders and reorder windows are then kept.
    pub fn scrubbing(&self) -> bool {
        (super::diag::video2()
            || super::diag::knob("PM_SCRUB_EXTRA_MIB").is_some_and(|m| m > 0))
            && self.scrubbed_within(std::time::Duration::from_millis(500))
    }

    /// Whether a drag asked for video within `within`.
    pub fn scrubbed_within(&self, within: std::time::Duration) -> bool {
        let at = self.scrubbed_ms.load(Ordering::Relaxed);
        at != 0 && scrub_clock_ms().saturating_sub(at) < within.as_millis() as u64
    }

    /// Playback or export began: retention and the allowance end now.
    pub fn clear_scrub_mark(&self) {
        self.scrubbed_ms.store(0, Ordering::Relaxed);
    }

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
            scrubbed_ms: AtomicU64::new(0),
            open_sessions: AtomicU64::new(0),
            recovery_opens: AtomicU64::new(0),
            lease: Mutex::new(MediaLease::UNLIMITED),
            displayed: Mutex::new(VecDeque::new()),
        }
    }

    /// The display is asking for this source now. Preparation does not
    /// call this: a future clip must not displace the one being displayed.
    pub fn note_display(&self, source: u64, instance: InstanceId) {
        let key = SessionKey {
            source,
            instance,
            purpose: DecodePurpose::Display,
        };
        let mut displayed = self.displayed.lock().unwrap();
        if let Some(at) = displayed.iter().position(|k| *k == key) {
            displayed.remove(at);
        }
        displayed.push_front(key);
        displayed.truncate(32);
    }

    fn fit_older_than(&self, key: SessionKey) {
        self.fit_decoders_before(Some(key));
    }

    /// How many decoders failed sessions' recoveries tried to open. For tests.
    #[doc(hidden)]
    pub fn recovery_opens(&self) -> u64 {
        self.recovery_opens.load(Ordering::Relaxed)
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
            None => self.open_session(source, purpose, prefer)?,
        };
        let mut session = session;
        session.set_reorder(self.scrubbing());
        Ok(SessionGuard {
            service: self,
            key,
            source: source.clone(),
            session: Some(session),
        })
    }

    /// A new session for `source` (counted as open).
    fn open_session(
        &self,
        source: &Arc<MediaSource>,
        purpose: DecodePurpose,
        prefer: Option<Backend>,
    ) -> Result<Session> {
        // The probe's demuxer serves the first session.
        let input = match source.startup_input.lock().unwrap().take() {
            Some(input) => input,
            None => Input::open(source.path())?,
        };
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
        s.set_window_bytes(
            super::diag::knob("PM_ABL_WINDOW_MIB").map_or(32 << 20, |m| (m << 20) as usize),
        );
        self.open_sessions.fetch_add(1, Ordering::Relaxed);
        super::diag::bump(&super::diag::SESSIONS_OPENED, 1);
        Ok(s)
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
                super::diag::bump(&super::diag::SESSIONS_RELEASED, v.len() as u64);
            }
            !drop
        });
        if let Some(s) = source {
            self.cache.remove_source(s);
        }
    }

    /// Applies a lease change and enforces it. The frame cache stays within
    /// its maximum and yields unused capacity to warm decoder references.
    /// Over their combined allowance, windows and free buffers go first,
    /// then idle sessions. A zero decoder share closes every idle session.
    /// Checked-out sessions return their windows at their next request and
    /// close on return while still over. Live frames remain counted until
    /// their renderer lets them go.
    pub fn apply_lease(&self, lease: MediaLease) -> MediaUsage {
        *self.lease.lock().unwrap() = lease;
        self.cache.set_budget(lease.frame_cache);
        self.fit_decoders();
        self.usage()
    }

    /// A decoder may use the unused part of the frame-cache share, but
    /// their combined retained bytes must still fit the unchanged lease.
    /// Zero decoder leases remain a request to close every idle session.
    fn decoders_over(&self) -> bool {
        let lease = *self.lease.lock().unwrap();
        if lease.decoders == usize::MAX {
            return false;
        }
        if lease.decoders == 0 || lease.frame_cache == 0 {
            return self.usage().decoders > lease.decoders;
        }
        let total = lease.decoders.saturating_add(lease.frame_cache);
        let usage = self.usage();
        if usage.decoders.saturating_add(usage.frame_cache) > total && usage.pool.free_bytes > 0 {
            // Reusable free storage gives way before cached answers do.
            self.pool.trim(0);
        }
        let usage = self.usage();
        self.cache
            .set_budget(lease.frame_cache.min(total.saturating_sub(usage.decoders)));
        let usage = self.usage();
        usage.decoders.saturating_add(usage.frame_cache) > total
    }

    /// Brings combined media memory within its allowance, cheapest first.
    fn fit_decoders(&self) {
        self.fit_decoders_before(None);
    }

    fn fit_decoders_before(&self, before: Option<SessionKey>) {
        let mut idle = self.idle.lock().unwrap();
        let displayed = self.displayed.lock().unwrap().clone();
        let rank = |key: &SessionKey| displayed.iter().position(|k| k == key).unwrap_or(usize::MAX);
        let before = before.map(|key| rank(&key));
        let eligible = |key: &SessionKey| before.is_none_or(|current| rank(key) > current);
        if self.decoders_over() {
            for (key, sessions) in idle.iter_mut() {
                if eligible(key) {
                    for s in sessions {
                        s.clear_window();
                    }
                }
            }
        }
        self.trim_free();
        // A zero share closes every idle session: the state VideoToolbox
        // keeps per session is not measurable, so nothing else frees it.
        let zero = self.lease.lock().unwrap().decoders == 0;
        // PM_FIX_KEEP_SESSION=2: over the share, idle sessions keep only
        // their decoder (windows and free pool memory go above); only a zero
        // share (idle release, project close) closes them.
        let keep = super::diag::knob("PM_FIX_KEEP_SESSION") == Some(2) || self.scrubbing();
        while zero || (!keep && self.decoders_over()) {
            let key = idle
                .iter()
                .filter(|(key, list)| !list.is_empty() && eligible(key))
                .max_by_key(|(key, _)| rank(key))
                .map(|(key, _)| *key);
            let Some(s) = key.and_then(|key| idle.get_mut(&key).and_then(|list| list.pop())) else {
                break;
            };
            drop(s);
            self.open_sessions.fetch_sub(1, Ordering::Relaxed);
            super::diag::bump(&super::diag::SESSIONS_CLOSED_OVER_SHARE, 1);
            self.trim_free();
        }
        idle.retain(|_, l| !l.is_empty());
    }

    /// Free buffers use only what the combined allowance leaves after
    /// cached frames and live decoder storage, within the configured cap.
    fn trim_free(&self) {
        let lease = *self.lease.lock().unwrap();
        let u = self.usage();
        let live = u.decoders - u.pool.free_bytes.min(u.decoders);
        let free = if lease.decoders == 0 {
            0
        } else {
            lease
                .decoders
                .saturating_add(lease.frame_cache)
                .saturating_sub(u.frame_cache)
                .saturating_sub(live)
        };
        self.pool.trim(self.config.pool_free_bytes.min(free));
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

/// Milliseconds since this process first asked (PM_FIX_VIDEO2's clock).
fn scrub_clock_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}
