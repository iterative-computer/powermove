//! Fonts for the engine's render threads (`docs/engine/text.md`, "Fonts on
//! render threads").
//!
//! The font registry lives on the utility process's JavaScript thread (the
//! process's main thread, which AppKit's font manager needs), in
//! [`super::FontService`]. Render threads (the viewer presenter, the job
//! engine's lanes) lay text out and draw it with the registry's answers and
//! full font bytes: each render thread's `pm_render::text::served::
//! ServedFonts` asks the [`FontHub`] for the queries it has not seen; the
//! hub queues them, wakes the JavaScript thread (the same wake the job
//! service uses, which makes the utility process call `pump`), and waits.
//! [`FontHub::drain`] runs on the JavaScript thread and answers every
//! queued batch from the service.
//!
//! The hub's generation (`FontServer::generation`) moves whenever the
//! registry's generation moves (project fonts or sources changed, a new
//! generation base on a new port) or main sends different font cache seeds,
//! so render threads start their layout over and their raster keys change.
//!
//! The JavaScript thread never waits for a render thread while it holds a
//! request (a render thread only waits for it), so the only way a render
//! thread's wait could outlast the service is shutdown: [`FontHub::shutdown`]
//! ends every wait, and the engine host calls it before the presenter and
//! job lanes are joined.
//!
//! Until main has finished replaying to a font port (`Op::Open`, the project
//! fonts, then the seeds, which go last), requests wait for it: a frame is
//! never laid out without the project fonts and seeds main is about to send. A host that
//! never opens one makes the requests fail after [`OPEN_TIMEOUT`], and the
//! layers that needed them are refused (named), never drawn with another
//! font.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pm_text::answers::Query;
use pm_text::compat::FontCacheSeed;
use pm_text::served::{FontServer, Served};

/// How long a render thread waits for main to open the font port.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a render thread waits for one answer once the port is open
/// (the first fallback manifest holds the JavaScript thread for about
/// 0.6 s; answers are milliseconds).
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// Wakes the utility process's JavaScript thread so it calls `pump`.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct State {
    next: u64,
    outbox: VecDeque<(u64, Vec<Query>)>,
    replies: HashMap<u64, Result<Served, String>>,
    /// Main opened a font port (seeds and project fonts are in).
    open: bool,
    shutdown: bool,
    seeds: super::SeedTable,
    /// The window each client port belongs to (main names it on connect).
    port_owners: HashMap<u32, u32>,
    /// The registry generation last noted.
    registry: Option<u64>,
    served: u64,
    round_trips: u64,
}

/// The render threads' side of the font service.
pub struct FontHub {
    state: Mutex<State>,
    changed: Condvar,
    wake: Mutex<Option<Wake>>,
    generation: AtomicU64,
    open_timeout: Duration,
}

impl Default for FontHub {
    fn default() -> Self {
        Self::with_timeout(OPEN_TIMEOUT)
    }
}

/// Counters for the engine's status.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubStats {
    pub generation: u64,
    pub open: bool,
    pub queued: usize,
    pub served: u64,
    pub round_trips: u64,
    pub seeds: usize,
    pub seed_owners: usize,
}

impl FontHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// A hub whose requests wait at most `open_timeout` for an open port.
    pub fn with_timeout(open_timeout: Duration) -> Self {
        FontHub {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            wake: Mutex::new(None),
            generation: AtomicU64::new(0),
            open_timeout,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets how the JavaScript thread is woken for queued requests.
    pub fn set_wake(&self, wake: Wake) {
        *self.wake.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake);
    }

    fn wake(&self) {
        let wake = self.wake.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(wake) = wake {
            wake();
        }
    }

    fn bump(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Main opened a font port (on the JavaScript thread, after the service
    /// handled `Op::Open`). Requests waiting for it go out at the next
    /// [`FontHub::drain`].
    pub fn opened(&self) {
        self.lock().open = true;
        self.changed.notify_all();
        self.wake();
    }

    /// The registry's generation after a request the service handled.
    pub fn note_registry_generation(&self, generation: u64) {
        let mut st = self.lock();
        if st.registry != Some(generation) {
            let first = st.registry.is_none();
            st.registry = Some(generation);
            drop(st);
            if !first {
                self.bump();
            }
        }
    }

    /// A client port of window `owner` connected (`None`: main's own
    /// ports and peers), or went away.
    pub fn set_port_owner(&self, port: u32, owner: Option<u32>) {
        let mut st = self.lock();
        match owner {
            Some(o) => st.port_owners.insert(port, o),
            None => st.port_owners.remove(&port),
        };
    }

    /// Main's font cache seeds.
    pub fn set_seeds(&self, seeds: super::SeedTable) {
        let mut st = self.lock();
        if st.seeds != seeds {
            st.seeds = seeds;
            drop(st);
            self.bump();
        }
    }

    /// Ends every wait; later requests fail at once.
    pub fn shutdown(&self) {
        self.lock().shutdown = true;
        self.changed.notify_all();
    }

    /// Whether render threads are waiting for answers.
    pub fn has_requests(&self) -> bool {
        !self.lock().outbox.is_empty()
    }

    pub fn stats(&self) -> HubStats {
        let st = self.lock();
        HubStats {
            generation: self.generation.load(Ordering::SeqCst),
            open: st.open,
            queued: st.outbox.len(),
            served: st.served,
            round_trips: st.round_trips,
            seeds: st.seeds.fallback.len(),
            seed_owners: st.seeds.owners.len(),
        }
    }

    /// Answers every queued batch with `answer` (on the JavaScript thread;
    /// `answer` is the font service, or a refusal when there is none).
    /// Nothing is answered before the port is open.
    pub fn drain(&self, mut answer: impl FnMut(&[Query]) -> Result<Served, String>) -> usize {
        let batches: Vec<(u64, Vec<Query>)> = {
            let mut st = self.lock();
            if !st.open || st.outbox.is_empty() {
                return 0;
            }
            st.outbox.drain(..).collect()
        };
        let n = batches.len();
        let generation = self.generation.load(Ordering::SeqCst);
        let replies: Vec<(u64, Result<Served, String>)> = batches
            .into_iter()
            .map(|(token, queries)| {
                let reply = answer(&queries).map(|mut s| {
                    s.generation = generation;
                    s
                });
                (token, reply)
            })
            .collect();
        let mut st = self.lock();
        st.served += n as u64;
        st.replies.extend(replies);
        drop(st);
        self.changed.notify_all();
        n
    }
}

impl FontServer for FontHub {
    fn serve(&self, queries: &[Query]) -> Result<Served, String> {
        let started = Instant::now();
        let mut st = self.lock();
        if st.shutdown {
            return Err("the engine is shutting down".into());
        }
        st.next += 1;
        st.round_trips += 1;
        let token = st.next;
        st.outbox.push_back((token, queries.to_vec()));
        let open = st.open;
        drop(st);
        if open {
            self.wake();
        }
        let mut st = self.lock();
        loop {
            if let Some(reply) = st.replies.remove(&token) {
                return reply;
            }
            let fail = |st: &mut MutexGuard<'_, State>, why: String| {
                st.outbox.retain(|(t, _)| *t != token);
                Err(why)
            };
            if st.shutdown {
                return fail(&mut st, "the engine is shutting down".into());
            }
            let limit = if st.open {
                ANSWER_TIMEOUT
            } else {
                self.open_timeout
            };
            let left = limit.saturating_sub(started.elapsed());
            if left.is_zero() {
                let why = if st.open {
                    format!(
                        "the font service did not answer within {} s",
                        limit.as_secs()
                    )
                } else {
                    "main has not opened the engine's font port".into()
                };
                return fail(&mut st, why);
            }
            st = self
                .changed
                .wait_timeout(st, left.min(Duration::from_millis(100)))
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
}

#[cfg(target_os = "macos")]
impl pm_render::text::served::SeedSource for FontHub {
    fn seeds(&self, scope: pm_render::text::served::SeedScope) -> Vec<FontCacheSeed> {
        let st = self.lock();
        let port_owner = scope.port.and_then(|p| st.port_owners.get(&p).copied());
        st.seeds.resolve(port_owner, scope.session).to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_text::answers::Answers;

    fn query() -> Query {
        Query::Variable { family: "X".into() }
    }

    #[test]
    fn a_request_waits_for_the_javascript_thread() {
        let hub = Arc::new(FontHub::new());
        let woken = Arc::new(AtomicU64::new(0));
        let w = woken.clone();
        hub.set_wake(Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }));
        hub.opened();
        let h = hub.clone();
        let t = std::thread::spawn(move || h.serve(&[query()]));
        // The JavaScript thread's pump.
        let mut served = 0;
        while served == 0 {
            served = hub.drain(|q| {
                assert_eq!(q, &[query()]);
                Ok(Served {
                    generation: 99,
                    answers: Answers::default(),
                    fonts: vec![],
                })
            });
            std::thread::yield_now();
        }
        let reply = t.join().unwrap().unwrap();
        // The reply carries the hub's generation, not the service's.
        assert_eq!(reply.generation, hub.generation());
        assert!(woken.load(Ordering::SeqCst) >= 1);
        assert_eq!(hub.stats().served, 1);
    }

    #[test]
    fn nothing_is_answered_before_main_opens_the_port() {
        let hub = Arc::new(FontHub::with_timeout(Duration::from_millis(300)));
        let h = hub.clone();
        let t = std::thread::spawn(move || h.serve(&[query()]));
        while !hub.has_requests() {
            std::thread::yield_now();
        }
        assert_eq!(hub.drain(|_| Ok(Served::default())), 0);
        let err = t.join().unwrap().err().unwrap();
        assert!(err.contains("not opened"), "{err}");
        assert!(!hub.has_requests());
    }

    #[test]
    fn requests_queued_before_the_open_are_answered_after_it() {
        let hub = Arc::new(FontHub::new());
        let h = hub.clone();
        let t = std::thread::spawn(move || h.serve(&[query()]));
        while !hub.has_requests() {
            std::thread::yield_now();
        }
        hub.opened();
        assert_eq!(hub.drain(|_| Ok(Served::default())), 1);
        assert!(t.join().unwrap().is_ok());
    }

    #[test]
    fn shutdown_ends_every_wait() {
        let hub = Arc::new(FontHub::new());
        hub.opened();
        let h = hub.clone();
        let t = std::thread::spawn(move || h.serve(&[query()]));
        while !hub.has_requests() {
            std::thread::yield_now();
        }
        hub.shutdown();
        assert!(t.join().unwrap().err().unwrap().contains("shutting down"));
        assert!(hub.serve(&[query()]).is_err());
    }

    #[test]
    fn registry_and_seed_changes_move_the_generation() {
        let hub = FontHub::new();
        let g0 = hub.generation();
        hub.note_registry_generation(5);
        assert_eq!(
            hub.generation(),
            g0,
            "the first generation seen is no change"
        );
        hub.note_registry_generation(5);
        assert_eq!(hub.generation(), g0);
        hub.note_registry_generation(6);
        assert_eq!(hub.generation(), g0 + 1);
        let seed = FontCacheSeed {
            family: "system-ui".into(),
            weight: 400.0,
            italic: false,
            size: 24.0,
            specified: 12.0,
            cache_key: None,
        };
        let table = |seeds: Vec<FontCacheSeed>| crate::fonts::SeedTable {
            fallback: seeds,
            ..Default::default()
        };
        hub.set_seeds(table(vec![seed.clone()]));
        assert_eq!(hub.generation(), g0 + 2);
        hub.set_seeds(table(vec![seed]));
        assert_eq!(hub.generation(), g0 + 2, "the same seeds are no change");
    }

    /// Each window is its own renderer: a view measures with the seeds of
    /// the window its port belongs to, an export with those of the window
    /// main opened its session for, anything else with the fallback.
    #[cfg(target_os = "macos")]
    #[test]
    fn layouts_measure_with_the_seeds_of_their_window() {
        use pm_render::text::served::{SeedScope, SeedSource};
        let seed = |size: f32, specified: f32| FontCacheSeed {
        cache_key: None,
            family: "system-ui".into(),
            weight: 400.0,
            italic: false,
            size,
            specified,
        };
        let retina = vec![seed(24.0, 12.0), seed(26.0, 13.0)];
        let external = vec![seed(12.0, 12.0), seed(13.0, 13.0)];
        let mut all = retina.clone();
        all.extend(external.clone());
        let hub = FontHub::new();
        hub.set_seeds(crate::fonts::SeedTable {
            fallback: all.clone(),
            owners: [(1, external.clone()), (2, retina.clone())].into(),
            sessions: [(40, 2)].into(),
        });
        hub.set_port_owner(11, Some(1));
        hub.set_port_owner(12, Some(2));
        assert_eq!(hub.seeds(SeedScope::port(11)), external, "editor on 1x");
        assert_eq!(hub.seeds(SeedScope::port(12)), retina, "popout on 2x");
        let export = SeedScope {
            port: None,
            session: Some(40),
        };
        assert_eq!(hub.seeds(export), retina, "the export of window 2");
        // A window's port names the window even for a session of another.
        let frames = SeedScope {
            port: Some(11),
            session: Some(40),
        };
        assert_eq!(hub.seeds(frames), external);
        assert_eq!(hub.seeds(SeedScope::port(99)), all, "no window known");
        assert_eq!(hub.seeds(SeedScope::default()), all);
        hub.set_port_owner(11, None);
        assert_eq!(hub.seeds(SeedScope::port(11)), all, "the port went away");
    }
}
