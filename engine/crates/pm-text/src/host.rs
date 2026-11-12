//! Hosting the font registry for the threads that render (plan section 12,
//! RFC 16).
//!
//! The registry ([`crate::registry::CoreTextRegistry`]) uses AppKit's font
//! manager, so it lives on the process's main thread, behind a
//! [`FontService`]. The threads that render (the viewer presenter's render
//! thread, the job engine's lanes) lay text out with level 1 layout over a
//! [`ReplayResolver`] holding the registry's answers and the **full** font
//! files (outlines for drawing), as `pm-wasm` replays them over subsets: the
//! layouts agree glyph for glyph.
//!
//! A query the replay has no answer for is asked of the main thread through a
//! [`FontBroker`]: the asking thread queues the queries, wakes the main
//! thread (the host's `pump`), and waits for the reply with a bounded
//! timeout. The main thread never waits for a render thread, so this cannot
//! deadlock; a main thread that does not answer in time (or a registry that
//! cannot be created) makes the render fail visibly ([`FontHealth`]), never
//! draw text with missing fonts.
//!
//! Project fonts and variable sources change the registry's generation; the
//! broker carries it, and a renderer whose fonts are of an older generation
//! installs fresh ones.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::answers::{Answers, Query, ReplayResolver};
use crate::font::{FaceId, FaceInfo, FontBytes, FontDesc, FontResolver};

/// How long a render thread waits for the main thread's answer. The first
/// fallback manifest takes about 0.6 s on that thread.
pub const ASK_TIMEOUT: Duration = Duration::from_secs(8);

/// Questions a rendering thread has for the registry.
pub struct FontAsk {
    pub queries: Vec<Query>,
    /// Faces (blob, index) whose full bytes are wanted.
    pub blobs: Vec<(String, u32)>,
    reply: SyncSender<Result<FontReply, String>>,
}

/// The registry's answer: records and answers, and the full bytes of every
/// face they name (shared, never copied).
#[derive(Default)]
pub struct FontReply {
    pub generation: u64,
    pub answers: Answers,
    pub fonts: Vec<(String, u32, FontBytes)>,
}

impl std::fmt::Debug for FontReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontReply")
            .field("generation", &self.generation)
            .field("answers", &self.answers.answers.len())
            .field("faces", &self.answers.faces.len())
            .field("fonts", &self.fonts.len())
            .finish()
    }
}

type Wake = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    asks: Mutex<VecDeque<FontAsk>>,
    wake: Mutex<Option<Wake>>,
    generation: AtomicU64,
    closed: AtomicBool,
    asked: AtomicU64,
    answered: AtomicU64,
    failed: AtomicU64,
}

/// The rendezvous between rendering threads and the font service.
#[derive(Clone)]
pub struct FontBroker(Arc<Shared>);

impl std::fmt::Debug for FontBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontBroker")
            .field("generation", &self.generation())
            .field("closed", &self.0.closed.load(Ordering::Relaxed))
            .finish()
    }
}

impl Default for FontBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl FontBroker {
    pub fn new() -> Self {
        FontBroker(Arc::new(Shared {
            asks: Mutex::new(VecDeque::new()),
            wake: Mutex::new(None),
            generation: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            asked: AtomicU64::new(0),
            answered: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        }))
    }

    /// What wakes the font service's thread (the host's sink: it calls
    /// `pump`, which answers the queued asks).
    pub fn set_wake(&self, wake: Wake) {
        *self.0.wake.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake);
    }

    /// The registry generation the service last reported.
    pub fn generation(&self) -> u64 {
        self.0.generation.load(Ordering::Acquire)
    }

    pub fn set_generation(&self, generation: u64) {
        self.0.generation.store(generation, Ordering::Release);
    }

    /// `(asked, answered, failed)` asks, for status and tests.
    pub fn counts(&self) -> (u64, u64, u64) {
        (
            self.0.asked.load(Ordering::Relaxed),
            self.0.answered.load(Ordering::Relaxed),
            self.0.failed.load(Ordering::Relaxed),
        )
    }

    /// Asks the registry and waits (at most `timeout`) for its answer. Called
    /// on a rendering thread, never on the service's own thread.
    pub fn ask(
        &self,
        queries: Vec<Query>,
        blobs: Vec<(String, u32)>,
        timeout: Duration,
    ) -> Result<FontReply, String> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err("the engine's font service has stopped".into());
        }
        let (reply, rx) = sync_channel(1);
        self.0.asked.fetch_add(1, Ordering::Relaxed);
        self.0
            .asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(FontAsk {
                queries,
                blobs,
                reply,
            });
        let wake = self
            .0
            .wake
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        match wake {
            Some(wake) => wake(),
            None => {
                self.0.failed.fetch_add(1, Ordering::Relaxed);
                return Err("the engine has no font service".into());
            }
        }
        match rx.recv_timeout(timeout) {
            Ok(Ok(reply)) => {
                self.0.answered.fetch_add(1, Ordering::Relaxed);
                Ok(reply)
            }
            Ok(Err(error)) => {
                self.0.failed.fetch_add(1, Ordering::Relaxed);
                Err(error)
            }
            Err(_) => {
                self.0.failed.fetch_add(1, Ordering::Relaxed);
                Err(format!(
                    "the engine's font service did not answer within {} ms",
                    timeout.as_millis()
                ))
            }
        }
    }

    /// The queued asks (the service's thread answers them).
    pub fn take_asks(&self) -> Vec<FontAsk> {
        self.0
            .asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    /// Stops the broker: queued and later asks fail at once (a closing
    /// engine never leaves a rendering thread waiting).
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::Release);
        for ask in self.take_asks() {
            let _ = ask
                .reply
                .send(Err("the engine's font service has stopped".into()));
        }
    }
}

impl FontAsk {
    /// Answers with `answer` (the service's thread).
    pub fn answer(self, answer: Result<FontReply, String>) {
        let _ = self.reply.send(answer);
    }
}

/// What a source of registry answers must do (the font service; tests use a
/// registry directly).
pub trait FontSource {
    fn generation(&self) -> u64;
    fn answer(&mut self, queries: &[Query]) -> Result<Answers, String>;
    /// The full bytes of a blob.
    fn blob(&self, key: &str) -> Option<FontBytes>;
}

/// Answers every queued ask from `source` (on the service's thread).
pub fn serve(source: &mut dyn FontSource, asks: Vec<FontAsk>) {
    for ask in asks {
        let result = (|| {
            let answers = if ask.queries.is_empty() {
                Answers::default()
            } else {
                source.answer(&ask.queries)?
            };
            let mut wanted: Vec<(String, u32)> = answers
                .faces
                .iter()
                .map(|f| (f.blob.clone(), f.index))
                .chain(ask.blobs.iter().cloned())
                .collect();
            wanted.sort();
            wanted.dedup();
            let mut fonts = Vec::with_capacity(wanted.len());
            for (blob, index) in wanted {
                let bytes = source
                    .blob(&blob)
                    .ok_or_else(|| format!("the registry has no font {blob}"))?;
                fonts.push((blob, index, bytes));
            }
            Ok(FontReply {
                generation: source.generation(),
                answers,
                fonts,
            })
        })();
        ask.answer(result);
    }
}

/// Whether the fonts of one renderer met a failure (a render that used them
/// must not be shown).
#[derive(Clone, Debug, Default)]
pub struct FontHealth(Arc<Mutex<Option<String>>>);

impl FontHealth {
    /// The failure since the last call, cleared.
    pub fn take_failure(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// Whether a failure is recorded (later asks fail at once until it is
    /// taken: one unanswered ask must not stall a frame per glyph run).
    pub fn failed(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// Records a failure (the first since the last take is kept).
    pub fn fail(&self, why: String) {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(why);
        }
    }
}

/// A replay of the registry's answers over full fonts that asks the registry
/// for what it lacks.
pub struct BrokerResolver {
    inner: ReplayResolver,
    broker: FontBroker,
    health: FontHealth,
    timeout: Duration,
}

impl BrokerResolver {
    pub fn new(broker: FontBroker, health: FontHealth) -> Self {
        BrokerResolver {
            inner: ReplayResolver::new(&Answers::default()),
            broker,
            health,
            timeout: ASK_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Asks for the misses and missing bytes of the last lookup; true when
    /// the replay gained something.
    fn fill(&mut self) -> bool {
        let queries = std::mem::take(&mut self.inner.misses);
        let blobs: Vec<(String, u32)> = std::mem::take(&mut self.inner.pending_blobs)
            .into_iter()
            .collect();
        if queries.is_empty() && blobs.is_empty() {
            return false;
        }
        if self.health.failed() {
            return false;
        }
        match self.broker.ask(queries, blobs, self.timeout) {
            Ok(reply) => {
                self.inner.add_answers(&reply.answers);
                for (blob, index, bytes) in reply.fonts {
                    // Full fonts: read face `index` of the file.
                    self.inner.add_font(&blob, index, bytes, index);
                }
                true
            }
            Err(why) => {
                self.health.fail(why);
                false
            }
        }
    }

    /// Runs a lookup, filling the replay from the registry until it is
    /// answered (a query, then the bytes of the face it named).
    fn lookup<T>(&mut self, f: impl Fn(&mut ReplayResolver) -> Option<T>) -> Option<T> {
        for _ in 0..3 {
            self.inner.clear_reports();
            if let Some(found) = f(&mut self.inner) {
                return Some(found);
            }
            if !self.fill() {
                return None;
            }
        }
        self.inner.clear_reports();
        f(&mut self.inner)
    }
}

impl FontResolver for BrokerResolver {
    fn face(&self, id: FaceId) -> &FaceInfo {
        self.inner.face(id)
    }
    fn family(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId> {
        self.lookup(|r| r.family(family, desc))
    }
    fn standard(&mut self, desc: &FontDesc) -> Option<FaceId> {
        self.lookup(|r| r.standard(desc))
    }
    fn fallback_for_char(
        &mut self,
        desc: &FontDesc,
        ch: u32,
        primary: FaceId,
        emoji: bool,
    ) -> Option<FaceId> {
        self.lookup(|r| r.fallback_for_char(desc, ch, primary, emoji))
    }
    fn last_resort(&mut self, desc: &FontDesc) -> Option<FaceId> {
        self.lookup(|r| r.last_resort(desc))
    }
    fn variable_face(&mut self, family: &str) -> Option<FaceId> {
        self.lookup(|r| r.variable_face(family))
    }
    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        // `None` is a real answer here (advances follow `hmtx`), so only an
        // unanswered query is asked for.
        self.inner.clear_reports();
        let first = self.inner.em_advance(face, size);
        if first.is_some() || self.inner.misses.is_empty() {
            return first;
        }
        if !self.fill() {
            return None;
        }
        self.inner.clear_reports();
        self.inner.em_advance(face, size)
    }
    fn sbix_metric_rect(&mut self, face: FaceId, glyph: u32, size: f32) -> Option<[f64; 4]> {
        // As `em_advance`: `None` is a real answer (CoreText has a path).
        self.inner.clear_reports();
        let first = self.inner.sbix_metric_rect(face, glyph, size);
        if first.is_some() || self.inner.misses.is_empty() {
            return first;
        }
        if !self.fill() {
            return None;
        }
        self.inner.clear_reports();
        self.inner.sbix_metric_rect(face, glyph, size)
    }
}

/// The registry behind the service: the CoreText registry on macOS.
#[cfg(all(target_os = "macos", feature = "system"))]
type Registry = crate::registry::CoreTextRegistry;

/// The font service of one engine launch: the CoreText registry on the main
/// thread, answering the render threads' asks ([`serve`]).
#[cfg(all(target_os = "macos", feature = "system"))]
pub struct FontService {
    layout: crate::compat::Compat<Registry>,
}

#[cfg(all(target_os = "macos", feature = "system"))]
impl FontService {
    /// Creates the registry. Fails off the main thread (AppKit).
    pub fn new() -> Result<Self, String> {
        let registry =
            Registry::try_new().ok_or("the font registry must be created on the main thread")?;
        Ok(FontService {
            layout: crate::compat::Compat::new(registry),
        })
    }

    fn registry(&mut self) -> &mut Registry {
        self.layout.resolver_mut()
    }

    /// The registry generation as the renderer sees it: the registry's own
    /// count of project font changes.
    pub fn generation(&self) -> u64 {
        self.layout.resolver().generation()
    }

    /// Numeric payload accounting; full font files can be memory mapped,
    /// so these byte lengths are not a resident-memory measurement.
    pub fn memory_usage(&self) -> crate::registry::FontMemoryUsage {
        self.layout.resolver().memory_usage()
    }
}

#[cfg(all(target_os = "macos", feature = "system"))]
impl FontSource for FontService {
    fn generation(&self) -> u64 {
        FontService::generation(self)
    }

    fn answer(&mut self, queries: &[Query]) -> Result<Answers, String> {
        self.registry().answer(queries).map_err(|e| e.to_string())
    }

    fn blob(&self, key: &str) -> Option<FontBytes> {
        self.layout.resolver().blob(key)
    }
}

/// Without a native registry (not macOS, or `system` off), every request is
/// refused.
#[cfg(not(all(target_os = "macos", feature = "system")))]
pub struct FontService;

#[cfg(not(all(target_os = "macos", feature = "system")))]
impl FontService {
    pub fn new() -> Result<Self, String> {
        Err("the native font registry is macOS only".into())
    }

    pub fn generation(&self) -> u64 {
        0
    }
}

#[cfg(not(all(target_os = "macos", feature = "system")))]
impl FontSource for FontService {
    fn generation(&self) -> u64 {
        0
    }

    fn answer(&mut self, _queries: &[Query]) -> Result<Answers, String> {
        Err("the native font registry is macOS only".into())
    }

    fn blob(&self, _key: &str) -> Option<FontBytes> {
        None
    }
}
