//! Hook leases (P-STOP, P-PRETOOL): pre_tool blocks, rewrites or allows a
//! host tool call before it is made; the stop gate continues a turn up to its
//! limit; a dropped question fails closed for a tool and lets a turn end; a
//! cancelled turn's question still waits for its answer.

mod support;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::{
    Answered, Error, HookAnswer, HookKind, HookKinds, HookLease, Host, Outcome, Runtime,
    StopReason, Tool, ToolLease,
};
use support::{Driver, Model, Reply, Seen, config, on_worker, sse_text, sse_tool_calls};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;
const LOOKUP: [Tool<'static>; 1] = [Tool {
    name: "lookup",
    description: "Looks something up.",
    input_schema_json: SCHEMA,
    idempotent: false,
    writes: false,
}];

enum Asked {
    Tool(ToolLease),
    Hook(HookLease),
}

struct Gate {
    kinds: HookKinds,
    out: Mutex<Sender<Asked>>,
    /// Answers every stop question inline, from the callback.
    stop_inline: bool,
}

impl Host for Gate {
    fn tool_call(&self, call: ToolLease) {
        let _ = self.out.lock().unwrap().send(Asked::Tool(call));
    }
    fn hook_kinds(&self) -> HookKinds {
        self.kinds
    }
    fn hook(&self, question: HookLease) {
        if self.stop_inline && question.kind() == HookKind::Stop {
            question.answer(HookAnswer::Allow).expect("inline answer");
            return;
        }
        let _ = self.out.lock().unwrap().send(Asked::Hook(question));
    }
}

fn gate(pre_tool: bool, stop: bool) -> (Arc<Gate>, Receiver<Asked>) {
    let (send, receive) = channel();
    (
        Arc::new(Gate {
            kinds: HookKinds { pre_tool, stop },
            out: Mutex::new(send),
            stop_inline: false,
        }),
        receive,
    )
}

fn recording(
    bodies: Arc<Mutex<Vec<String>>>,
    reply: impl Fn(usize) -> Reply + Send + Sync + 'static,
) -> Model {
    Arc::new(move |call| {
        bodies.lock().unwrap().push(call.body.clone());
        reply(call.index)
    })
}

fn next(driver: &mut Driver, asked: &Receiver<Asked>) -> Asked {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        driver.pump(10);
        if let Ok(item) = asked.try_recv() {
            return item;
        }
        assert!(
            Instant::now() < deadline,
            "nothing asked; saw {:#?}",
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
fn pre_tool_blocks_rewrites_and_allows_before_the_call_is_made() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, asked) = gate(true, false);
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let model = recording(bodies.clone(), |index| match index {
            0 => Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"a"}"#])),
            1 => Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"b"}"#])),
            2 => Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"c"}"#])),
            _ => Reply::Sse(sse_text("done")),
        });
        let runtime = Runtime::create(&config(ENDPOINT, &LOOKUP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let turn = driver
            .runtime
            .prompt("look three things up", "m-1")
            .expect("prompt");

        // a: blocked. The question names the call and its input.
        let Asked::Hook(first) = next(&mut driver, &asked) else {
            panic!("a tool ran before pre_tool")
        };
        assert_eq!(
            (first.kind(), first.name(), first.input_json()),
            (HookKind::PreTool, "lookup", r#"{"q":"a"}"#)
        );
        assert_eq!(first.turn(), turn);
        // A stop answer is not one pre_tool takes: the lease stays open.
        let (first, error) = first
            .answer(HookAnswer::Continue("go on"))
            .expect_err("wrong kind");
        assert_eq!(error, Error::Invalid);
        let (first, error) = first
            .answer(HookAnswer::Rewrite("[1]"))
            .expect_err("not an object");
        assert_eq!(error, Error::Invalid);
        assert!(matches!(
            first.answer(HookAnswer::Block("lookups of a are not allowed")),
            Ok(Answered::Applied)
        ));

        // b: rewritten; the call it allows carries the question's call id.
        let Asked::Hook(second) = next(&mut driver, &asked) else {
            panic!("blocked call ran")
        };
        assert_eq!(second.input_json(), r#"{"q":"b"}"#);
        let promised = second.call();
        second
            .answer(HookAnswer::Rewrite(r#"{"q":"rewritten"}"#))
            .expect("rewrite");
        let Asked::Tool(call) = next(&mut driver, &asked) else {
            panic!("no call")
        };
        assert_eq!(call.call(), promised);
        assert_eq!(call.input_json(), r#"{"q":"rewritten"}"#);
        call.answer("b found", false).expect("answer");

        // c: allowed unchanged.
        let Asked::Hook(third) = next(&mut driver, &asked) else {
            panic!("no question")
        };
        third.answer(HookAnswer::Allow).expect("allow");
        let Asked::Tool(call) = next(&mut driver, &asked) else {
            panic!("no call")
        };
        assert_eq!(call.input_json(), r#"{"q":"c"}"#);
        call.answer("c found", false).expect("answer");

        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        let bodies = bodies.lock().unwrap();
        assert!(
            bodies[1].contains("lookups of a are not allowed"),
            "{}",
            bodies[1]
        );
        assert!(bodies[2].contains("b found"));
        drop(bodies);
        assert!(asked.try_recv().is_err(), "nothing else was asked");
        driver.runtime.close();
    });
}

#[test]
fn the_stop_gate_continues_a_turn_up_to_its_limit() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, asked) = gate(false, true);
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let model = recording(bodies.clone(), |index| {
            Reply::Sse(sse_text(&format!("reply {index}")))
        });
        let cfg = pm_aero_core::Config {
            max_stop_continuations: 2,
            ..config(ENDPOINT, &[])
        };
        let runtime = Runtime::create(&cfg, host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let turn = driver
            .runtime
            .prompt("finish the task", "m-1")
            .expect("prompt");
        // Asked max_stop_continuations + 1 times; it always asks to continue.
        for asks in 0..3u32 {
            let Asked::Hook(question) = next(&mut driver, &asked) else {
                panic!("a tool?")
            };
            assert_eq!(question.kind(), HookKind::Stop);
            assert_eq!(question.turn(), turn);
            assert_eq!(question.continuations(), asks);
            assert_eq!(question.final_text(), format!("reply {asks}"));
            question
                .answer(HookAnswer::Continue(&format!("not done yet, step {asks}")))
                .expect("continue");
        }
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        assert!(asked.try_recv().is_err(), "a fourth ask");
        let bodies = bodies.lock().unwrap();
        // The continuations reached the model; the last one did not run.
        assert_eq!(bodies.len(), 3, "{bodies:#?}");
        assert!(bodies[1].contains("not done yet, step 0"));
        assert!(bodies[2].contains("not done yet, step 1"));
        drop(bodies);
        // The gate counts per turn: the next turn is asked afresh.
        let next_turn = driver.runtime.prompt("again", "m-2").expect("prompt");
        let Asked::Hook(question) = next(&mut driver, &asked) else {
            panic!()
        };
        assert_eq!(question.continuations(), 0);
        question.answer(HookAnswer::Allow).expect("allow");
        assert_eq!(turn_end(&mut driver, next_turn), StopReason::EndTurn);
        driver.runtime.close();
    });
}

#[test]
fn a_dropped_question_fails_closed_for_a_tool_and_lets_a_turn_end() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, asked) = gate(true, true);
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let model = recording(bodies.clone(), |index| match index {
            0 => Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"a"}"#])),
            _ => Reply::Sse(sse_text("done")),
        });
        let runtime = Runtime::create(&config(ENDPOINT, &LOOKUP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let turn = driver.runtime.prompt("look it up", "m-1").expect("prompt");
        let Asked::Hook(question) = next(&mut driver, &asked) else {
            panic!("ran before pre_tool")
        };
        drop(question);
        let Asked::Hook(stop) = next(&mut driver, &asked) else {
            panic!("the dropped call ran")
        };
        assert_eq!(stop.kind(), HookKind::Stop);
        drop(stop);
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        assert!(bodies.lock().unwrap()[1].contains("pre_tool check failed"));
        driver.runtime.close();
    });
}

#[test]
fn a_cancelled_turns_question_still_waits_for_its_answer() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, asked) = gate(true, false);
        let model: Model = Arc::new(|call| match call.index {
            0 => Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"a"}"#])),
            _ => Reply::Sse(sse_text("done")),
        });
        let runtime = Runtime::create(&config(ENDPOINT, &LOOKUP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let turn = driver.runtime.prompt("look it up", "m-1").expect("prompt");
        let Asked::Hook(question) = next(&mut driver, &asked) else {
            panic!()
        };
        assert_eq!(driver.remote().cancel(turn), Ok(Outcome::Applied));
        // The question's lease holds the turn open until it is answered.
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            for event in driver.pump(10) {
                assert!(
                    !matches!(event, Seen::TurnEnd { .. }),
                    "the turn ended under an open question"
                );
            }
        }
        assert_eq!(
            question.input_json(),
            r#"{"q":"a"}"#,
            "the lease's input is still valid"
        );
        assert!(matches!(
            question.answer(HookAnswer::Allow),
            Ok(Answered::Applied)
        ));
        assert_eq!(turn_end(&mut driver, turn), StopReason::Cancelled);
        assert!(
            asked.try_recv().is_err(),
            "a cancelled turn's allowed call never runs"
        );
        driver.runtime.close();
    });
}

#[test]
fn a_question_answered_inside_its_callback_and_one_of_a_kind_not_taken() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        // Stop answered from the callback; pre_tool not taken (allowed at once).
        let (send, asked) = channel();
        let host = Arc::new(Gate {
            kinds: HookKinds {
                pre_tool: false,
                stop: true,
            },
            out: Mutex::new(send),
            stop_inline: true,
        });
        let model: Model = Arc::new(|call| match call.index {
            0 => Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"a"}"#])),
            _ => Reply::Sse(sse_text("done")),
        });
        let runtime = Runtime::create(&config(ENDPOINT, &LOOKUP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let turn = driver.runtime.prompt("look it up", "m-1").expect("prompt");
        let Asked::Tool(call) = next(&mut driver, &asked) else {
            panic!("pre_tool was asked")
        };
        call.answer("found", false).expect("answer");
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        assert!(asked.try_recv().is_err());
        driver.runtime.close();
    });
}
