//! Durability with the store in the loop (Sol's m2 items 2 and 3): the
//! journal writer keeps an ack the core could not take and sends it again; a
//! store failure while the model streams ends the turn as an error even with
//! no barrier waiting; and every head the store acknowledges reopens: a
//! prompt whose record could never reopen is refused, a legal image near the
//! limit reopens, and a long turn the store must stop still reopens.

mod support;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::journal::{JournalWriter, SessionFolder};
use pm_aero_core::{
    Error, Host, Image, Input, JournalAppend, OpenArgs, Runtime, StopReason, Tool, ToolLease, sys,
};
use support::{
    Driver, Model, Reply, Seen, VISION_CATALOG, config, on_worker, sse_pieces, sse_text,
    sse_tool_calls,
};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;
const SAVE: [Tool<'static>; 1] = [Tool {
    name: "save",
    description: "Saves something.",
    input_schema_json: SCHEMA,
    idempotent: false,
    writes: true,
}];
const READ: [Tool<'static>; 1] = [Tool {
    name: "read",
    description: "Reads a large file.",
    input_schema_json: SCHEMA,
    idempotent: true,
    writes: false,
}];

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aero-durable-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// An append copied for the relay: id, turn, first and last seq, barrier,
/// events.
type Relayed = (u64, u64, u64, u64, bool, String);

/// A journaled host whose appends reach the session folder's writer, each
/// after `delay` (so the core's flush is waiting by the time it is acked),
/// and whose tool calls are answered at once with `result`.
struct Stored {
    writer: Arc<JournalWriter>,
    relay: Mutex<Sender<Relayed>>,
    result: String,
}

impl Host for Stored {
    fn journaled(&self) -> bool {
        true
    }
    fn journal_append(&self, append: JournalAppend<'_>) {
        let _ = self.relay.lock().unwrap().send((
            append.append,
            append.turn,
            append.first_seq,
            append.last_seq,
            append.barrier,
            append.events_json.to_string(),
        ));
    }
    fn tool_call(&self, call: ToolLease) {
        let _ = call.answer(&self.result, false);
    }
}

fn stored_host(dir: &Path, delay: Duration, result: String) -> Arc<Stored> {
    let folder = SessionFolder::open(dir).expect("hold the folder");
    let writer = JournalWriter::start(folder).expect("writer");
    let (send, receive) = channel::<Relayed>();
    let relayed = writer.clone();
    std::thread::spawn(move || {
        for (append, turn, first_seq, last_seq, barrier, json) in receive {
            std::thread::sleep(delay);
            relayed.enqueue(&JournalAppend {
                append,
                turn,
                first_seq,
                last_seq,
                barrier,
                events_json: &json,
            });
        }
    });
    Arc::new(Stored {
        writer,
        relay: Mutex::new(send),
        result,
    })
}

fn start(
    dir: &Path,
    tools: &'static [Tool<'static>],
    model: Model,
    delay: Duration,
    result: String,
    max_tool_result_bytes: u32,
) -> (Driver, Arc<Stored>) {
    let host = stored_host(dir, delay, result);
    let mut cfg = config(ENDPOINT, tools);
    cfg.max_tool_result_bytes = max_tool_result_bytes;
    let runtime = Runtime::create(&cfg, host.clone()).expect("create");
    host.writer.bind(runtime.remote());
    let mut driver = Driver::new(runtime, model, ENDPOINT);
    driver.catalog = Some(VISION_CATALOG.to_string());
    (driver, host)
}

fn opened(driver: &mut Driver, request: u64) -> Seen {
    driver.until(20, |e| {
        matches!(e, Seen::Opened { request: r, .. } | Seen::RequestDone { request: r, .. } | Seen::ImagesNeeded { request: r, .. } if *r == request)
            || matches!(e, Seen::Exited { .. })
    })
}

fn turn_end(driver: &mut Driver, turn: u64, secs: u64) -> StopReason {
    match driver.until(secs, |e| {
        matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn) || matches!(e, Seen::Exited { .. })
    }) {
        Seen::TurnEnd { reason, .. } => reason,
        other => panic!("turn ended with {other:?}"),
    }
}

fn settle(driver: &mut Driver) {
    let until = Instant::now() + Duration::from_millis(300);
    while Instant::now() < until {
        driver.pump(10);
    }
}

/// Opens `dir`'s stored session in a fresh runtime; returns what the open
/// ended with.
fn reopen(dir: &Path, args: impl Fn(&pm_aero_core::journal::Stored) -> OpenArgs<'_>) -> Seen {
    let stored = SessionFolder::load(dir).expect("load");
    let (mut driver, host) = start(
        dir,
        &READ,
        Arc::new(|_| Reply::Sse(sse_text("ok"))),
        Duration::ZERO,
        String::new(),
        0,
    );
    let request = driver.runtime.open(&args(&stored)).unwrap_or_else(|error| {
        panic!(
            "the stored head ({} bytes of journal) does not reopen: {error}",
            stored.journal_json.len()
        )
    });
    let event = opened(&mut driver, request);
    driver.runtime.close();
    host.writer.stop();
    event
}

fn plain(stored: &pm_aero_core::journal::Stored) -> OpenArgs<'_> {
    OpenArgs {
        journal_json: &stored.journal_json,
        checkpoint: &stored.checkpoint,
        ..OpenArgs::default()
    }
}

#[test]
fn the_journal_writer_sends_an_ack_the_core_could_not_take_again() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let dir = scratch("retry");
        let model: Model = Arc::new(|call| {
            if call.index == 0 {
                Reply::Sse(sse_tool_calls("save", &[r#"{"q":"x"}"#]))
            } else {
                Reply::Sse(sse_text("saved"))
            }
        });
        let (mut driver, host) = start(
            &dir,
            &SAVE,
            model,
            Duration::from_millis(30),
            "written".into(),
            0,
        );
        driver.open();
        // The first ack that releases a waiting barrier cannot queue its
        // answer (out of memory): it is not consumed, and the writer must
        // send it again or the barrier waits for ever.
        driver
            .remote()
            .arm_fault(sys::FX_FAULT_REPLY_OOM, 0)
            .unwrap();
        let turn = driver.runtime.prompt("save it", "r-1").expect("prompt");
        assert_eq!(turn_end(&mut driver, turn, 20), StopReason::EndTurn);
        assert_eq!(driver.text_of(turn), "saved");
        driver.runtime.close();
        host.writer.stop();
        let _ = std::fs::remove_dir_all(&dir);
    });
}

/// A journaled host that acks barrier appends at once and holds the others
/// for the test to settle.
struct Held(Mutex<Sender<(u64, bool)>>);
impl Host for Held {
    fn journaled(&self) -> bool {
        true
    }
    fn journal_append(&self, append: JournalAppend<'_>) {
        let _ = self.0.lock().unwrap().send((append.append, append.barrier));
    }
}

#[test]
fn a_store_failure_while_the_model_streams_ends_the_turn_as_an_error() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (send, appends): (_, Receiver<(u64, bool)>) = channel();
        let runtime = Runtime::create(&config(ENDPOINT, &[]), Arc::new(Held(Mutex::new(send))))
            .expect("create");
        let pieces = sse_pieces(&["one ", "two ", "three ", "four ", "five ", "six"]);
        let mut driver = Driver::new(
            runtime,
            Arc::new(move |_| Reply::Chunks(pieces.clone(), Duration::from_millis(60))),
            ENDPOINT,
        );
        let remote = driver.remote();
        let request = driver.runtime.open_session("").expect("open");
        // The open's own appends: acked as they come.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let done = driver
                .pump(10)
                .iter()
                .any(|e| matches!(e, Seen::Opened { request: r, .. } if *r == request));
            while let Ok((append, _)) = appends.try_recv() {
                let _ = remote.journal_ack(append);
            }
            if done {
                break;
            }
            assert!(Instant::now() < deadline, "no open: {:#?}", driver.seen);
        }
        let turn = driver.runtime.prompt("count", "s-1").expect("prompt");
        let mut held: Vec<u64> = Vec::new();
        let mut streaming = false;
        let mut failed = false;
        let deadline = Instant::now() + Duration::from_secs(30);
        let reason = 'turn: loop {
            assert!(
                Instant::now() < deadline,
                "timed out; saw {:#?}",
                driver.seen
            );
            for event in driver.pump(10) {
                match event {
                    Seen::Text { turn: t, .. } if t == turn => streaming = true,
                    Seen::TurnEnd { turn: t, reason } if t == turn => break 'turn reason,
                    Seen::Exited { .. } => panic!("the runtime stopped: {:#?}", driver.seen),
                    _ => {}
                }
            }
            while let Ok((append, barrier)) = appends.try_recv() {
                if barrier {
                    let _ = remote.journal_ack(append);
                } else {
                    held.push(append);
                }
            }
            // The store fails an append the model's reply made, with no
            // barrier waiting on it.
            if !failed
                && streaming
                && let Some(&append) = held.first()
            {
                assert_eq!(
                    remote.journal_fail(append, "the disk is full", false),
                    Ok(pm_aero_core::Outcome::Applied)
                );
                failed = true;
            }
        };
        assert!(
            failed,
            "no plain append came while the model streamed: {:#?}",
            driver.seen
        );
        assert_eq!(reason, StopReason::Error, "{:#?}", driver.seen);
        assert!(
            driver.seen.iter().any(|e| matches!(e, Seen::Error { turn: t, message } if *t == turn && message.contains("disk is full"))),
            "{:#?}",
            driver.seen
        );
        assert_eq!(driver.runtime.prompt("more", "s-2"), Err(Error::State));
        driver.runtime.close();
    });
}

/// A PNG of `len` bytes: a real one-pixel PNG, padded after its end.
fn png(len: usize) -> Vec<u8> {
    let mut bytes = include_bytes!("support/pixel.png").to_vec();
    bytes.resize(len, 0);
    bytes
}

#[test]
fn a_legal_near_limit_image_reopens_and_one_that_could_not_is_refused() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        // Without an image store the journal holds the image inline: one
        // whose record could never reopen is refused before anything runs.
        let dir = scratch("image-inline");
        let (mut driver, host) = start(
            &dir,
            &READ,
            Arc::new(|_| Reply::Sse(sse_text("an image"))),
            Duration::ZERO,
            String::new(),
            0,
        );
        driver.open();
        // A 3.5 MiB PNG (under the model's 3.75 MiB per image) and one just
        // past the journal's inline bound.
        let big = png(3_500 << 10);
        for (id, bytes) in [("i-1", &big), ("i-1b", &png(1_450 << 10))] {
            let image = [Image {
                mime: "image/png",
                data: bytes,
            }];
            assert_eq!(
                driver.runtime.prompt_input(
                    &Input {
                        text: "what is this?",
                        images: &image
                    },
                    id
                ),
                Err(Error::TooLarge),
                "a prompt whose turn a reopen cannot load was admitted ({} bytes)",
                bytes.len()
            );
        }
        // One near the limit is legal and reopens.
        let near = png(1_350 << 10);
        let image = [Image {
            mime: "image/png",
            data: &near,
        }];
        let turn = driver
            .runtime
            .prompt_input(
                &Input {
                    text: "what is this?",
                    images: &image,
                },
                "i-2",
            )
            .expect("a legal image");
        assert_eq!(
            turn_end(&mut driver, turn, 30),
            StopReason::EndTurn,
            "{:#?}",
            driver.seen
        );
        settle(&mut driver);
        driver.runtime.close();
        host.writer.stop();
        drop(host);
        let event = reopen(&dir, plain);
        assert!(matches!(event, Seen::Opened { turns: 1, .. }), "{event:?}");
        let _ = std::fs::remove_dir_all(&dir);

        // With an image store the journal names it: the largest legal image
        // reopens once staged.
        let dir = scratch("image-store");
        let (mut driver, host) = start(
            &dir,
            &READ,
            Arc::new(|_| Reply::Sse(sse_text("an image"))),
            Duration::ZERO,
            String::new(),
            0,
        );
        let request = driver
            .runtime
            .open(&OpenArgs {
                image_store: true,
                inputs_durable: true,
                ..OpenArgs::default()
            })
            .expect("open");
        assert!(matches!(opened(&mut driver, request), Seen::Opened { .. }));
        let name = host
            .writer
            .folder()
            .put_image(&big)
            .expect("store the image");
        let image = [Image {
            mime: "image/png",
            data: &big,
        }];
        let turn = driver
            .runtime
            .prompt_input(
                &Input {
                    text: "what is this?",
                    images: &image,
                },
                "i-3",
            )
            .expect("a legal image with a store");
        assert_eq!(
            turn_end(&mut driver, turn, 30),
            StopReason::EndTurn,
            "{:#?}",
            driver.seen
        );
        settle(&mut driver);
        driver.runtime.close();
        host.writer.stop();
        drop(host);
        let stored = SessionFolder::load(&dir).expect("load");
        assert!(
            stored.journal_json.len() < 1 << 20,
            "the journal names the image"
        );
        let (mut driver, host) = start(
            &dir,
            &READ,
            Arc::new(|_| Reply::Sse(sse_text("ok"))),
            Duration::ZERO,
            String::new(),
            0,
        );
        let args = OpenArgs {
            image_store: true,
            inputs_durable: true,
            ..plain(&stored)
        };
        let request = driver.runtime.open(&args).expect("open");
        let Seen::ImagesNeeded { names, .. } = opened(&mut driver, request) else {
            panic!("{:#?}", driver.seen)
        };
        assert_eq!(names, vec![name.clone()]);
        let bytes = SessionFolder::image(&dir, &name).expect("read it");
        let staged = driver
            .runtime
            .stage_images(&[(name.as_str(), bytes.as_slice())])
            .expect("stage");
        driver.until(
            20,
            |e| matches!(e, Seen::RequestDone { request, .. } if *request == staged),
        );
        let request = driver.runtime.open(&args).expect("open again");
        let event = opened(&mut driver, request);
        assert!(matches!(event, Seen::Opened { turns: 1, .. }), "{event:?}");
        driver.runtime.close();
        host.writer.stop();
        let _ = std::fs::remove_dir_all(&dir);
    });
}

#[test]
fn a_long_journaled_turn_stopped_before_a_quiet_checkpoint_still_reopens() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let dir = scratch("long");
        // Twelve reads of 600 KiB each in one turn: far past what a reopen
        // loads, with no quiet point for a checkpoint in between.
        let model: Model = Arc::new(|call| {
            if call.index < 12 {
                Reply::Sse(sse_tool_calls(
                    "read",
                    &[&format!(r#"{{"q":"part {}"}}"#, call.index)],
                ))
            } else {
                Reply::Sse(sse_text("read it all"))
            }
        });
        let (mut driver, host) = start(
            &dir,
            &READ,
            model,
            Duration::ZERO,
            "x".repeat(600 << 10),
            1 << 20,
        );
        driver.open();
        let turn = driver
            .runtime
            .prompt("read the whole file", "l-1")
            .expect("prompt");
        // The store refuses the append that would outgrow a reopen: the turn
        // ends as an error with everything before it acknowledged.
        assert_eq!(
            turn_end(&mut driver, turn, 60),
            StopReason::Error,
            "{:#?}",
            driver.seen
        );
        settle(&mut driver);
        // Killed here, before any quiet checkpoint: what is on disk is all
        // there is.
        driver.runtime.close();
        host.writer.stop();
        drop(host);
        let stored = SessionFolder::load(&dir).expect("load");
        assert!(
            stored.journal_json.len() <= 4 << 20,
            "{} bytes",
            stored.journal_json.len()
        );
        let event = reopen(&dir, plain);
        assert!(matches!(event, Seen::Opened { .. }), "{event:?}");
        let _ = std::fs::remove_dir_all(&dir);
    });
}
