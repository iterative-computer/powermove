//! Hook leases under faults (Sol's m2 item 5): an answer the core could not
//! take (out of memory, busy) leaves the question open and the lease in the
//! host's hands; a terminal error hands no lease back, so nothing can read
//! released strings; a dropped question settles even when its fallback first
//! meets a retryable error; and close never waits on, or races, a lease.
//! The core's FX_E_OOM and FX_BUSY answers are injected at the wrapper's C
//! call (`fail_next_hook_answers`), as the core returns them.

mod support;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pm_aero_core::{
    Answered, Error, HookAnswer, HookAnswerError, HookKind, HookKinds, HookLease, Host, Runtime,
    StopReason, Tool, ToolLease, fail_next_hook_answers, sys,
};
use support::{Driver, Reply, Seen, config, on_worker, sse_text, sse_tool_calls};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;
const LOOKUP: [Tool<'static>; 1] = [Tool {
    name: "lookup",
    description: "Looks something up.",
    input_schema_json: SCHEMA,
    idempotent: false,
    writes: false,
}];

/// The injected faults are process-wide: one test at a time.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

struct Gate {
    kinds: HookKinds,
    hooks: Mutex<Sender<HookLease>>,
    tools: Mutex<Sender<ToolLease>>,
}

impl Host for Gate {
    fn tool_call(&self, call: ToolLease) {
        let _ = self.tools.lock().unwrap().send(call);
    }
    fn hook_kinds(&self) -> HookKinds {
        self.kinds
    }
    fn hook(&self, question: HookLease) {
        let _ = self.hooks.lock().unwrap().send(question);
    }
}

fn runtime(pre_tool: bool, stop: bool) -> (Driver, Receiver<HookLease>, Receiver<ToolLease>) {
    let (hooks, asked) = channel();
    let (tools, called) = channel();
    let host = Arc::new(Gate {
        kinds: HookKinds { pre_tool, stop },
        hooks: Mutex::new(hooks),
        tools: Mutex::new(tools),
    });
    let model = Arc::new(|call: &support::ModelCall| {
        if call.index == 0 && call.body.contains("use the tool") {
            Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"a"}"#]))
        } else {
            Reply::Sse(sse_text("done"))
        }
    });
    let runtime = Runtime::create(&config(ENDPOINT, &LOOKUP), host).expect("create");
    let mut driver = Driver::new(runtime, model, ENDPOINT);
    driver.open();
    (driver, asked, called)
}

fn question(driver: &mut Driver, asked: &Receiver<HookLease>) -> HookLease {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        driver.pump(10);
        if let Ok(question) = asked.try_recv() {
            return question;
        }
        assert!(
            Instant::now() < deadline,
            "no question; saw {:#?}",
            driver.seen
        );
    }
}

fn turn_end(driver: &mut Driver, turn: u64) -> StopReason {
    match driver.until(30, |e| {
        matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn) || matches!(e, Seen::Exited { .. })
    }) {
        Seen::TurnEnd { reason, .. } => reason,
        other => panic!("turn ended with {other:?}"),
    }
}

#[test]
fn an_answer_the_core_could_not_take_keeps_the_lease_answerable() {
    let _serial = serial();
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (mut driver, asked, _called) = runtime(false, true);
        let turn = driver.runtime.prompt("finish", "h-1").expect("prompt");
        let lease = question(&mut driver, &asked);
        assert_eq!(lease.kind(), HookKind::Stop);
        // Out of memory once: the question stays open, the lease comes back
        // and still reads the core's strings.
        fail_next_hook_answers(1, sys::FX_E_OOM);
        let Err(HookAnswerError::Open(lease, error)) = lease.answer(HookAnswer::Allow) else {
            panic!("an answer the core did not take consumed the lease")
        };
        assert_eq!(error, Error::OutOfMemory);
        assert_eq!(lease.final_text(), "done");
        // Busy past every retry: open again.
        fail_next_hook_answers(64, sys::FX_BUSY);
        let Err(HookAnswerError::Open(lease, error)) = lease.answer(HookAnswer::Allow) else {
            panic!("a busy core consumed the lease")
        };
        fail_next_hook_answers(0, 0);
        assert_eq!(error, Error::Busy);
        assert_eq!(
            lease.answer(HookAnswer::Allow).ok(),
            Some(Answered::Applied)
        );
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        driver.runtime.close();
    });
}

#[test]
fn a_dropped_question_settles_when_its_fallback_meets_a_retryable_error() {
    let _serial = serial();
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        // A stop question dropped: its fallback (allow) is out of memory,
        // then busy, before the core takes it; the turn still ends.
        let (mut driver, asked, _called) = runtime(false, true);
        let turn = driver.runtime.prompt("finish", "d-1").expect("prompt");
        let lease = question(&mut driver, &asked);
        fail_next_hook_answers(3, sys::FX_E_OOM);
        drop(lease);
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        driver.runtime.close();

        // A pre_tool question dropped the same way fails closed: the call
        // never runs, and the turn goes on.
        let (mut driver, asked, called) = runtime(true, false);
        let turn = driver
            .runtime
            .prompt("use the tool", "d-2")
            .expect("prompt");
        let lease = question(&mut driver, &asked);
        assert_eq!(lease.kind(), HookKind::PreTool);
        fail_next_hook_answers(5, sys::FX_BUSY);
        // Dropped on a thread of its own, as a host's worker would.
        std::thread::spawn(move || drop(lease)).join().unwrap();
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        assert!(
            called.try_recv().is_err(),
            "a call its failed check blocked ran"
        );
        driver.runtime.close();
    });
}

#[test]
fn a_terminal_answer_error_hands_back_no_lease_and_close_never_waits_on_it() {
    let _serial = serial();
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        // The runtime stops taking answers: the error carries no lease, so
        // nothing can read the strings the destroy releases, and close does
        // not wait for it.
        let (mut driver, asked, _called) = runtime(false, true);
        driver.runtime.prompt("finish", "t-1").expect("prompt");
        let lease = question(&mut driver, &asked);
        fail_next_hook_answers(1, sys::FX_E_FAILED);
        let error = match lease.answer(HookAnswer::Allow) {
            Err(HookAnswerError::Spent(error)) => error,
            Err(HookAnswerError::Open(..)) => panic!("a terminal error handed the lease back"),
            Ok(answered) => panic!("not answered: {answered:?}"),
        };
        assert_eq!(error, Error::Failed);
        let closed = Instant::now();
        driver.runtime.close();
        assert!(
            closed.elapsed() < Duration::from_secs(5),
            "close waited on a spent lease"
        );

        // An open lease held across close: close waits for it, and dropping
        // it (its fallback first refused as busy) lets close finish.
        let (mut driver, asked, _called) = runtime(false, true);
        driver.runtime.prompt("finish", "t-2").expect("prompt");
        let lease = question(&mut driver, &asked);
        fail_next_hook_answers(1, sys::FX_E_OOM);
        let Err(HookAnswerError::Open(lease, _)) = lease.answer(HookAnswer::Allow) else {
            panic!("an out-of-memory answer consumed the lease")
        };
        let holder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(
                lease.final_text(),
                "done",
                "the lease reads its strings until it ends"
            );
            fail_next_hook_answers(2, sys::FX_BUSY);
            drop(lease);
        });
        let closed = Instant::now();
        driver.runtime.close();
        holder.join().unwrap();
        assert!(
            closed.elapsed() < Duration::from_secs(5),
            "close never finished"
        );
    });
}
