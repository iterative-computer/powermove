//! Tool calls (Sol's items 1 and 8): the input is pinned through the
//! callback and through the lease, concurrency follows the limit and writes
//! run alone, a panicking or dropped lease still answers the core, a
//! cancelled call still needs its answer, and teardown never blocks the
//! dropping thread.

mod support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::{
    Answered, Config, Error, Host, Outcome, Runtime, StopReason, Tool, ToolLease, sys,
};
use support::{Driver, Model, Reply, Seen, config, on_worker, sse_text, sse_tool_calls};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;

fn lookup(idempotent: bool, writes: bool) -> Tool<'static> {
    Tool {
        name: "lookup",
        description: "Looks something up.",
        input_schema_json: SCHEMA,
        idempotent,
        writes,
    }
}

/// Asks for `inputs` as tool calls until a request carries a tool result,
/// then replies with text that quotes whether the result says `marker`.
fn tool_model(inputs: &'static [&'static str]) -> Model {
    Arc::new(move |call| {
        if call.body.contains("\"tool\"")
            || call.body.contains("tool-result")
            || call.body.contains("tool_result")
            || call.index > 0
        {
            let quoted = if call.body.contains("panicked") {
                "saw panic"
            } else if call.body.contains("dropped") {
                "saw drop"
            } else {
                "done"
            };
            Reply::Sse(sse_text(quoted))
        } else {
            Reply::Sse(sse_tool_calls("lookup", inputs))
        }
    })
}

/// Hands every lease to the test.
struct Queue {
    leases: Mutex<Sender<ToolLease>>,
    cancels: Mutex<Vec<(u64, u64)>>,
}

impl Host for Queue {
    fn tool_call(&self, call: ToolLease) {
        let _ = self.leases.lock().unwrap().send(call);
    }
    fn tool_cancel(&self, call: u64, turn: u64) {
        self.cancels.lock().unwrap().push((call, turn));
    }
}

fn queue() -> (Arc<Queue>, Receiver<ToolLease>) {
    let (send, receive) = channel();
    (
        Arc::new(Queue {
            leases: Mutex::new(send),
            cancels: Mutex::new(Vec::new()),
        }),
        receive,
    )
}

fn pump_for_lease(driver: &mut Driver, leases: &Receiver<ToolLease>, secs: u64) -> ToolLease {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        driver.pump(10);
        if let Ok(lease) = leases.try_recv() {
            return lease;
        }
        assert!(
            Instant::now() < deadline,
            "no tool call; saw {:#?}",
            driver.seen
        );
    }
}

/// The reviewer's race: the callback enqueues the call, an executor answers
/// it on another thread at once, and the callback then reads its input.
struct ReadsAfterEnqueue {
    leases: Mutex<Sender<ToolLease>>,
    late_reads: Mutex<Vec<String>>,
}

impl Host for ReadsAfterEnqueue {
    fn tool_call(&self, call: ToolLease) {
        let ptr = call.input_ptr();
        let len = call.input_json().len();
        let _ = self.leases.lock().unwrap().send(call);
        // The executor has answered by now; the core must still hold the
        // input, because this callback has not returned.
        std::thread::sleep(Duration::from_millis(30));
        // SAFETY: the header pins the input through the callback's return.
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        self.late_reads
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(bytes).into_owned());
    }
}

#[test]
fn the_input_is_pinned_through_the_callback_and_the_lease() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (send, leases) = channel();
        let host = Arc::new(ReadsAfterEnqueue {
            leases: Mutex::new(send),
            late_reads: Mutex::new(Vec::new()),
        });
        let tools = [lookup(true, false)];
        let runtime = Runtime::create(&config(ENDPOINT, &tools), host.clone()).expect("create");
        let mut driver = Driver::new(runtime, tool_model(&[r#"{"q":"alpha"}"#]), ENDPOINT);
        driver.open();
        let turn = driver.runtime.prompt("look it up", "t-1").expect("prompt");
        let lease = pump_for_lease(&mut driver, &leases, 20);
        assert_eq!(lease.turn(), turn);
        assert_eq!(lease.name(), "lookup");
        assert!(lease.idempotent() && !lease.writes());
        // Read on another thread (the lease's own window), then answer.
        let executor = std::thread::spawn(move || {
            let input = lease.input_json().to_string();
            (input, lease.answer("found it", false))
        });
        let (input, answered) = executor.join().unwrap();
        assert_eq!(input, r#"{"q":"alpha"}"#);
        assert_eq!(answered, Ok(Answered::Applied));
        driver.until(
            20,
            |e| matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn),
        );
        assert_eq!(driver.text_of(turn), "done");
        assert_eq!(
            host.late_reads.lock().unwrap().as_slice(),
            [r#"{"q":"alpha"}"#.to_string()]
        );
        // Answer outcomes by id: a second answer is stale, an id never issued
        // is invalid.
        let handle = driver.remote().handle();
        let out = sys::fx_tool_output {
            text: sys::fx_str::of("again"),
            images: std::ptr::null(),
            image_count: 0,
            is_error: false,
        };
        // SAFETY: valid pointers for the call.
        unsafe {
            assert_eq!(sys::fx_tool_result(handle, 1, &out), sys::FX_STALE);
            assert_eq!(sys::fx_tool_result(handle, 999, &out), sys::FX_E_INVALID);
        }
        driver.runtime.close();
    });
}

/// Holds each lease 40 ms on its own thread and records the most at once.
struct Concurrency {
    outstanding: AtomicUsize,
    most: AtomicUsize,
    calls: AtomicUsize,
}

impl Host for Concurrency {
    fn tool_call(&self, call: ToolLease) {
        let now = self.outstanding.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        let this = self as *const Concurrency as usize;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            // SAFETY: the host outlives every lease (teardown waits for them).
            let this = unsafe { &*(this as *const Concurrency) };
            this.outstanding.fetch_sub(1, Ordering::SeqCst);
            let _ = call.answer("ok", false);
        });
    }
}

fn concurrency_run(tool: Tool<'static>, limit: u32) -> (usize, usize) {
    let host = Arc::new(Concurrency {
        outstanding: AtomicUsize::new(0),
        most: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
    });
    let tools = [tool];
    let cfg = Config {
        max_concurrent_tools: limit,
        ..config(ENDPOINT, &tools)
    };
    let runtime = Runtime::create(&cfg, host.clone()).expect("create");
    let inputs: &'static [&'static str] = &[
        r#"{"q":"1"}"#,
        r#"{"q":"2"}"#,
        r#"{"q":"3"}"#,
        r#"{"q":"4"}"#,
        r#"{"q":"5"}"#,
        r#"{"q":"6"}"#,
    ];
    let mut driver = Driver::new(runtime, tool_model(inputs), ENDPOINT);
    driver.open();
    let (_, reason) = driver.turn("all of them", "c-1");
    assert_eq!(reason, StopReason::EndTurn);
    driver.runtime.close();
    (
        host.calls.load(Ordering::SeqCst),
        host.most.load(Ordering::SeqCst),
    )
}

#[test]
fn concurrent_calls_follow_the_limit_and_writes_run_alone() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (calls, most) = concurrency_run(lookup(true, false), 2);
        assert_eq!(calls, 6);
        assert_eq!(
            most, 2,
            "idempotent calls run in parallel up to max_concurrent_tools"
        );
        let (calls, most) = concurrency_run(lookup(false, true), 16);
        assert_eq!(calls, 6);
        assert_eq!(most, 1, "a writes tool runs alone");
    });
}

struct Panics;
impl Host for Panics {
    fn tool_call(&self, call: ToolLease) {
        let _keep = call;
        panic!("the tool blew up");
    }
}

/// A panic payload whose own drop panics (Sol's 2a item 2): dropping it on
/// the callback's stack must not abort at the C boundary.
struct Bomb;
impl Drop for Bomb {
    fn drop(&mut self) {
        panic!("the panic payload's drop panics too");
    }
}

struct PanicsWithBomb;
impl Host for PanicsWithBomb {
    fn tool_call(&self, call: ToolLease) {
        let _keep = call;
        std::panic::panic_any(Bomb);
    }
}

struct Drops;
impl Host for Drops {
    fn tool_call(&self, call: ToolLease) {
        drop(call);
    }
}

#[test]
fn a_panicking_or_dropped_tool_still_answers_the_core() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        for (host, expected) in [
            (Arc::new(Panics) as Arc<dyn Host>, "saw panic"),
            (Arc::new(PanicsWithBomb), "saw panic"),
            (Arc::new(Drops), "saw drop"),
        ] {
            let tools = [lookup(true, false)];
            let runtime = Runtime::create(&config(ENDPOINT, &tools), host).expect("create");
            let mut driver = Driver::new(runtime, tool_model(&[r#"{"q":"x"}"#]), ENDPOINT);
            driver.open();
            let (turn, reason) = driver.turn("go", "p-1");
            assert_eq!(reason, StopReason::EndTurn);
            assert_eq!(driver.text_of(turn), expected);
            assert!(
                driver
                    .seen
                    .iter()
                    .any(|e| matches!(e, Seen::ToolEnd { failed: true, .. }))
            );
            driver.runtime.close();
        }
    });
}

#[test]
fn a_cancelled_call_gets_tool_cancel_and_still_needs_its_answer() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, leases) = queue();
        let tools = [lookup(true, false)];
        let runtime = Runtime::create(&config(ENDPOINT, &tools), host.clone()).expect("create");
        let mut driver = Driver::new(runtime, tool_model(&[r#"{"q":"slow"}"#]), ENDPOINT);
        driver.open();
        let turn = driver.runtime.prompt("go", "k-1").expect("prompt");
        let lease = pump_for_lease(&mut driver, &leases, 20);
        assert_eq!(driver.remote().cancel(turn), Ok(Outcome::Applied));
        // tool_cancel arrives after tool_call, for this call and turn.
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.cancels.lock().unwrap().is_empty() {
            driver.pump(10);
            assert!(Instant::now() < deadline, "no tool_cancel");
        }
        assert_eq!(
            host.cancels.lock().unwrap().as_slice(),
            [(lease.call(), turn)]
        );
        // The core still waits for the lease: the turn has not ended.
        for _ in 0..10 {
            assert!(
                !driver
                    .pump(10)
                    .iter()
                    .any(|e| matches!(e, Seen::TurnEnd { .. }))
            );
        }
        // The lease's input is still readable; its answer ends it.
        assert_eq!(lease.input_json(), r#"{"q":"slow"}"#);
        assert_eq!(lease.answer("too late", false), Ok(Answered::Applied));
        match driver.until(20, |e| matches!(e, Seen::TurnEnd { .. })) {
            Seen::TurnEnd { reason, .. } => assert_eq!(reason, StopReason::Cancelled),
            _ => unreachable!(),
        }
        driver.runtime.close();
    });
}

#[test]
fn the_step_limit_is_enforced() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        struct Answers;
        impl Host for Answers {
            fn tool_call(&self, call: ToolLease) {
                let _ = call.answer("again", false);
            }
        }
        let tools = [lookup(true, false)];
        let cfg = Config {
            max_agent_steps: 2,
            ..config(ENDPOINT, &tools)
        };
        let runtime = Runtime::create(&cfg, Arc::new(Answers)).expect("create");
        // A model that always asks for another call.
        let model: Model = Arc::new(|_| Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"more"}"#])));
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let (_, reason) = driver.turn("loop", "s-1");
        assert_eq!(reason, StopReason::MaxSteps);
        driver.runtime.close();
    });
}

/// The step limit arrives typed (P-STEPS): a reply that quotes the core's
/// step-limit notice in a turn that then fails for another reason (here
/// repeated malformed tool input, which libfx ends as refused) is a
/// refusal, not the step limit.
#[test]
fn a_quoted_step_limit_notice_is_not_the_step_limit() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        struct Answers;
        impl Host for Answers {
            fn tool_call(&self, call: ToolLease) {
                let _ = call.answer("ok", false);
            }
        }
        let tools = [lookup(true, false)];
        let runtime =
            Runtime::create(&config(ENDPOINT, &tools), Arc::new(Answers)).expect("create");
        let quote = support::json_string(&format!("The log said: {STEP_LIMIT_NOTICE} "));
        let reply = format!(
            "data: {{\"type\":\"text-delta\",\"id\":\"0\",\"delta\":{quote}}}\n\n{}",
            sse_tool_calls("lookup", &[r#"{"q":"#])
        );
        let model: Model = Arc::new(move |_| Reply::Sse(reply.clone()));
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let (turn, reason) = driver.turn("quote it", "q-1");
        assert!(
            driver.text_of(turn).contains(STEP_LIMIT_NOTICE),
            "the reply quoted the notice: {:#?}",
            driver.seen
        );
        assert_eq!(reason, StopReason::Refusal, "{:#?}", driver.seen);
        driver.runtime.close();
    });
}

/// core/agent/runtime/config.zig, default_step_limit_notice.
const STEP_LIMIT_NOTICE: &str =
    "Agent step limit reached; continue with a follow-up prompt if needed.";

/// Records when the host is dropped (teardown freed it).
struct Tracked {
    leases: Mutex<Sender<ToolLease>>,
    dropped: Arc<AtomicBool>,
}
impl Host for Tracked {
    fn tool_call(&self, call: ToolLease) {
        let _ = self.leases.lock().unwrap().send(call);
    }
}
impl Drop for Tracked {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[test]
fn dropping_a_runtime_never_blocks_and_teardown_waits_for_leases() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let (send, leases) = channel();
        let host = Arc::new(Tracked {
            leases: Mutex::new(send),
            dropped: dropped.clone(),
        });
        let tools = [lookup(true, false)];
        let runtime = Runtime::create(&config(ENDPOINT, &tools), host).expect("create");
        let mut driver = Driver::new(runtime, tool_model(&[r#"{"q":"held"}"#]), ENDPOINT);
        driver.open();
        driver.runtime.prompt("go", "d-1").expect("prompt");
        let lease = pump_for_lease(&mut driver, &leases, 20);
        let remote = driver.remote();
        // Mid-turn, with a lease out: dropping returns at once.
        let started = Instant::now();
        drop(driver);
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "drop blocked for {:?}",
            started.elapsed()
        );
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !dropped.load(Ordering::SeqCst),
            "teardown must wait for the lease"
        );
        // The lease is still the core's input, readable on this thread.
        assert_eq!(lease.input_json(), r#"{"q":"held"}"#);
        drop(lease);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !dropped.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "teardown did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(remote.cancel(1), Err(Error::Closed));
    });
}
