//! Every accepted steer settles exactly once (Sol's m2 item 6): it joins
//! (STEER_JOINED naming its request), is withdrawn (its withdraw says
//! FX_OK), or comes back once (STEER_RETURNED before its own turn's
//! TURN_END, or a REQUEST_DONE that is not FX_OK), including a steer
//! admitted while its turn ends and a handoff libfx cannot withdraw.

mod support;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::{
    Error, Host, Image, Input, Outcome, Runtime, Steered, StopReason, Tool, ToolLease,
};
use support::{
    Driver, Reply, Seen, VISION_CATALOG, config, on_worker, sse_pieces, sse_text, sse_tool_calls,
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

fn png() -> &'static [u8] {
    include_bytes!("support/pixel.png")
}

fn turn_end(driver: &mut Driver, turn: u64) -> StopReason {
    // It may have been drained already, by a pump between steers.
    if let Some(reason) = driver.seen.iter().find_map(|e| match e {
        Seen::TurnEnd { turn: t, reason } if *t == turn => Some(*reason),
        _ => None,
    }) {
        return reason;
    }
    match driver.until(30, |e| {
        matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn) || matches!(e, Seen::Exited { .. })
    }) {
        Seen::TurnEnd { reason, .. } => reason,
        other => panic!("turn ended with {other:?}"),
    }
}

fn request_done(driver: &mut Driver, request: u64) -> Result<Outcome, Error> {
    if let Some(outcome) = driver.seen.iter().find_map(|e| match e {
        Seen::RequestDone {
            request: r,
            outcome,
            ..
        } if *r == request => Some(*outcome),
        _ => None,
    }) {
        return outcome;
    }
    match driver.until(
        20,
        |e| matches!(e, Seen::RequestDone { request: r, .. } if *r == request),
    ) {
        Seen::RequestDone { outcome, .. } => outcome,
        _ => unreachable!(),
    }
}

/// Where `seen` has `turn`'s TURN_END.
fn end_at(seen: &[Seen], turn: u64) -> usize {
    seen.iter()
        .position(|e| matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn))
        .expect("the turn ended")
}

#[test]
fn an_image_handoff_libfx_cannot_withdraw_still_comes_back() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, leases) = queue();
        let model = Arc::new(|call: &support::ModelCall| match call.index {
            0 => Reply::Sse(sse_tool_calls("lookup", &[r#"{"q":"a"}"#])),
            _ => Reply::Sse(sse_text("ok")),
        });
        let runtime = Runtime::create(&config(ENDPOINT, &LOOKUP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.catalog = Some(VISION_CATALOG.to_string());
        driver.open();
        let turn = driver.runtime.prompt("look it up", "m-1").expect("prompt");
        let deadline = Instant::now() + Duration::from_secs(20);
        let call = loop {
            driver.pump(10);
            if let Ok(call) = leases.try_recv() {
                break call;
            }
            assert!(Instant::now() < deadline, "no tool call");
        };
        let image = [Image {
            mime: "image/png",
            data: png(),
        }];
        let Steered::Accepted(request) = driver
            .runtime
            .steer_input(
                turn,
                &Input {
                    text: "like this",
                    images: &image,
                },
                "s-img",
            )
            .expect("steer")
        else {
            panic!("the turn is running")
        };
        assert_eq!(request_done(&mut driver, request), Ok(Outcome::Applied));
        // libfx withdraws by input id, and a handoff has none there: the
        // withdraw is stale, so the steer must still come back.
        let withdraw = driver.runtime.withdraw(turn, "s-img").expect("withdraw");
        assert_eq!(request_done(&mut driver, withdraw), Ok(Outcome::Stale));
        call.answer("found", false).expect("answer");
        turn_end(&mut driver, turn);
        let returned = driver.seen.iter().position(|e| matches!(e,
            Seen::SteerReturned { turn: t, request: r, message_id } if *t == turn && *r == request && message_id == "s-img"));
        assert!(
            returned.is_some_and(|at| at < end_at(&driver.seen, turn)),
            "the handoff was lost: {:#?}",
            driver.seen
        );
        driver.runtime.close();
    });
}

#[test]
fn steers_admitted_while_a_turn_ends_come_back_exactly_once() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, _leases) = queue();
        let runtime = Runtime::create(&config(ENDPOINT, &LOOKUP), host).expect("create");
        let mut driver = Driver::new(runtime, Arc::new(|_| Reply::Sse(sse_text("ok"))), ENDPOINT);
        driver.catalog = Some(VISION_CATALOG.to_string());
        driver.open();
        let image = [Image {
            mime: "image/png",
            data: png(),
        }];
        let mut settled = 0usize;
        for round in 0..30 {
            let turn = driver
                .runtime
                .prompt("go", &format!("p-{round}"))
                .expect("prompt");
            // Steers with images until the turn is over: the first hands the
            // turn off, and the rest land before, in and after its end.
            let mut accepted = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(20);
            for index in 0.. {
                assert!(Instant::now() < deadline, "the turn never ended");
                let id = format!("s-{round}-{index}");
                match driver.runtime.steer_input(
                    turn,
                    &Input {
                        text: "this",
                        images: &image,
                    },
                    &id,
                ) {
                    Ok(Steered::Accepted(request)) => accepted.push(request),
                    Ok(Steered::Stale) => break,
                    Err(Error::Backpressure) | Err(Error::Busy) => {}
                    Err(error) => panic!("steer: {error}"),
                }
                if index % 4 == 3 {
                    driver.pump(0);
                }
                if accepted.len() >= 200 {
                    break;
                }
            }
            turn_end(&mut driver, turn);
            // Every accepted steer's REQUEST_DONE comes.
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let missing: Vec<u64> = accepted
                    .iter()
                    .copied()
                    .filter(|request| {
                        !driver.seen.iter().any(
                            |e| matches!(e, Seen::RequestDone { request: r, .. } if r == request),
                        )
                    })
                    .collect();
                if missing.is_empty() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "round {round}: no REQUEST_DONE for {missing:?} of {} accepted",
                    accepted.len()
                );
                driver.pump(10);
            }
            for &request in &accepted {
                let done = request_done(&mut driver, request);
                let returns: Vec<(u64, usize)> = driver
                    .seen
                    .iter()
                    .enumerate()
                    .filter_map(|(at, e)| match e {
                        Seen::SteerReturned {
                            turn: t,
                            request: r,
                            ..
                        } if *r == request => Some((*t, at)),
                        _ => None,
                    })
                    .collect();
                assert!(
                    returns.len() <= 1,
                    "round {round}: request {request} came back twice"
                );
                assert!(
                    returns
                        .iter()
                        .all(|&(t, at)| t == turn && at < end_at(&driver.seen, turn)),
                    "round {round}: request {request} came back outside its turn: {returns:?}"
                );
                let came_back = !returns.is_empty() || done != Ok(Outcome::Applied);
                let twice = !returns.is_empty() && done != Ok(Outcome::Applied);
                assert!(
                    came_back && !twice,
                    "round {round}: request {request} (done {done:?}, returned {returns:?}) did not come back exactly once"
                );
                settled += 1;
            }
        }
        assert!(settled > 30, "too few steers to mean anything");
        driver.runtime.close();
    });
}

#[test]
fn a_withdraw_racing_a_join_reports_the_true_outcome() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, _leases) = queue();
        let pieces = sse_pieces(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let model = Arc::new(move |call: &support::ModelCall| {
            if call.body.contains("join me") {
                Reply::Sse(sse_text("joined"))
            } else {
                Reply::Chunks(pieces.clone(), Duration::from_millis(15))
            }
        });
        let runtime = Runtime::create(&config(ENDPOINT, &LOOKUP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let (mut joined, mut withdrawn) = (0, 0);
        for round in 0..20 {
            let turn = driver
                .runtime
                .prompt("stream", &format!("w-{round}"))
                .expect("prompt");
            driver.until(
                20,
                |e| matches!(e, Seen::Text { turn: t, .. } if *t == turn),
            );
            // Spread the withdraw over the join's window.
            std::thread::sleep(Duration::from_micros(300 * (round % 7)));
            let id = format!("j-{round}");
            let Steered::Accepted(steer) =
                driver.runtime.steer(turn, "join me", &id).expect("steer")
            else {
                continue;
            };
            std::thread::sleep(Duration::from_micros(200 * (round % 5)));
            let withdraw = match driver.runtime.withdraw(turn, &id) {
                Ok(request) => Some(request),
                Err(Error::Stale) => None,
                Err(error) => panic!("withdraw: {error}"),
            };
            turn_end(&mut driver, turn);
            let outcome = withdraw.map(|w| request_done(&mut driver, w));
            let joins: Vec<u64> = driver
                .seen
                .iter()
                .filter_map(|e| match e {
                    Seen::SteerJoined {
                        request,
                        message_id,
                        ..
                    } if *message_id == id => Some(*request),
                    _ => None,
                })
                .collect();
            let returned = driver.seen.iter().any(|e| {
                matches!(e,
                Seen::SteerReturned { message_id, .. } if *message_id == id)
            });
            assert!(joins.len() <= 1);
            if let Some(&request) = joins.first() {
                assert_eq!(
                    request, steer,
                    "round {round}: the join names another request"
                );
                assert_ne!(
                    outcome,
                    Some(Ok(Outcome::Applied)),
                    "round {round}: joined and withdrawn"
                );
                joined += 1;
            } else if outcome == Some(Ok(Outcome::Applied)) {
                assert!(!returned, "round {round}: withdrawn and returned");
                withdrawn += 1;
            } else {
                assert!(
                    returned || request_done(&mut driver, steer) != Ok(Outcome::Applied),
                    "round {round}: neither joined, withdrawn nor back: withdraw {outcome:?}, {:#?}",
                    driver.seen
                );
            }
        }
        eprintln!("joined {joined}, withdrawn {withdrawn}");
        driver.runtime.close();
    });
}
