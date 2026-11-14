//! The helper pool: shared helpers, results in order, no caller waiting
//! for another caller's work, no queue behind busy helpers, and a call that
//! panicked never takes a helper with it.

use super::pool;
use std::sync::atomic::Ordering;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Holds every item of a call until opened.
#[derive(Default)]
struct Gate {
    state: Mutex<(usize, bool)>,
    turn: Condvar,
}
impl Gate {
    fn enter(&self) {
        let mut g = self.state.lock().unwrap();
        g.0 += 1;
        self.turn.notify_all();
        while !g.1 {
            g = self.turn.wait(g).unwrap();
        }
    }
    fn started(&self, n: usize) -> bool {
        let g = self.state.lock().unwrap();
        let (g, _) = self
            .turn
            .wait_timeout_while(g, Duration::from_secs(2), |g| g.0 < n)
            .unwrap();
        g.0 >= n
    }
    fn open(&self) {
        self.state.lock().unwrap().1 = true;
        self.turn.notify_all();
    }
}

/// Opens the gate however a test ends.
struct Opens<'a>(&'a Gate);
impl Drop for Opens<'_> {
    fn drop(&mut self) {
        self.0.open();
    }
}

/// Calls share one set of helper threads: none is started per call, and
/// results come back in order whoever computed them.
#[test]
fn calls_reuse_the_same_helpers_and_keep_their_order() {
    let ids = || {
        super::map(256, |i| {
            std::thread::sleep(Duration::from_micros(50));
            (i, std::thread::current().id())
        })
    };
    let mut seen = std::collections::HashSet::new();
    for _ in 0..8 {
        let out = ids();
        assert!(out.iter().enumerate().all(|(at, (i, _))| at == *i));
        seen.extend(out.into_iter().map(|(_, id)| id));
    }
    assert!(
        seen.len() <= pm_model::parallel::workers(),
        "{} threads for {} workers",
        seen.len(),
        pm_model::parallel::workers()
    );
}

/// Two renderers share the helpers. A second caller whose helpers are
/// busy with the first caller's work does its own share and returns: it
/// never waits for the other caller's work (it waited for all of it).
#[test]
fn a_second_caller_never_waits_for_the_first_callers_work() {
    let workers = pm_model::parallel::workers();
    if workers < 2 {
        return;
    }
    // Another test asking for the helpers at the same instant makes the
    // first call run alone: that attempt proves nothing, try again.
    for _ in 0..20 {
        let gate = Gate::default();
        let attempt = std::thread::scope(|s| {
            let opens = Opens(&gate);
            let first = s.spawn(|| {
                super::map(4 * workers, |i| {
                    gate.enter();
                    i
                })
            });
            // The first caller and at least one helper are held.
            if !gate.started(2) {
                return None;
            }
            let second = s.spawn(|| super::map(8, |i| i * 2));
            let deadline = Instant::now() + Duration::from_secs(5);
            while !second.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            let on_its_own = second.is_finished();
            drop(opens);
            assert_eq!(first.join().unwrap(), (0..4 * workers).collect::<Vec<_>>());
            Some((on_its_own, second.join().unwrap()))
        });
        if let Some((on_its_own, second)) = attempt {
            assert!(
                on_its_own,
                "the second call waited for the first call's work"
            );
            assert_eq!(second, (0..8).map(|i| i * 2).collect::<Vec<_>>());
            return;
        }
    }
    panic!("no helper ever started the first call");
}

/// While the helpers are busy with one caller's work, another caller's
/// calls run inline: none is queued behind them (each was, and the
/// closed calls piled up until the helpers got to them).
#[test]
fn calls_never_queue_behind_busy_helpers() {
    let workers = pm_model::parallel::workers();
    if workers < 2 {
        return;
    }
    for _ in 0..20 {
        let gate = Gate::default();
        let queued = std::thread::scope(|s| {
            let opens = Opens(&gate);
            let first = s.spawn(|| {
                super::map(4 * workers, |i| {
                    gate.enter();
                    i
                })
            });
            if !gate.started(workers) {
                return None;
            }
            let before = pool::QUEUED.load(Ordering::Acquire);
            for _ in 0..200 {
                assert_eq!(super::map(8, |i| i), (0..8).collect::<Vec<_>>());
            }
            let queued = pool::QUEUED.load(Ordering::Acquire).saturating_sub(before);
            drop(opens);
            first.join().unwrap();
            Some(queued)
        });
        if let Some(queued) = queued {
            assert!(
                queued < workers,
                "{queued} tasks queued behind busy helpers"
            );
            return;
        }
    }
    panic!("the helpers never all started the first call");
}

/// A call whose task panicked is closed with its lock poisoned; a helper
/// that takes its task afterwards skips it and keeps serving later calls.
#[test]
fn a_helper_survives_a_call_poisoned_by_a_panic() {
    if pm_model::parallel::workers() < 2 {
        return;
    }
    let started = Instant::now();
    loop {
        match pool::poisoned_call_then_probe() {
            Some(ran) => {
                assert!(ran, "the helper died on the poisoned call");
                return;
            }
            None => {
                assert!(
                    started.elapsed() < Duration::from_secs(30),
                    "no idle helper"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
