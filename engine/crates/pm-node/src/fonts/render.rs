//! Fonts for the engine's own text rendering (plan section 12; Phase 3c
//! "full render fonts on the render and job threads"; RFC 16).
//!
//! The registry (`pm_text::registry::CoreTextRegistry`) uses AppKit's font
//! manager, so it lives on the utility process's JavaScript thread (the
//! process main thread, [`super::FontService`]). The threads that render
//! (the viewer presenter's render thread, the job engine's lanes) lay text
//! out with `pm-text`'s level 1 layout over a [`ReplayResolver`] holding
//! the registry's answers and the **full** font files (outlines for
//! drawing), as `pm-wasm` replays them over subsets: the layouts agree
//! glyph for glyph (`tests/fonts.rs`).
//!
//! A query the replay has no answer for is asked of the JavaScript thread
//! through a [`FontBroker`]: the asking thread queues the queries, wakes
//! the JavaScript thread (the host's `pump`), and waits for the reply with
//! a bounded timeout. The JavaScript thread never waits for a render
//! thread, so this cannot deadlock; a JavaScript thread that does not
//! answer in time (or a registry that cannot be created) makes the render
//! fail visibly ([`FontHealth`]), never draw text with missing fonts.
//!
//! Project fonts and variable sources change the registry's generation;
//! the broker carries it, and a renderer whose fonts are of an older
//! generation installs fresh ones (dropping its glyph and raster caches).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pm_text::answers::{Answers, Query, ReplayResolver};
use pm_text::font::{FaceId, FaceInfo, FontBytes, FontDesc, FontResolver};

/// How long a render thread waits for the JavaScript thread's answer. The
/// first fallback manifest takes about 0.6 s on that thread.
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

/// What a source of registry answers must do (the font service; tests use
/// a registry directly).
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

    fn fail(&self, why: String) {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(why);
        }
    }
}

/// A replay of the registry's answers over full fonts that asks the
/// registry for what it lacks.
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
}

/// The fonts a renderer lays text out with, over a broker (the interactive
/// engine's render thread and the job engine's lanes).
#[cfg(target_os = "macos")]
pub struct EngineFonts {
    measure: pm_eval::text_measure::CompatMeasure<BrokerResolver>,
    generation: u64,
}

#[cfg(target_os = "macos")]
impl EngineFonts {
    /// Fonts at the broker's current generation.
    pub fn new(broker: &FontBroker, health: &FontHealth) -> Self {
        let generation = broker.generation();
        EngineFonts {
            measure: pm_eval::text_measure::CompatMeasure::new(BrokerResolver::new(
                broker.clone(),
                health.clone(),
            )),
            generation,
        }
    }
}

#[cfg(target_os = "macos")]
impl pm_render::text::TextFonts for EngineFonts {
    fn measure(&mut self) -> &mut dyn pm_eval::text_geom::TextMeasure {
        &mut self.measure
    }
    fn line_runs(&mut self, d: &pm_text::compat::TextContent) -> Vec<pm_text::compat::LineRuns> {
        self.measure.compat().line_runs(d)
    }
    fn text_runs(
        &mut self,
        d: &pm_text::compat::TextContent,
        text: &str,
        tracking: f64,
    ) -> Vec<pm_text::compat::GlyphRun> {
        self.measure.compat().text_runs(d, text, tracking).0
    }
    fn face(&mut self, id: FaceId) -> &FaceInfo {
        self.measure.compat().resolver().face(id)
    }
    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        self.measure.compat().resolver_mut().em_advance(face, size)
    }
    fn generation(&self) -> u64 {
        self.generation
    }
}

/// A renderer's fonts and their state, as a rendering thread keeps them.
#[cfg(target_os = "macos")]
#[derive(Clone, Debug)]
pub struct RenderFonts {
    pub broker: FontBroker,
    pub health: FontHealth,
    /// Set after a render met a failure: the next use installs fonts afresh.
    reinstall: Arc<AtomicBool>,
}

#[cfg(target_os = "macos")]
impl RenderFonts {
    pub fn new(broker: FontBroker) -> Self {
        RenderFonts {
            broker,
            health: FontHealth::default(),
            reinstall: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Installs fonts at the broker's generation into `renderer` when it
    /// has none or older ones; true when it installed.
    pub fn sync(&self, renderer: &mut pm_render::Renderer, installed: &mut Option<u64>) -> bool {
        let generation = self.broker.generation();
        let reinstall = self.reinstall.swap(false, Ordering::AcqRel);
        if *installed == Some(generation) && !reinstall {
            return false;
        }
        renderer.set_text_fonts(Some(Box::new(EngineFonts::new(&self.broker, &self.health))));
        *installed = Some(generation);
        true
    }

    /// After a render: the font failure it met, if any. The renderer's fonts
    /// are installed afresh before the next render (their caches may hold
    /// answers the failure left empty).
    pub fn after_render(&self) -> Option<String> {
        let failure = self.health.take_failure();
        if failure.is_some() {
            self.reinstall.store(true, Ordering::Release);
        }
        failure
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_text::answers::{Answer, FaceRecord};

    /// A registry stand-in: one family, served from bytes it owns.
    struct Fake {
        generation: u64,
        answered: usize,
    }

    impl FontSource for Fake {
        fn generation(&self) -> u64 {
            self.generation
        }
        fn answer(&mut self, queries: &[Query]) -> Result<Answers, String> {
            self.answered += queries.len();
            let mut answers = Answers::default();
            for q in queries {
                answers.answers.push(Answer {
                    query: q.clone(),
                    face: None,
                    advance: None,
                });
            }
            Ok(answers)
        }
        fn blob(&self, key: &str) -> Option<FontBytes> {
            (key == "present").then(|| FontBytes::new(vec![1, 2, 3]))
        }
    }

    fn family(name: &str) -> Query {
        Query::Family {
            name: name.into(),
            weight: 400f32.to_bits(),
            italic: false,
            size: 16f32.to_bits(),
        }
    }

    /// Serves `broker` from `source` on this thread until `done`.
    fn serve_until(broker: &FontBroker, source: &mut dyn FontSource, done: &AtomicBool) {
        while !done.load(Ordering::Acquire) {
            serve(source, broker.take_asks());
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_rendering_thread_waits_for_the_service_threads_answer() {
        let broker = FontBroker::new();
        let woken = Arc::new(AtomicU64::new(0));
        let w = woken.clone();
        broker.set_wake(Arc::new(move || {
            w.fetch_add(1, Ordering::Relaxed);
        }));
        broker.set_generation(7);
        let done = Arc::new(AtomicBool::new(false));
        let asking = {
            let broker = broker.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                let reply = broker
                    .ask(
                        vec![family("A"), family("B")],
                        vec![("present".into(), 0)],
                        ASK_TIMEOUT,
                    )
                    .unwrap();
                done.store(true, Ordering::Release);
                reply
            })
        };
        let mut source = Fake {
            generation: 7,
            answered: 0,
        };
        serve_until(&broker, &mut source, &done);
        let reply = asking.join().unwrap();
        assert_eq!(reply.generation, 7);
        assert_eq!(reply.answers.answers.len(), 2);
        assert_eq!(reply.fonts.len(), 1, "the asked face's bytes");
        assert_eq!(source.answered, 2);
        assert!(
            woken.load(Ordering::Relaxed) >= 1,
            "the ask woke the service"
        );
        assert_eq!(broker.counts(), (1, 1, 0));
    }

    #[test]
    fn an_unanswered_ask_fails_in_bounded_time() {
        let broker = FontBroker::new();
        broker.set_wake(Arc::new(|| {}));
        let started = std::time::Instant::now();
        let error = broker
            .ask(vec![family("A")], Vec::new(), Duration::from_millis(50))
            .err()
            .unwrap();
        assert!(error.contains("did not answer"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(broker.counts(), (1, 0, 1));
    }

    #[test]
    fn a_closed_broker_releases_waiting_threads_and_refuses_new_asks() {
        let broker = FontBroker::new();
        broker.set_wake(Arc::new(|| {}));
        let waiting = {
            let broker = broker.clone();
            std::thread::spawn(move || broker.ask(vec![family("A")], Vec::new(), ASK_TIMEOUT))
        };
        // Let the ask queue, then stop (as a closing engine does).
        while broker.0.asks.lock().unwrap().is_empty() {
            std::thread::sleep(Duration::from_millis(1));
        }
        let started = std::time::Instant::now();
        broker.close();
        assert!(waiting.join().unwrap().is_err());
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "released at once"
        );
        assert!(broker.ask(Vec::new(), Vec::new(), ASK_TIMEOUT).is_err());
    }

    #[test]
    fn without_a_service_an_ask_fails_at_once() {
        let broker = FontBroker::new();
        assert!(
            broker
                .ask(vec![family("A")], Vec::new(), ASK_TIMEOUT)
                .unwrap_err()
                .contains("no font service")
        );
    }

    #[test]
    fn a_missing_blob_is_an_error_not_a_silent_gap() {
        let broker = FontBroker::new();
        broker.set_wake(Arc::new(|| {}));
        let done = Arc::new(AtomicBool::new(false));
        let asking = {
            let broker = broker.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                let r = broker.ask(Vec::new(), vec![("absent".into(), 0)], ASK_TIMEOUT);
                done.store(true, Ordering::Release);
                r
            })
        };
        let mut source = Fake {
            generation: 1,
            answered: 0,
        };
        serve_until(&broker, &mut source, &done);
        assert!(
            asking
                .join()
                .unwrap()
                .err()
                .unwrap()
                .contains("no font absent")
        );
    }

    #[test]
    fn the_resolver_stops_asking_after_a_failure_until_it_is_taken() {
        let broker = FontBroker::new();
        broker.set_wake(Arc::new(|| {}));
        let health = FontHealth::default();
        let mut resolver = BrokerResolver::new(broker.clone(), health.clone())
            .with_timeout(Duration::from_millis(20));
        let desc = FontDesc {
            weight: 400.0,
            italic: false,
            size: 16.0,
        };
        assert_eq!(resolver.family("A", &desc), None);
        assert!(health.failed());
        // Later lookups fail at once (one ask, not one per lookup).
        for name in ["B", "C", "D"] {
            assert_eq!(resolver.family(name, &desc), None);
        }
        assert_eq!(broker.counts().0, 1);
        assert!(health.take_failure().unwrap().contains("did not answer"));
        assert!(!health.failed());
    }

    #[test]
    fn records_and_answers_reach_the_replay() {
        // An answer naming a face is usable once its bytes arrive with it.
        let record = FaceRecord {
            id: 0,
            blob: "present".into(),
            index: 0,
            coords: Vec::new(),
            synthetic_bold: false,
            synthetic_italic: false,
            custom: false,
            web: None,
            synthesized_hyphens: [None, None],
            postscript_name: "Fake".into(),
            typeface_key: "fake".into(),
            fallback_key: None,
            facts: Default::default(),
        };
        struct Named(FaceRecord);
        impl FontSource for Named {
            fn generation(&self) -> u64 {
                3
            }
            fn answer(&mut self, queries: &[Query]) -> Result<Answers, String> {
                Ok(Answers {
                    faces: vec![self.0.clone()],
                    answers: queries
                        .iter()
                        .map(|q| Answer {
                            query: q.clone(),
                            face: Some(0),
                            advance: None,
                        })
                        .collect(),
                })
            }
            fn blob(&self, key: &str) -> Option<FontBytes> {
                (key == "present").then(|| FontBytes::new(vec![0; 4]))
            }
        }
        let broker = FontBroker::new();
        broker.set_wake(Arc::new(|| {}));
        let health = FontHealth::default();
        let done = Arc::new(AtomicBool::new(false));
        let looking = {
            let broker = broker.clone();
            let health = health.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                let mut resolver = BrokerResolver::new(broker, health);
                let found = resolver.family(
                    "Fake",
                    &FontDesc {
                        weight: 400.0,
                        italic: false,
                        size: 16.0,
                    },
                );
                // Answered from the replay now: no second ask.
                let again = resolver.family(
                    "Fake",
                    &FontDesc {
                        weight: 400.0,
                        italic: false,
                        size: 16.0,
                    },
                );
                done.store(true, Ordering::Release);
                (found, again)
            })
        };
        let mut source = Named(record);
        serve_until(&broker, &mut source, &done);
        assert_eq!(looking.join().unwrap(), (Some(0), Some(0)));
        assert_eq!(
            broker.counts().0,
            1,
            "one ask: the answer and its bytes together"
        );
        assert!(!health.failed());
    }
}
