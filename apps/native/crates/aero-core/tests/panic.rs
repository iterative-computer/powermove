//! Panic containment (P-PANIC, Sol's item 2), by fault injection. In every
//! case the test process keeps running, the failed session ends with a
//! readable error, host calls on it return at once instead of waiting on a
//! lock the parked thread holds, and other runtimes are unaffected.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::{
    Error, Host, JournalAppend, Remote, Runtime, State, StopReason, Tool, ToolLease, sys,
};
use support::{
    Driver, Model, Reply, Seen, config, on_worker, sse_text, sse_tool_calls, text_model,
};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;
const TOOLS: [Tool<'static>; 1] = [Tool {
    name: "lookup",
    description: "Looks something up.",
    input_schema_json: SCHEMA,
    idempotent: true,
    writes: false,
}];

struct NoTools;
impl Host for NoTools {}

fn tool_then_text(inputs: &'static [&'static str]) -> Model {
    Arc::new(move |call| {
        if call.index == 0 {
            Reply::Sse(sse_tool_calls("lookup", inputs))
        } else {
            Reply::Sse(sse_text("done"))
        }
    })
}

/// Pumps until the runtime's EXITED, and checks it reads as this panic.
fn expect_panicked(driver: &mut Driver, point: &str) -> bool {
    let exited = driver.until(20, |e| matches!(e, Seen::Exited { .. }));
    let Seen::Exited {
        state,
        message,
        events_lost,
    } = exited
    else {
        unreachable!()
    };
    assert_eq!(state, State::Panicked);
    assert!(
        message.contains(&format!("fault injected at {point}")),
        "message: {message}"
    );
    let (state, info) = driver.runtime.info().expect("info");
    assert_eq!(state, State::Panicked);
    assert_eq!(info, message);
    // EXITED comes once, and nothing after it.
    for _ in 0..5 {
        assert!(driver.pump(5).is_empty());
    }
    events_lost
}

/// Host calls on the failed runtime answer at once.
fn calls_return_at_once(driver: &mut Driver) {
    let remote = driver.remote();
    let started = Instant::now();
    assert_eq!(driver.runtime.prompt("more", "more-1"), Err(Error::Failed));
    assert_eq!(remote.cancel(1), Err(Error::Failed));
    assert_eq!(remote.fetch_push(1, b"x"), Err(Error::Failed));
    assert_eq!(remote.journal_ack(1), Err(Error::Failed));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "host calls waited {:?}",
        started.elapsed()
    );
}

/// Another runtime runs a whole turn after the failure.
fn bystander_runs() {
    let runtime = Runtime::create(&config(ENDPOINT, &[]), Arc::new(NoTools)).expect("create");
    let mut driver = Driver::new(runtime, text_model("still here"), ENDPOINT);
    driver.open();
    let (turn, reason) = driver.turn("hello", "by-1");
    assert_eq!(reason, StopReason::EndTurn);
    assert_eq!(driver.text_of(turn), "still here");
    driver.runtime.close();
}

/// Destroy of the panicked runtime returns at once and retires the handle.
fn close_quickly(driver: Driver) {
    let remote = driver.remote();
    let started = Instant::now();
    driver.runtime.close();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "destroy took {:?}",
        started.elapsed()
    );
    assert_eq!(remote.cancel(1), Err(Error::Closed));
}

fn victim(model: Model, host: Arc<dyn Host>, tools: &'static [Tool<'static>]) -> Driver {
    let runtime = Runtime::create(&config(ENDPOINT, tools), host).expect("create");
    Driver::new(runtime, model, ENDPOINT)
}

#[test]
fn a_panic_holding_the_fetch_lock() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let mut driver = victim(text_model("never"), Arc::new(NoTools), &[]);
        driver.open();
        driver
            .runtime
            .arm_fault(sys::FX_FAULT_FETCH_LOCKED, 0)
            .unwrap();
        driver.runtime.prompt("go", "f-1").expect("prompt");
        expect_panicked(&mut driver, "fetch_locked");
        // The parked thread holds the bridge lock: the fetch calls return.
        let remote = driver.remote();
        let started = Instant::now();
        assert_eq!(remote.fetch_status(1, 200, None), Err(Error::Failed));
        assert_eq!(remote.fetch_finish(1, true), Err(Error::Failed));
        assert_eq!(
            remote.fetch_disposition(1),
            pm_aero_core::Disposition::Retired
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        calls_return_at_once(&mut driver);
        bystander_runs();
        close_quickly(driver);
    });
}

#[test]
fn a_panic_holding_the_event_lock() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let mut driver = victim(text_model("never"), Arc::new(NoTools), &[]);
        // The first event the core queues is SESSION_OPENED.
        driver
            .runtime
            .arm_fault(sys::FX_FAULT_EVENT_LOCKED, 0)
            .unwrap();
        driver.runtime.open_session("").expect("open");
        let lost = expect_panicked(&mut driver, "event_locked");
        assert!(
            lost,
            "the queue was held by the parked thread: EXITED says events were lost"
        );
        calls_return_at_once(&mut driver);
        bystander_runs();
        close_quickly(driver);
    });
}

#[test]
fn a_panic_while_streaming() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let mut driver = victim(text_model("streamed"), Arc::new(NoTools), &[]);
        driver.open();
        driver
            .runtime
            .arm_fault(sys::FX_FAULT_STREAMING, 0)
            .unwrap();
        let turn = driver.runtime.prompt("go", "s-1").expect("prompt");
        let lost = expect_panicked(&mut driver, "streaming");
        assert!(!lost);
        // What was queued before the panic was delivered first.
        assert!(
            driver
                .seen
                .iter()
                .any(|e| matches!(e, Seen::TurnStart { turn: t, .. } if *t == turn))
        );
        calls_return_at_once(&mut driver);
        bystander_runs();
        close_quickly(driver);
    });
}

struct Queue(Mutex<Sender<ToolLease>>);
impl Host for Queue {
    fn tool_call(&self, call: ToolLease) {
        let _ = self.0.lock().unwrap().send(call);
    }
}

fn lease_from(driver: &mut Driver, leases: &Receiver<ToolLease>) -> ToolLease {
    let deadline = Instant::now() + Duration::from_secs(20);
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

#[test]
fn a_panic_inside_a_tool_result_submission_never_parks_the_host() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (send, leases) = channel();
        let mut driver = victim(
            tool_then_text(&[r#"{"q":"a"}"#]),
            Arc::new(Queue(Mutex::new(send))),
            &TOOLS,
        );
        driver.open();
        driver.runtime.prompt("go", "t-1").expect("prompt");
        let lease = lease_from(&mut driver, &leases);
        driver
            .runtime
            .arm_fault(sys::FX_FAULT_TOOL_RESULT, 0)
            .unwrap();
        // The panic happens on this thread, inside fx_tool_result, holding
        // the runtime lock: the call returns FX_E_FAILED instead of parking.
        let started = Instant::now();
        assert_eq!(lease.answer("result", false), Err(Error::Failed));
        assert!(started.elapsed() < Duration::from_secs(1));
        expect_panicked(&mut driver, "tool_result");
        // The runtime lock stays held by the unwound call; nothing waits on it.
        calls_return_at_once(&mut driver);
        bystander_runs();
        close_quickly(driver);
    });
}

/// Journals in memory; the test acks from its own thread (callbacks never
/// call the runtime).
struct Journal(Mutex<Sender<u64>>);
impl Host for Journal {
    fn journaled(&self) -> bool {
        true
    }
    fn journal_append(&self, append: JournalAppend<'_>) {
        assert!(append.events_json.starts_with('['));
        let _ = self.0.lock().unwrap().send(append.append);
    }
}

#[test]
fn a_panic_inside_a_journal_append() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (send, appends) = channel();
        let mut driver = victim(
            text_model("journaled"),
            Arc::new(Journal(Mutex::new(send))),
            &[],
        );
        let remote = driver.remote();
        // A healthy journaled turn first: appends arrive and are acked.
        let acker = std::thread::spawn(move || {
            let mut acked = 0;
            while let Ok(append) = appends.recv() {
                let _ = remote.journal_ack(append);
                acked += 1;
            }
            acked
        });
        driver.open();
        let (_, reason) = driver.turn("first", "j-1");
        assert_eq!(reason, StopReason::EndTurn);
        driver
            .runtime
            .arm_fault(sys::FX_FAULT_JOURNAL_APPEND, 0)
            .unwrap();
        driver.runtime.prompt("second", "j-2").expect("prompt");
        expect_panicked(&mut driver, "journal_append");
        calls_return_at_once(&mut driver);
        bystander_runs();
        close_quickly(driver);
        let acked = acker.join().unwrap();
        assert!(acked > 0, "the healthy turn was journaled");
    });
}

/// A tool callback that blocks (only in this test) until released, so a
/// core thread is mid-callback while another panics.
struct Blocking {
    entered: Mutex<Sender<u64>>,
    release: Mutex<Receiver<()>>,
    calls: AtomicUsize,
}
impl Host for Blocking {
    fn tool_call(&self, call: ToolLease) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.entered.lock().unwrap().send(call.call());
        let _ = self
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(30));
        let _ = call.answer("released", false);
    }
}

fn blocking() -> (Arc<Blocking>, Receiver<u64>, Sender<()>) {
    let (entered, entered_rx) = channel();
    let (release_tx, release) = channel();
    (
        Arc::new(Blocking {
            entered: Mutex::new(entered),
            release: Mutex::new(release),
            calls: AtomicUsize::new(0),
        }),
        entered_rx,
        release_tx,
    )
}

fn wait_entered(driver: &mut Driver, entered: &Receiver<u64>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while entered.try_recv().is_err() {
        driver.pump(10);
        assert!(
            Instant::now() < deadline,
            "no callback; saw {:#?}",
            driver.seen
        );
    }
}

#[test]
fn a_panic_while_another_runtimes_core_thread_is_mid_callback() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, entered, release) = blocking();
        let mut busy = victim(tool_then_text(&[r#"{"q":"a"}"#]), host, &TOOLS);
        busy.open();
        let busy_turn = busy.runtime.prompt("go", "b-1").expect("prompt");
        wait_entered(&mut busy, &entered);
        // A's core thread is inside its callback; B panics now.
        let mut failing = victim(text_model("x"), Arc::new(NoTools), &[]);
        failing.open();
        failing
            .runtime
            .arm_fault(sys::FX_FAULT_STREAMING, 0)
            .unwrap();
        failing.runtime.prompt("go", "f-1").expect("prompt");
        expect_panicked(&mut failing, "streaming");
        // A finishes untouched.
        release.send(()).unwrap();
        match busy.until(
            20,
            |e| matches!(e, Seen::TurnEnd { turn, .. } if *turn == busy_turn),
        ) {
            Seen::TurnEnd { reason, .. } => assert_eq!(reason, StopReason::EndTurn),
            _ => unreachable!(),
        }
        assert_eq!(busy.text_of(busy_turn), "done");
        close_quickly(failing);
        busy.runtime.close();
    });
}

#[test]
fn a_sibling_panic_mid_callback_waits_for_the_callback_then_calls_nothing() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, entered, release) = blocking();
        let mut driver = victim(tool_then_text(&[r#"{"q":"a"}"#]), host.clone(), &TOOLS);
        driver.open();
        let turn = driver.runtime.prompt("go", "m-1").expect("prompt");
        wait_entered(&mut driver, &entered);
        // A prompt worker is inside the tool callback. The core's reader
        // thread now panics holding the event queue, as it queues the answer
        // to a steer (a steer waits for a running tool).
        driver
            .runtime
            .arm_fault(sys::FX_FAULT_EVENT_LOCKED, 0)
            .unwrap();
        assert!(matches!(
            driver.runtime.steer(turn, "hurry", "st-1"),
            Ok(pm_aero_core::Steered::Accepted(_))
        ));
        let lost = expect_panicked(&mut driver, "event_locked");
        assert!(lost);
        // Destroy waits for the running callback, then returns; none after.
        let handle = driver.remote().handle();
        let (done_tx, done) = channel();
        std::thread::spawn(move || {
            // SAFETY: plain value call (the C destroy, without the wrapper's
            // lease wait, to watch the header's rule directly).
            let status = unsafe { sys::fx_runtime_destroy(handle) };
            let _ = done_tx.send(status);
        });
        assert!(
            done.recv_timeout(Duration::from_millis(300)).is_err(),
            "destroy returned with a callback running"
        );
        release.send(()).unwrap();
        assert_eq!(done.recv_timeout(Duration::from_secs(5)), Ok(sys::FX_OK));
        let calls = host.calls.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            host.calls.load(Ordering::SeqCst),
            calls,
            "a callback ran after destroy"
        );
        assert_eq!(calls, 1);
        let remote: Remote = driver.remote();
        assert_eq!(remote.cancel(1), Err(Error::Closed));
        bystander_runs();
        // The wrapper's teardown finds the handle retired and frees its box.
        driver.runtime.close();
    });
}

/// Journals in memory and acks nothing itself.
struct Collect(Mutex<Sender<(u64, bool)>>);
impl Host for Collect {
    fn journaled(&self) -> bool {
        true
    }
    fn journal_append(&self, append: JournalAppend<'_>) {
        let _ = self.0.lock().unwrap().send((append.append, append.barrier));
    }
}

#[test]
fn a_host_call_failing_the_transport_never_waits_on_the_input_lock() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (send, _appends) = channel();
        let mut driver = victim(
            text_model("never"),
            Arc::new(Collect(Mutex::new(send))),
            &[],
        );
        // Opening a journaled session makes the core write its journal_open
        // request: that core thread then holds the input lock and, armed,
        // waits for the runtime to stop before it panics there.
        driver
            .runtime
            .arm_fault(sys::FX_FAULT_INPUT_LOCKED, 0)
            .unwrap();
        driver
            .runtime
            .arm_fault(sys::FX_FAULT_HOST_TRANSPORT, 0)
            .unwrap();
        driver.runtime.open_session("").expect("open");
        std::thread::sleep(Duration::from_millis(500));
        // This host call fails the transport while the lock is held: the
        // failure is published first, the core thread panics holding the
        // lock, and the call gives up on it instead of waiting for ever.
        let remote = driver.remote();
        let (done_tx, done) = channel();
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = remote.journal_ack(1);
            let _ = done_tx.send((result, started.elapsed()));
        });
        let (result, took) = done
            .recv_timeout(Duration::from_secs(5))
            .expect("the host call is stranded on the input lock");
        assert_eq!(result, Err(Error::Failed));
        assert!(took < Duration::from_secs(1), "the host call took {took:?}");
        expect_panicked(&mut driver, "input_locked");
        calls_return_at_once(&mut driver);
        bystander_runs();
        close_quickly(driver);
    });
}

#[test]
fn wrapper_close_with_an_outstanding_lease_on_a_panicked_runtime() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (send, leases) = channel();
        let mut driver = victim(
            tool_then_text(&[r#"{"q":"a"}"#, r#"{"q":"b"}"#]),
            Arc::new(Queue(Mutex::new(send))),
            &TOOLS,
        );
        driver.open();
        driver.runtime.prompt("go", "w-1").expect("prompt");
        let first = lease_from(&mut driver, &leases);
        let second = lease_from(&mut driver, &leases);
        let (held, answered) = if first.input_json().contains('a') {
            (first, second)
        } else {
            (second, first)
        };
        // The runtime panics inside the other lease's answer.
        driver
            .runtime
            .arm_fault(sys::FX_FAULT_TOOL_RESULT, 0)
            .unwrap();
        assert_eq!(answered.answer("b", false), Err(Error::Failed));
        expect_panicked(&mut driver, "tool_result");
        // Close on this thread, which still holds a lease: it does not wait
        // for it, since a panicked runtime's memory is never freed.
        let remote = driver.remote();
        let started = Instant::now();
        driver.runtime.close();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "close waited {:?} for a lease",
            started.elapsed()
        );
        // The lease's input is still readable after destroy (header rule 8),
        // and its late answer finds the handle closed.
        assert_eq!(held.input_json(), r#"{"q":"a"}"#);
        assert_eq!(held.name(), "lookup");
        assert_eq!(held.answer("late", false), Err(Error::Closed));
        assert_eq!(remote.cancel(1), Err(Error::Closed));
        bystander_runs();
    });
}
