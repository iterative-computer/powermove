//! Typed events (P-EVENTS) and libfx's steering, end to end against the Rust
//! fake model: a steer cuts a streaming reply, waits for a running tool,
//! joins the turn, and can be withdrawn; a retried message id never runs
//! twice.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::{Host, Outcome, Runtime, Steered, StopReason, Tool, ToolLease};
use support::{
    Driver, Model, ModelCall, Reply, Seen, config, on_worker, sse_pieces, sse_reasoning, sse_text,
    sse_tool_calls,
};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;
const LOOKUP: [Tool<'static>; 1] = [Tool {
    name: "lookup",
    description: "Looks something up.",
    input_schema_json: SCHEMA,
    idempotent: true,
    writes: false,
}];

struct Queue(Mutex<Sender<ToolLease>>);

impl Host for Queue {
    fn tool_call(&self, call: ToolLease) {
        let _ = self.0.lock().unwrap().send(call);
    }
}

fn queue() -> (Arc<Queue>, Receiver<ToolLease>) {
    let (send, receive) = channel();
    (Arc::new(Queue(Mutex::new(send))), receive)
}

/// A model that answers each request by its index, and keeps every body.
fn scripted(
    bodies: Arc<Mutex<Vec<String>>>,
    reply: impl Fn(usize, &ModelCall) -> Reply + Send + Sync + 'static,
) -> Model {
    Arc::new(move |call| {
        bodies.lock().unwrap().push(call.body.clone());
        reply(call.index, call)
    })
}

fn lease(driver: &mut Driver, leases: &Receiver<ToolLease>) -> ToolLease {
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

fn turn_end(driver: &mut Driver, turn: u64) -> StopReason {
    match driver.until(30, |e| {
        matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn) || matches!(e, Seen::Exited { .. })
    }) {
        Seen::TurnEnd { reason, .. } => reason,
        other => panic!("turn ended with {other:?}"),
    }
}

#[test]
fn typed_events_carry_reasoning_text_and_named_tool_cards() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, leases) = queue();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let model = scripted(bodies, |index, _| match index {
            0 => Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"a"}"#])),
            _ => Reply::Sse(sse_reasoning("thinking it over", "done")),
        });
        let runtime = Runtime::create(&config(ENDPOINT, &LOOKUP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let turn = driver.runtime.prompt("look it up", "m-1").expect("prompt");
        lease(&mut driver, &leases)
            .answer("found", false)
            .expect("answer");
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        let ours: Vec<&Seen> = driver
            .seen
            .iter()
            .filter(|e| match e {
                Seen::TurnStart { turn: t, .. }
                | Seen::Text { turn: t, .. }
                | Seen::Thought { turn: t, .. }
                | Seen::ToolStart { turn: t, .. }
                | Seen::ToolEnd { turn: t, .. }
                | Seen::TurnEnd { turn: t, .. } => *t == turn,
                _ => false,
            })
            .collect();
        assert!(
            matches!(ours.first(), Some(Seen::TurnStart { .. })),
            "{ours:#?}"
        );
        assert!(
            matches!(ours.last(), Some(Seen::TurnEnd { .. })),
            "{ours:#?}"
        );
        let start = ours
            .iter()
            .position(|e| matches!(e, Seen::ToolStart { name, .. } if name == "lookup"))
            .expect("a tool card");
        // The card's end names its tool, as its start does.
        let end = ours
            .iter()
            .position(
                |e| matches!(e, Seen::ToolEnd { name, failed: false, .. } if name == "lookup"),
            )
            .expect("the card's end");
        assert!(start < end);
        let thought: String = ours
            .iter()
            .filter_map(|e| match e {
                Seen::Thought { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(thought, "thinking it over");
        assert_eq!(driver.text_of(turn), "done");
        driver.runtime.close();
    });
}

#[test]
fn a_steer_cuts_a_streaming_reply_and_joins_the_turn() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let model = scripted(bodies.clone(), |index, _| match index {
            0 => Reply::Chunks(sse_pieces(&["word "; 60]), Duration::from_millis(25)),
            _ => Reply::Sse(sse_text("brief")),
        });
        let runtime = Runtime::create(
            &config(ENDPOINT, &[]),
            Arc::new(Queue(Mutex::new(channel().0))),
        )
        .expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let turn = driver.runtime.prompt("write a lot", "m-1").expect("prompt");
        driver.until(20, |e| matches!(e, Seen::Text { .. }));
        let request = match driver
            .runtime
            .steer(turn, "be brief", "s-1")
            .expect("steer")
        {
            Steered::Accepted(request) => request,
            Steered::Stale => panic!("the turn is running"),
        };
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        assert!(driver.seen.iter().any(|e| matches!(e,
            Seen::SteerJoined { turn: t, request: r, message_id } if *t == turn && *r == request && message_id == "s-1")));
        assert!(driver.seen.iter().any(|e| matches!(e,
            Seen::RequestDone { request: r, outcome: Ok(Outcome::Applied), .. } if *r == request)));
        // The streaming reply was cut short, and the next request read the steer.
        let text = driver.text_of(turn);
        assert!(text.matches("word").count() < 60, "{text}");
        assert!(text.ends_with("brief"), "{text}");
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2);
        assert!(bodies[1].contains("be brief"));
        drop(bodies);
        driver.runtime.close();
    });
}

#[test]
fn a_steer_waits_for_a_running_tool_and_a_withdrawn_one_never_joins() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, leases) = queue();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let model = scripted(bodies.clone(), |index, _| match index {
            0 => Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"a"}"#])),
            _ => Reply::Sse(sse_text("done")),
        });
        let runtime = Runtime::create(&config(ENDPOINT, &LOOKUP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let turn = driver.runtime.prompt("look it up", "m-1").expect("prompt");
        let call = lease(&mut driver, &leases);
        // Both steers wait while the tool runs; one is withdrawn.
        let kept = match driver
            .runtime
            .steer(turn, "also check b", "s-keep")
            .unwrap()
        {
            Steered::Accepted(request) => request,
            Steered::Stale => panic!("stale"),
        };
        let Steered::Accepted(_) = driver.runtime.steer(turn, "forget it", "s-drop").unwrap()
        else {
            panic!("stale")
        };
        let withdraw = driver.runtime.withdraw(turn, "s-drop").expect("withdraw");
        driver.until(
            20,
            |e| matches!(e, Seen::RequestDone { request, .. } if *request == withdraw),
        );
        assert!(driver.seen.iter().any(|e| matches!(e,
            Seen::RequestDone { request, outcome: Ok(Outcome::Applied), .. } if *request == withdraw)));
        assert_eq!(
            bodies.lock().unwrap().len(),
            1,
            "nothing goes to the model while the tool runs"
        );
        call.answer("found", false).expect("answer");
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        assert!(driver.seen.iter().any(|e| matches!(e,
            Seen::SteerJoined { request, message_id, .. } if *request == kept && message_id == "s-keep")));
        assert!(!driver.seen.iter().any(|e| matches!(e,
            Seen::SteerJoined { message_id, .. } if message_id == "s-drop")));
        let bodies = bodies.lock().unwrap();
        assert!(bodies[1].contains("also check b") && bodies[1].contains("found"));
        assert!(!bodies[1].contains("forget it"));
        drop(bodies);
        driver.runtime.close();
    });
}

#[test]
fn a_retried_message_id_never_runs_twice() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let model: Model = Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            Reply::Sse(sse_text("once"))
        });
        let runtime = Runtime::create(
            &config(ENDPOINT, &[]),
            Arc::new(Queue(Mutex::new(channel().0))),
        )
        .expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let (_, reason) = driver.turn("hello", "m-1");
        assert_eq!(reason, StopReason::EndTurn);
        let before = calls.load(Ordering::SeqCst);
        let again = driver.runtime.prompt("hello", "m-1").expect("retry");
        assert_eq!(turn_end(&mut driver, again), StopReason::EndTurn);
        assert!(driver.seen.iter().any(|e| matches!(e,
            Seen::TurnStart { turn, repeated: true, .. } if *turn == again)));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            before,
            "a repeated turn reaches no model"
        );
        // A new id runs.
        let (_, reason) = driver.turn("hello", "m-2");
        assert_eq!(reason, StopReason::EndTurn);
        assert!(calls.load(Ordering::SeqCst) > before);
        // A steer of a turn that ended is stale.
        assert_eq!(
            driver.runtime.steer(again, "late", "s-1"),
            Ok(Steered::Stale)
        );
        driver.runtime.close();
    });
}
