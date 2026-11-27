//! Close and memory release: closing runtimes gives back their threads and
//! descriptors, a session's memory grows over 1,000 tool steps only with the
//! history it keeps (fx's long-turn memory method), and close releases it.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use pm_aero_core::{Config, Host, Runtime, StopReason, Tool, ToolLease};
use support::{Driver, Model, Reply, config, on_worker, sse_text, sse_tool_calls, thread_count};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;
const LOOKUP: [Tool<'static>; 1] = [Tool {
    name: "lookup",
    description: "Looks something up.",
    input_schema_json: SCHEMA,
    idempotent: true,
    writes: false,
}];

/// Answers every call at once, from the callback.
struct Answers;
impl Host for Answers {
    fn tool_call(&self, call: ToolLease) {
        let _ = call.answer("found", false);
    }
}

fn descriptors() -> usize {
    std::fs::read_dir("/dev/fd")
        .map(|dir| dir.count())
        .unwrap_or(0)
}

/// Bytes malloc holds for live allocations (RSS also counts what the
/// allocator caches after a free, so it says nothing about leaks).
fn in_use() -> u64 {
    // SAFETY: an all-zero malloc_statistics_t is valid; a null zone means all.
    unsafe {
        let mut stats: libc::malloc_statistics_t = std::mem::zeroed();
        libc::malloc_zone_statistics(std::ptr::null_mut(), &mut stats);
        stats.size_in_use as u64
    }
}

/// Answers every call at once with a changing ~1 KiB result, as fx's
/// long-turn memory method reads a changing file each step.
struct Kilobyte(std::sync::atomic::AtomicUsize);
impl Host for Kilobyte {
    fn tool_call(&self, call: ToolLease) {
        let n = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = call.answer(&format!("{n:06} {}", "x".repeat(1017)), false);
    }
}

/// One tool call, then text: two model steps a turn.
fn two_steps() -> Model {
    Arc::new(|call| {
        if call.body.contains("found") {
            Reply::Sse(sse_text("done"))
        } else {
            Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"a"}"#]))
        }
    })
}

#[test]
fn closing_gives_back_threads_and_descriptors_and_steps_do_not_grow_memory() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        // Warm the process-wide pieces (io, allocator pools) first.
        {
            let runtime =
                Runtime::create(&config(ENDPOINT, &LOOKUP), Arc::new(Answers)).expect("create");
            let mut driver = Driver::new(runtime, two_steps(), ENDPOINT);
            driver.open();
            driver.turn("warm", "w-1");
            driver.runtime.close();
        }
        std::thread::sleep(Duration::from_millis(200));
        let (threads, fds) = (thread_count(), descriptors());
        let mut drivers: Vec<Driver> = (0..16)
            .map(|_| {
                let runtime =
                    Runtime::create(&config(ENDPOINT, &LOOKUP), Arc::new(Answers)).expect("create");
                Driver::new(runtime, two_steps(), ENDPOINT)
            })
            .collect();
        for (index, driver) in drivers.iter_mut().enumerate() {
            driver.open();
            let (_, reason) = driver.turn("look it up", &format!("m-{index}"));
            assert_eq!(reason, StopReason::EndTurn);
        }
        assert!(thread_count() > threads, "each runtime has its core thread");
        for driver in drivers {
            driver.runtime.close();
        }
        // The fake model's serving threads finish on their own.
        let deadline = Instant::now() + Duration::from_secs(5);
        while (thread_count() > threads || descriptors() > fds) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            thread_count() <= threads,
            "threads: {} after, {threads} before",
            thread_count()
        );
        assert!(
            descriptors() <= fds,
            "descriptors: {} after, {fds} before",
            descriptors()
        );

        // fx's long-turn memory method: one turn of 1,000 tool steps, each
        // result ~1 KiB. What the core holds grows with the history it keeps
        // (linearly), and closing the session gives all of it back.
        let base = in_use();
        let samples = Arc::new(std::sync::Mutex::new(Vec::<(usize, usize, u64)>::new()));
        let recorded = samples.clone();
        let model: Model = Arc::new(move |call| {
            recorded
                .lock()
                .unwrap()
                .push((call.index, call.body.len(), in_use()));
            if call.index >= 1000 {
                Reply::Sse(sse_text("done"))
            } else {
                Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"a"}"#]))
            }
        });
        let cfg = Config {
            max_agent_steps: 1001,
            ..config(ENDPOINT, &LOOKUP)
        };
        let runtime =
            Runtime::create(&cfg, Arc::new(Kilobyte(Default::default()))).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let turn = driver
            .runtime
            .prompt("read the file a thousand times", "m-1")
            .expect("prompt");
        loop {
            let new = driver.pump(5);
            // The driver's own record is the test's memory, not the core's.
            driver.seen.clear();
            driver.fetches.lock().unwrap().clear();
            if new
                .iter()
                .any(|e| matches!(e, support::Seen::TurnEnd { turn: t, .. } if *t == turn))
            {
                break;
            }
        }
        driver.runtime.close();
        std::thread::sleep(Duration::from_millis(300));
        let after_close = in_use();
        let samples = samples.lock().unwrap();
        assert_eq!(samples.len(), 1001, "1,000 tool steps and the reply");
        let (_, body_100, at_100) = samples[100];
        let (_, _, at_550) = samples[550];
        let (_, body_1000, at_1000) = samples[1000];
        let history = (body_1000 - body_100) as u64;
        let (first, second) = (
            at_550.saturating_sub(at_100),
            at_1000.saturating_sub(at_550),
        );
        eprintln!(
            "steps 100..1000: history +{history} B, in use +{} B (+{first} then +{second}); after close {after_close} B vs {base} B before",
            first + second
        );
        // fx's gate for this method is a peak under 512 MiB at 500 steps;
        // ours holds 1,000 steps under 128 MiB of growth, and the growth is
        // linear in the steps (a per-step copy of the history would make
        // the second half ~2.4x the first).
        assert!(
            first + second < 128 << 20,
            "in use grew {} B",
            first + second
        );
        assert!(
            second * 2 < first * 3,
            "growth is not linear: +{first} then +{second}"
        );
        assert!(
            after_close < base + (4 << 20),
            "close kept {} B",
            after_close.saturating_sub(base)
        );
    });
}
