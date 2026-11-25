//! Sol's item 3: turn-scoped controls never reach a later turn, and cancel
//! retires a fetch the core is waiting on for its response headers.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pm_aero_core::{Disposition, Error, Host, Outcome, Purpose, Runtime, Steered, StopReason};
use support::{Driver, Model, Reply, Seen, config, on_worker, sse_text};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";

struct NoTools;
impl Host for NoTools {}

/// Answers the first `answered` model requests, then holds every later one
/// (no status line, so the core waits on its headers).
fn answers_then_holds(answered: usize) -> Model {
    let seen = Arc::new(AtomicUsize::new(0));
    Arc::new(move |_| {
        if seen.fetch_add(1, Ordering::SeqCst) < answered {
            Reply::Sse(sse_text("ok"))
        } else {
            Reply::Hold
        }
    })
}

fn held_fetch(driver: &mut Driver, turn: u64) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        driver.pump(10);
        if let Some(fetch) = driver
            .fetches
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|f| f.turn == turn && f.purpose == Purpose::Model && f.url == ENDPOINT)
        {
            return fetch.fetch;
        }
        assert!(Instant::now() < deadline, "no model fetch for turn {turn}");
    }
}

#[test]
fn a_late_control_never_reaches_a_later_turn() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let runtime = Runtime::create(&config(ENDPOINT, &[]), Arc::new(NoTools)).expect("create");
        let mut driver = Driver::new(runtime, answers_then_holds(1), ENDPOINT);
        driver.open();
        let (first, reason) = driver.turn("one", "one-1");
        assert_eq!(reason, StopReason::EndTurn);
        let second = driver.runtime.prompt("two", "two-1").expect("prompt");
        held_fetch(&mut driver, second);
        // Sol's scenario: a cancel, steer and withdraw meant for the first
        // turn, arriving after the second started.
        let remote = driver.remote();
        assert_eq!(remote.cancel(first), Ok(Outcome::Stale));
        assert_eq!(
            driver.runtime.steer(first, "late", "late-1"),
            Ok(Steered::Stale)
        );
        assert_eq!(driver.runtime.withdraw(first, "late-1"), Err(Error::Stale));
        for _ in 0..20 {
            assert!(
                !driver
                    .pump(10)
                    .iter()
                    .any(|e| matches!(e, Seen::TurnEnd { turn, .. } if *turn == second)),
                "a stale cancel ended the later turn"
            );
        }
        // The cancel meant for the second turn does end it.
        assert_eq!(remote.cancel(second), Ok(Outcome::Applied));
        match driver.until(
            10,
            |e| matches!(e, Seen::TurnEnd { turn, .. } if *turn == second),
        ) {
            Seen::TurnEnd { reason, .. } => assert_eq!(reason, StopReason::Cancelled),
            _ => unreachable!(),
        }
        assert_eq!(remote.cancel(second), Ok(Outcome::Stale));
        driver.runtime.close();
    });
}

#[test]
fn cancel_retires_a_fetch_waiting_for_its_headers() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let runtime = Runtime::create(&config(ENDPOINT, &[]), Arc::new(NoTools)).expect("create");
        let mut driver = Driver::new(runtime, answers_then_holds(0), ENDPOINT);
        driver.open();
        let turn = driver.runtime.prompt("hold", "hold-1").expect("prompt");
        // A POST is taken and its response headers withheld.
        let fetch = held_fetch(&mut driver, turn);
        let remote = driver.remote();
        assert_eq!(remote.fetch_disposition(fetch), Disposition::Active);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(remote.cancel(turn), Ok(Outcome::Applied));
        // Retired at once, with a wake, and the turn ends cancelled.
        assert_eq!(remote.fetch_disposition(fetch), Disposition::Retired);
        let started = Instant::now();
        match driver.until(
            5,
            |e| matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn),
        ) {
            Seen::TurnEnd { reason, .. } => assert_eq!(reason, StopReason::Cancelled),
            _ => unreachable!(),
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
        // The host's late answer is refused: abort the HTTP request.
        assert_eq!(remote.fetch_status(fetch, 200, None), Ok(Outcome::Stale));
        assert_eq!(remote.fetch_push(fetch, b"data: x\n\n"), Ok(Outcome::Stale));
        // The session goes on.
        driver.model = Arc::new(|_| Reply::Sse(sse_text("after")));
        let (next, reason) = driver.turn("again", "again-1");
        assert_eq!(reason, StopReason::EndTurn);
        assert_eq!(driver.text_of(next), "after");
        driver.runtime.close();
    });
}

#[test]
fn the_fetch_stream_keeps_its_order() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let runtime = Runtime::create(&config(ENDPOINT, &[]), Arc::new(NoTools)).expect("create");
        let mut driver = Driver::new(runtime, answers_then_holds(0), ENDPOINT);
        driver.open();
        let turn = driver.runtime.prompt("order", "order-1").expect("prompt");
        let fetch = held_fetch(&mut driver, turn);
        let remote = driver.remote();
        // Bytes before the status, and a chunk that can never fit.
        assert_eq!(remote.fetch_push(fetch, b"early"), Err(Error::State));
        assert_eq!(
            remote.fetch_push(fetch, &vec![b'x'; (8 << 20) + 1]),
            Err(Error::TooLarge)
        );
        assert_eq!(remote.fetch_status(fetch, 200, None), Ok(Outcome::Applied));
        assert_eq!(remote.fetch_status(fetch, 200, None), Err(Error::State));
        support::push_all(remote, fetch, sse_text("ordered").as_bytes());
        assert_eq!(remote.fetch_finish(fetch, true), Ok(Outcome::Applied));
        assert_eq!(remote.fetch_finish(fetch, true), Ok(Outcome::Stale));
        driver.until(
            10,
            |e| matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn),
        );
        assert_eq!(driver.text_of(turn), "ordered");
        driver.runtime.close();
    });
}
