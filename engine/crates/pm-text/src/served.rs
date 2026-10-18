//! A font resolver served by a registry that lives elsewhere (plan section
//! 12; `docs/engine/text.md`, "Fonts on render threads").
//!
//! The CoreText registry runs on its process's main thread (AppKit's font
//! manager). Render threads lay text out and draw it with the full fonts
//! the registry reads (outlines, color glyphs, variable axes, hidden system
//! fonts, project fonts), so they need its answers without being on that
//! thread. A [`FontServer`] answers a batch of [`Query`]s with the registry's
//! answers plus the full bytes of every face they name; [`ServedResolver`]
//! asks it synchronously for each query it has not seen, keeps every answer,
//! and so answers exactly what the registry would.
//!
//! Unlike [`crate::answers::ReplayResolver`] (pm-wasm, whose host answers
//! asynchronously and lays text out again), nothing is guessed: a query is
//! answered before the layout continues. When the server cannot answer (the
//! engine is shutting down, the registry thread did not answer in time), the
//! resolver records the failure ([`ServedResolver::take_failures`]) and the
//! host must refuse the frame rather than draw it with another font.
//!
//! Answers are valid for one registry generation. A resolver remembers the
//! generation of its first answer; [`ServedResolver::is_current`] turns
//! false once the server's generation moves (project fonts changed), and the
//! host replaces the resolver at its next frame boundary.

use std::collections::HashMap;
use std::sync::Arc;

use crate::answers::{Answer, Answers, Query};
use crate::font::{FaceId, FaceInfo, FontBytes, FontDesc, FontResolver};

/// The full bytes of one face an answer named.
#[derive(Clone)]
pub struct ServedFont {
    /// The registry's blob key and face index (`FaceRecord::blob`, `index`).
    pub blob: String,
    pub index: u32,
    /// The font file (a collection keeps its other faces).
    pub data: FontBytes,
    /// The face index to read `data` at (`index` for a full font file).
    pub read_index: u32,
}

/// One reply of a [`FontServer`].
#[derive(Clone, Default)]
pub struct Served {
    /// The registry generation the answers were given under.
    pub generation: u64,
    pub answers: Answers,
    /// The bytes of every face `answers` names.
    pub fonts: Vec<ServedFont>,
}

/// A font registry reachable from any thread.
pub trait FontServer: Send + Sync {
    /// Answers `queries` as the registry's `answer` does, with the full
    /// bytes of every face the answers name. Blocks until answered.
    fn serve(&self, queries: &[Query]) -> Result<Served, String>;
    /// The registry's current generation (cheap; no round trip).
    fn generation(&self) -> u64;
}

/// Answers resolver queries by asking a [`FontServer`] once per new query.
pub struct ServedResolver {
    server: Arc<dyn FontServer>,
    /// The generation this resolver's answers belong to (`None` before the
    /// first reply).
    generation: Option<u64>,
    answers: HashMap<Query, Answer>,
    faces: Vec<Option<FaceInfo>>,
    /// Font files already received, by (blob, index).
    fonts: HashMap<(String, u32), (FontBytes, u32)>,
    failures: Vec<String>,
    /// A reply came from a later generation than the first one.
    mixed: bool,
    round_trips: u64,
    /// Family names asked so far (bounded), for batching: a canvas font asks
    /// for each family of its list and the standard font at one
    /// description, so a description not seen yet (an animated size) asks
    /// for all of them in one round trip.
    families: Vec<String>,
}

/// Most family names remembered for batching.
const BATCH_FAMILIES: usize = 32;

impl ServedResolver {
    pub fn new(server: Arc<dyn FontServer>) -> Self {
        ServedResolver {
            server,
            generation: None,
            answers: HashMap::new(),
            faces: Vec::new(),
            fonts: HashMap::new(),
            failures: Vec::new(),
            mixed: false,
            round_trips: 0,
            families: Vec::new(),
        }
    }

    pub fn server(&self) -> &Arc<dyn FontServer> {
        &self.server
    }

    /// Whether every answer this resolver holds is from the server's
    /// current generation.
    pub fn is_current(&self) -> bool {
        !self.mixed
            && self
                .generation
                .is_none_or(|g| g == self.server.generation())
    }

    /// The failures met since the last call (each one a query left
    /// unanswered).
    pub fn take_failures(&mut self) -> Vec<String> {
        std::mem::take(&mut self.failures)
    }

    /// Round trips to the server so far.
    pub fn round_trips(&self) -> u64 {
        self.round_trips
    }

    /// Asks the server for every query not answered yet, in one round trip
    /// (a document's families on open).
    pub fn prefetch(&mut self, queries: &[Query]) {
        let todo: Vec<Query> = queries
            .iter()
            .filter(|q| !self.answers.contains_key(*q))
            .cloned()
            .collect();
        if !todo.is_empty() {
            self.ask(&todo);
        }
    }

    fn ask(&mut self, queries: &[Query]) {
        self.round_trips += 1;
        match self.server.serve(queries) {
            Ok(served) => self.add(served),
            Err(e) => self
                .failures
                .push(format!("the font registry did not answer: {e}")),
        }
    }

    fn add(&mut self, served: Served) {
        match self.generation {
            None => self.generation = Some(served.generation),
            Some(g) if g != served.generation => self.mixed = true,
            Some(_) => {}
        }
        for f in served.fonts {
            self.fonts
                .entry((f.blob, f.index))
                .or_insert((f.data, f.read_index));
        }
        for rec in &served.answers.faces {
            let id = rec.id as usize;
            if self.faces.len() <= id {
                self.faces.resize(id + 1, None);
            }
            if self.faces[id].is_none() {
                match self.fonts.get(&(rec.blob.clone(), rec.index)) {
                    Some((data, read_index)) => {
                        self.faces[id] = Some(rec.info(data.clone(), *read_index));
                    }
                    None => self.failures.push(format!(
                        "the font registry named face {} ({}) without its bytes",
                        rec.id, rec.postscript_name
                    )),
                }
            }
        }
        for a in served.answers.answers {
            self.answers.insert(a.query.clone(), a);
        }
    }

    /// `q` and, for a family or standard font query, the other queries a
    /// canvas font asks at the same description (every family name seen so
    /// far, and the standard font) that are not answered yet.
    fn batch(&mut self, q: &Query) -> Vec<Query> {
        let mut batch = vec![q.clone()];
        if let Query::Family { name, .. } = q
            && !self.families.iter().any(|n| n == name)
            && self.families.len() < BATCH_FAMILIES
        {
            self.families.push(name.clone());
        }
        if matches!(q, Query::Family { .. } | Query::Standard { .. })
            && let Some(desc) = q.desc()
        {
            let siblings = self
                .families
                .iter()
                .map(|n| Query::family(n, &desc))
                .chain(std::iter::once(Query::standard(&desc)));
            for s in siblings {
                if &s != q && !self.answers.contains_key(&s) && !batch.contains(&s) {
                    batch.push(s);
                }
            }
        }
        batch
    }

    fn answer(&mut self, q: Query) -> Option<&Answer> {
        if !self.answers.contains_key(&q) {
            let batch = self.batch(&q);
            self.ask(&batch);
        }
        let a = self.answers.get(&q);
        if a.is_none() && self.failures.is_empty() {
            self.failures
                .push(format!("the font registry left {q:?} unanswered"));
        }
        a
    }

    fn face_answer(&mut self, q: Query) -> Option<FaceId> {
        let face = self.answer(q)?.face?;
        if matches!(self.faces.get(face as usize), Some(Some(_))) {
            Some(face)
        } else {
            self.failures
                .push(format!("face {face} was answered without its bytes"));
            None
        }
    }
}

impl FontResolver for ServedResolver {
    fn face(&self, id: FaceId) -> &FaceInfo {
        // Only ids `face_answer` returned (or records the server sent, which
        // name loaded faces) reach the layout.
        self.faces[id as usize]
            .as_ref()
            .expect("served faces carry their bytes")
    }
    fn family(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId> {
        self.face_answer(Query::family(family, desc))
    }
    fn standard(&mut self, desc: &FontDesc) -> Option<FaceId> {
        self.face_answer(Query::standard(desc))
    }
    fn fallback_for_char(
        &mut self,
        desc: &FontDesc,
        ch: u32,
        primary: FaceId,
        emoji: bool,
    ) -> Option<FaceId> {
        self.face_answer(Query::fallback(desc, ch, primary, emoji))
    }
    fn last_resort(&mut self, desc: &FontDesc) -> Option<FaceId> {
        self.face_answer(Query::last_resort(desc))
    }
    fn variable_face(&mut self, family: &str) -> Option<FaceId> {
        self.face_answer(Query::variable(family))
    }
    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        self.answer(Query::em_advance(face, size))?.advance
    }
}

/// Serves queries from a registry on the calling thread (`pm-cli`, tests):
/// the registry answers each batch directly, with its own font bytes.
///
/// The caller guarantees the thread: [`DirectServer`] is `Send + Sync` so it
/// fits [`FontServer`], but the registry inside is only ever touched from
/// the thread that created it. Every call checks the thread; the registry is
/// dropped there, or leaked when the last reference goes away on another
/// thread (never dropped off its thread). Off its thread, `generation`
/// answers the generation last read on it.
pub struct DirectServer<R> {
    inner: std::mem::ManuallyDrop<std::sync::Mutex<R>>,
    thread: std::thread::ThreadId,
    generation: Box<dyn Fn(&R) -> u64 + Send + Sync>,
    /// The registry's generation as last read on its thread.
    last_generation: std::sync::atomic::AtomicU64,
}

// SAFETY: the registry is used and dropped only on the thread that created
// the server (checked on every call and in `drop`); the mutex makes the
// access exclusive.
unsafe impl<R> Send for DirectServer<R> {}
unsafe impl<R> Sync for DirectServer<R> {}

impl<R> Drop for DirectServer<R> {
    fn drop(&mut self) {
        if std::thread::current().id() == self.thread {
            // SAFETY: dropped once, here, on the registry's thread.
            unsafe { std::mem::ManuallyDrop::drop(&mut self.inner) };
        }
        // Off its thread the registry is leaked: dropping it there is not
        // allowed (AppKit objects belong to their thread).
    }
}

impl<R: FontResolver + 'static> DirectServer<R> {
    /// `generation` reads the registry's generation.
    pub fn new(registry: R, generation: impl Fn(&R) -> u64 + Send + Sync + 'static) -> Self {
        let last = generation(&registry);
        DirectServer {
            inner: std::mem::ManuallyDrop::new(std::sync::Mutex::new(registry)),
            thread: std::thread::current().id(),
            generation: Box::new(generation),
            last_generation: std::sync::atomic::AtomicU64::new(last),
        }
    }

    /// The registry (on the creating thread only).
    pub fn with<T>(&self, f: impl FnOnce(&mut R) -> T) -> T {
        assert_eq!(
            std::thread::current().id(),
            self.thread,
            "a direct font server is used on the thread that made it"
        );
        let mut r = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let out = f(&mut r);
        self.note_generation(&r);
        out
    }

    fn note_generation(&self, r: &R) {
        self.last_generation
            .store((self.generation)(r), std::sync::atomic::Ordering::SeqCst);
    }
}

/// [`Served`] for `queries` from a registry on this thread: its answers and
/// the bytes of the faces they name.
pub fn serve_from(resolver: &mut dyn FontResolver, generation: u64, queries: &[Query]) -> Served {
    let answers = crate::answers::answer_batch(resolver, queries);
    let mut fonts: Vec<ServedFont> = Vec::new();
    for rec in &answers.faces {
        if fonts
            .iter()
            .any(|f| f.blob == rec.blob && f.index == rec.index)
        {
            continue;
        }
        let info = resolver.face(rec.id);
        fonts.push(ServedFont {
            blob: rec.blob.clone(),
            index: rec.index,
            data: info.data.clone(),
            read_index: info.index,
        });
    }
    Served {
        generation,
        answers,
        fonts,
    }
}

impl<R: FontResolver + 'static> FontServer for DirectServer<R> {
    fn serve(&self, queries: &[Query]) -> Result<Served, String> {
        if std::thread::current().id() != self.thread {
            return Err("a direct font server is used on the thread that made it".into());
        }
        let mut r = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let generation = (self.generation)(&r);
        self.last_generation
            .store(generation, std::sync::atomic::Ordering::SeqCst);
        Ok(serve_from(&mut *r, generation, queries))
    }

    fn generation(&self) -> u64 {
        if std::thread::current().id() != self.thread {
            return self
                .last_generation
                .load(std::sync::atomic::Ordering::SeqCst);
        }
        let r = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.note_generation(&r);
        (self.generation)(&r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FaceFacts;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn info(blob: &str) -> FaceInfo {
        FaceInfo {
            data: FontBytes::new(vec![1, 2, 3]),
            index: 0,
            blob: blob.into(),
            coords: vec![],
            synthetic_bold: false,
            synthetic_italic: false,
            custom: false,
            web: None,
            synthesized_hyphens: [None, None],
            postscript_name: blob.into(),
            typeface_key: blob.into(),
            fallback_key: None,
            facts: FaceFacts::default(),
        }
    }

    /// Family "A" is face 0; a CJK fallback is face 1; em advances echo.
    struct Registry(Vec<FaceInfo>);

    impl FontResolver for Registry {
        fn face(&self, id: FaceId) -> &FaceInfo {
            &self.0[id as usize]
        }
        fn family(&mut self, family: &str, _: &FontDesc) -> Option<FaceId> {
            (family == "A").then_some(0)
        }
        fn standard(&mut self, _: &FontDesc) -> Option<FaceId> {
            Some(0)
        }
        fn fallback_for_char(
            &mut self,
            _: &FontDesc,
            ch: u32,
            _: FaceId,
            _: bool,
        ) -> Option<FaceId> {
            (ch >= 0x4e00).then_some(1)
        }
        fn last_resort(&mut self, _: &FontDesc) -> Option<FaceId> {
            None
        }
        fn variable_face(&mut self, _: &str) -> Option<FaceId> {
            None
        }
        fn em_advance(&mut self, _: FaceId, size: f32) -> Option<f32> {
            Some(size + 1.0)
        }
    }

    /// A server on another "thread": counts round trips and can fail.
    struct Remote {
        registry: std::sync::Mutex<Registry>,
        generation: AtomicU64,
        trips: AtomicU64,
        fail: std::sync::atomic::AtomicBool,
    }

    impl FontServer for Remote {
        fn serve(&self, queries: &[Query]) -> Result<Served, String> {
            self.trips.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err("shutting down".into());
            }
            let mut r = self.registry.lock().unwrap();
            Ok(serve_from(
                &mut *r,
                self.generation.load(Ordering::SeqCst),
                queries,
            ))
        }
        fn generation(&self) -> u64 {
            self.generation.load(Ordering::SeqCst)
        }
    }

    fn desc() -> FontDesc {
        FontDesc {
            weight: 400.0,
            italic: false,
            size: 16.0,
        }
    }

    fn remote() -> Arc<Remote> {
        Arc::new(Remote {
            registry: std::sync::Mutex::new(Registry(vec![info("a"), info("cjk")])),
            generation: AtomicU64::new(3),
            trips: AtomicU64::new(0),
            fail: Default::default(),
        })
    }

    #[test]
    fn answers_like_the_registry_and_asks_each_query_once() {
        let server = remote();
        let mut r = ServedResolver::new(server.clone());
        let mut direct = Registry(vec![info("a"), info("cjk")]);
        for _ in 0..3 {
            assert_eq!(r.family("A", &desc()), direct.family("A", &desc()));
            assert_eq!(r.family("B", &desc()), None);
            assert_eq!(
                r.fallback_for_char(&desc(), 0x4e00, 0, false),
                direct.fallback_for_char(&desc(), 0x4e00, 0, false)
            );
            assert_eq!(r.em_advance(0, 12.0), Some(13.0));
        }
        assert_eq!(server.trips.load(Ordering::SeqCst), 4);
        assert_eq!(r.face(1).blob, "cjk");
        assert_eq!(r.face(1).data.as_slice(), &[1, 2, 3]);
        assert!(r.take_failures().is_empty());
        assert!(r.is_current());
    }

    #[test]
    fn prefetch_is_one_round_trip() {
        let server = remote();
        let mut r = ServedResolver::new(server.clone());
        r.prefetch(&[Query::family("A", &desc()), Query::standard(&desc())]);
        assert_eq!(server.trips.load(Ordering::SeqCst), 1);
        assert_eq!(r.family("A", &desc()), Some(0));
        assert_eq!(r.standard(&desc()), Some(0));
        assert_eq!(server.trips.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_new_description_asks_its_family_list_in_one_round_trip() {
        let server = remote();
        let mut r = ServedResolver::new(server.clone());
        let at = |size: f32| FontDesc { size, ..desc() };
        // The first description learns the list: three round trips.
        for name in ["A", "B"] {
            r.family(name, &at(16.0));
        }
        r.standard(&at(16.0));
        let first = server.trips.load(Ordering::SeqCst);
        assert!(first <= 3, "{first}");
        // A new size (an animated font size) asks for all of it at once.
        assert_eq!(r.family("A", &at(17.5)), Some(0));
        assert_eq!(r.family("B", &at(17.5)), None);
        assert_eq!(r.standard(&at(17.5)), Some(0));
        assert_eq!(server.trips.load(Ordering::SeqCst), first + 1);
        assert!(r.take_failures().is_empty());
    }

    #[test]
    fn a_failed_round_trip_is_reported_not_guessed() {
        let server = remote();
        server.fail.store(true, Ordering::SeqCst);
        let mut r = ServedResolver::new(server.clone());
        assert_eq!(r.family("A", &desc()), None);
        let failures = r.take_failures();
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].contains("shutting down"));
        // Not cached: the next layout asks again.
        server.fail.store(false, Ordering::SeqCst);
        assert_eq!(r.family("A", &desc()), Some(0));
        assert!(r.take_failures().is_empty());
    }

    #[test]
    fn a_new_generation_makes_the_resolver_stale() {
        let server = remote();
        let mut r = ServedResolver::new(server.clone());
        assert!(r.is_current());
        r.family("A", &desc());
        assert!(r.is_current());
        server.generation.store(4, Ordering::SeqCst);
        assert!(!r.is_current());
        let mut fresh = ServedResolver::new(server.clone());
        fresh.family("A", &desc());
        assert!(fresh.is_current());
        // A reply from a later generation than the resolver's first one.
        let mut mixed = ServedResolver::new(server.clone());
        server.generation.store(5, Ordering::SeqCst);
        mixed.family("A", &desc());
        server.generation.store(6, Ordering::SeqCst);
        mixed.family("B", &desc());
        server.generation.store(5, Ordering::SeqCst);
        assert!(!mixed.is_current());
    }

    #[test]
    fn a_direct_server_refuses_other_threads() {
        let server = Arc::new(DirectServer::new(Registry(vec![info("a")]), |_| 7));
        let mut r = ServedResolver::new(server.clone());
        assert_eq!(r.family("A", &desc()), Some(0));
        assert_eq!(server.generation(), 7);
        let s = server.clone();
        let other = std::thread::spawn(move || s.serve(&[Query::standard(&desc())]).is_err())
            .join()
            .unwrap();
        assert!(other);
        // Off its thread the generation is the one last read on it, and the
        // last reference dropped there leaks the registry, never drops it.
        struct Tracked(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Tracked {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        struct WithDrop(Registry, #[allow(dead_code)] Tracked);
        impl FontResolver for WithDrop {
            fn face(&self, id: FaceId) -> &FaceInfo {
                self.0.face(id)
            }
            fn family(&mut self, family: &str, d: &FontDesc) -> Option<FaceId> {
                self.0.family(family, d)
            }
            fn standard(&mut self, d: &FontDesc) -> Option<FaceId> {
                self.0.standard(d)
            }
            fn fallback_for_char(
                &mut self,
                d: &FontDesc,
                ch: u32,
                p: FaceId,
                e: bool,
            ) -> Option<FaceId> {
                self.0.fallback_for_char(d, ch, p, e)
            }
            fn last_resort(&mut self, d: &FontDesc) -> Option<FaceId> {
                self.0.last_resort(d)
            }
            fn variable_face(&mut self, family: &str) -> Option<FaceId> {
                self.0.variable_face(family)
            }
        }
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server = Arc::new(DirectServer::new(
            WithDrop(Registry(vec![info("a")]), Tracked(dropped.clone())),
            |_| 9,
        ));
        let s = server.clone();
        drop(server);
        let g = std::thread::spawn(move || {
            let g = s.generation();
            drop(s);
            g
        })
        .join()
        .unwrap();
        assert_eq!(g, 9);
        assert!(
            !dropped.load(Ordering::SeqCst),
            "not dropped off its thread"
        );
        let here = DirectServer::new(
            WithDrop(Registry(vec![info("a")]), Tracked(dropped.clone())),
            |_| 1,
        );
        drop(here);
        assert!(dropped.load(Ordering::SeqCst), "dropped on its thread");
    }
}
