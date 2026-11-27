//! Sol's 2a item 5: a call that fails changes nothing. A failed drain or
//! take keeps the previous one's borrows and loses nothing; a journal ack or
//! failure whose answers cannot be queued is not consumed and does not
//! report success.

mod support;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::{
    Error, Host, JournalAppend, Outcome, Runtime, StopReason, Tool, ToolLease, sys,
};
use support::{Driver, Model, Reply, Seen, config, on_worker, sse_text, sse_tool_calls};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;

struct NoTools;
impl Host for NoTools {}

fn zeroed_events(cap: usize) -> Vec<sys::fx_event> {
    // SAFETY: an all-zero fx_event is a valid value.
    (0..cap).map(|_| unsafe { std::mem::zeroed() }).collect()
}

fn raw_drain(handle: u64, buf: &mut [sys::fx_event]) -> (sys::fx_status, usize) {
    let mut count = 0usize;
    // SAFETY: `buf` holds buf.len() events of the size passed.
    let status = unsafe {
        sys::fx_events_drain(
            handle,
            buf.as_mut_ptr(),
            buf.len(),
            std::mem::size_of::<sys::fx_event>(),
            &mut count,
        )
    };
    (status, count)
}

/// The bytes a borrowed fx_str points at, read now.
fn read(value: sys::fx_str) -> Vec<u8> {
    if value.len == 0 {
        return Vec::new();
    }
    // SAFETY: the caller holds the borrow the header promises for `value`.
    unsafe { std::slice::from_raw_parts(value.ptr.cast::<u8>(), value.len) }.to_vec()
}

fn read_bytes(value: sys::fx_bytes) -> Vec<u8> {
    read(sys::fx_str {
        ptr: value.ptr.cast(),
        len: value.len,
    })
}

#[test]
fn a_failed_drain_keeps_the_previous_borrows_and_loses_nothing() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let runtime = Runtime::create(&config(ENDPOINT, &[]), Arc::new(NoTools)).expect("create");
        let mut driver = Driver::new(runtime, support::text_model("second reply"), ENDPOINT);
        driver.open();
        driver.turn("first", "one-1");
        let handle = driver.remote().handle();
        let turn = driver.runtime.prompt("again", "two-1").expect("prompt");
        // Serve the turn without draining, until it has surely ended.
        let deadline = Instant::now() + Duration::from_secs(20);
        while !driver
            .fetches
            .lock()
            .unwrap()
            .iter()
            .any(|f| f.turn == turn && f.url == ENDPOINT)
        {
            driver.serve_only();
            std::thread::sleep(Duration::from_millis(5));
            assert!(Instant::now() < deadline, "no model fetch");
        }
        std::thread::sleep(Duration::from_millis(500));

        // A partial drain hands out TURN_START; its text is borrowed now.
        let mut first = zeroed_events(1);
        assert_eq!(raw_drain(handle, &mut first), (sys::FX_OK, 1));
        assert_eq!(first[0].kind, sys::FX_EVENT_TURN_START);
        let borrowed = first[0].text;
        assert_eq!(read(borrowed), b"two-1");

        // A drain that fails changes nothing.
        driver
            .remote()
            .arm_fault(sys::FX_FAULT_DRAIN_OOM, 0)
            .unwrap();
        let mut rest = zeroed_events(64);
        assert_eq!(raw_drain(handle, &mut rest), (sys::FX_E_OOM, 0));
        assert_eq!(read(borrowed), b"two-1", "the failed drain ended a borrow");

        // The next drain delivers everything the failed one did not take.
        let (status, count) = raw_drain(handle, &mut rest);
        assert_eq!(status, sys::FX_OK);
        let mut text = Vec::new();
        let mut ended = false;
        for event in &rest[..count] {
            match event.kind {
                sys::FX_EVENT_TEXT_DELTA => text.extend(read(event.text)),
                sys::FX_EVENT_TURN_END => {
                    assert_eq!(event.turn, turn);
                    ended = true;
                }
                _ => {}
            }
        }
        assert_eq!(String::from_utf8(text).unwrap(), "second reply");
        assert!(ended, "TURN_END was lost");
        driver.runtime.close();
    });
}

/// Answers every tool call inside the callback.
struct Answers;
impl Host for Answers {
    fn tool_call(&self, call: ToolLease) {
        let _ = call.answer("looked up", false);
    }
}

fn take(handle: u64, out: &mut sys::fx_fetch_request) -> sys::fx_status {
    out.struct_size = std::mem::size_of::<sys::fx_fetch_request>() as u32;
    // SAFETY: `out` is valid for the call.
    unsafe { sys::fx_fetch_take(handle, out) }
}

fn not_found(remote: pm_aero_core::Remote, fetch: u64) {
    let _ = remote.fetch_status(fetch, 404, None);
    let _ = remote.fetch_finish(fetch, true);
}

fn reply(remote: pm_aero_core::Remote, fetch: u64, sse: &str) {
    assert_eq!(remote.fetch_status(fetch, 200, None), Ok(Outcome::Applied));
    support::push_all(remote, fetch, sse.as_bytes());
    let _ = remote.fetch_finish(fetch, true);
}

#[test]
fn a_failed_take_keeps_the_previous_borrows() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let tools = [Tool {
            name: "lookup",
            description: "Looks something up.",
            input_schema_json: SCHEMA,
            idempotent: true,
            writes: false,
        }];
        let runtime =
            Runtime::create(&config(ENDPOINT, &tools), Arc::new(Answers)).expect("create");
        let model: Model = Arc::new(|_| Reply::Sse(sse_text("done")));
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.open();
        let remote = driver.remote();
        let handle = remote.handle();
        let turn = driver.runtime.prompt("go", "t-1").expect("prompt");

        // The turn's first model request, taken by hand (catalog GETs get 404).
        // SAFETY: an all-zero fx_fetch_request is a valid value.
        let mut a: sys::fx_fetch_request = unsafe { std::mem::zeroed() };
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(Instant::now() < deadline, "no model request");
            let _ = driver.runtime.events.drain().map(|events| events.count());
            if take(handle, &mut a) != sys::FX_OK {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            if a.purpose == sys::FX_FETCH_MODEL && read(a.url) == ENDPOINT.as_bytes() {
                break;
            }
            not_found(remote, a.fetch);
        }
        let a_url = read(a.url);
        let a_body = read_bytes(a.body);
        let a_method = read(a.method);
        assert_eq!(a.turn, turn);
        // It asks for a tool; the host answers inside the callback, and the
        // core publishes its next request. It is not taken yet.
        reply(
            remote,
            a.fetch,
            &sse_tool_calls("lookup", &[r#"{"q":"x"}"#]),
        );
        std::thread::sleep(Duration::from_millis(1000));

        // A take that fails changes nothing: A's borrows stay valid.
        remote.arm_fault(sys::FX_FAULT_TAKE_OOM, 0).unwrap();
        // SAFETY: as above.
        let mut b: sys::fx_fetch_request = unsafe { std::mem::zeroed() };
        assert_eq!(take(handle, &mut b), sys::FX_E_OOM);
        assert_eq!(b.fetch, 0, "a failed take zeroes its out struct");
        assert_eq!(read(a.url), a_url);
        assert_eq!(read(a.method), a_method);
        assert_eq!(
            read_bytes(a.body),
            a_body,
            "the failed take ended A's borrows"
        );

        // The request stayed published: the next take hands it out.
        assert_eq!(take(handle, &mut b), sys::FX_OK);
        assert!(b.fetch > a.fetch);
        if b.purpose == sys::FX_FETCH_MODEL && read(b.url) == ENDPOINT.as_bytes() {
            assert!(String::from_utf8_lossy(&read_bytes(b.body)).contains("looked up"));
            reply(remote, b.fetch, &sse_text("done"));
        } else {
            not_found(remote, b.fetch);
        }
        match driver.until(
            30,
            |e| matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn),
        ) {
            Seen::TurnEnd { reason, .. } => assert_eq!(reason, StopReason::EndTurn),
            _ => unreachable!(),
        }
        assert_eq!(driver.text_of(turn), "done");
        driver.runtime.close();
    });
}

/// A journaled host with one writes tool (its intent is a barrier the core
/// flushes before the tool runs), answered inside the callback.
struct JournalTool(Mutex<Sender<(u64, bool)>>);
impl Host for JournalTool {
    fn journaled(&self) -> bool {
        true
    }
    fn journal_append(&self, append: JournalAppend<'_>) {
        let _ = self.0.lock().unwrap().send((append.append, append.barrier));
    }
    fn tool_call(&self, call: ToolLease) {
        let _ = call.answer("written", false);
    }
}

fn tool_once() -> Model {
    Arc::new(|call| {
        if call.index == 0 {
            Reply::Sse(sse_tool_calls("save", &[r#"{"q":"x"}"#]))
        } else {
            Reply::Sse(sse_text("saved"))
        }
    })
}

const SAVE: [Tool<'static>; 1] = [Tool {
    name: "save",
    description: "Saves something.",
    input_schema_json: SCHEMA,
    idempotent: false,
    writes: true,
}];

/// Pumps the turn; acks plain appends at once and hands each barrier append
/// to `on_barrier`. Returns how the turn ended and how many barriers came.
fn run_journaled(
    driver: &mut Driver,
    appends: &Receiver<(u64, bool)>,
    turn: u64,
    mut on_barrier: impl FnMut(u64),
) -> (StopReason, usize) {
    let remote = driver.remote();
    let mut barriers = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(
            Instant::now() < deadline,
            "timed out; saw {:#?}",
            driver.seen
        );
        for event in driver.pump(10) {
            match event {
                Seen::TurnEnd { turn: t, reason } if t == turn => return (reason, barriers),
                Seen::Exited { .. } => panic!("the runtime stopped: {:#?}", driver.seen),
                _ => {}
            }
        }
        while let Ok((append, barrier)) = appends.try_recv() {
            if barrier {
                barriers += 1;
                // The core asks for the flush right after the append.
                std::thread::sleep(Duration::from_millis(200));
                on_barrier(append);
            } else {
                let _ = remote.journal_ack(append);
            }
        }
    }
}

#[test]
fn a_journal_ack_whose_answer_cannot_be_queued_is_not_consumed() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (send, appends) = channel();
        let runtime = Runtime::create(
            &config(ENDPOINT, &SAVE),
            Arc::new(JournalTool(Mutex::new(send))),
        )
        .expect("create");
        let mut driver = Driver::new(runtime, tool_once(), ENDPOINT);
        driver.open();
        let remote = driver.remote();
        let turn = driver.runtime.prompt("save it", "j-1").expect("prompt");
        let (reason, barriers) = run_journaled(&mut driver, &appends, turn, |append| {
            remote.arm_fault(sys::FX_FAULT_REPLY_OOM, 0).unwrap();
            // The flush's answer cannot be queued: the ack fails whole.
            assert_eq!(remote.journal_ack(append), Err(Error::OutOfMemory));
            // Not consumed: the same ack applies now, and releases the barrier.
            assert_eq!(remote.journal_ack(append), Ok(Outcome::Applied));
            assert_eq!(remote.journal_ack(append), Ok(Outcome::Stale));
        });
        assert!(barriers >= 1, "the writes tool's intent was a barrier");
        assert_eq!(reason, StopReason::EndTurn);
        assert_eq!(driver.text_of(turn), "saved");
        driver.runtime.close();
    });
}

#[test]
fn a_journal_failure_whose_answer_cannot_be_queued_is_not_consumed() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (send, appends) = channel();
        let runtime = Runtime::create(
            &config(ENDPOINT, &SAVE),
            Arc::new(JournalTool(Mutex::new(send))),
        )
        .expect("create");
        let mut driver = Driver::new(runtime, tool_once(), ENDPOINT);
        driver.open();
        let remote = driver.remote();
        let turn = driver.runtime.prompt("save it", "f-1").expect("prompt");
        let (reason, barriers) = run_journaled(&mut driver, &appends, turn, |append| {
            remote.arm_fault(sys::FX_FAULT_REPLY_OOM, 0).unwrap();
            assert_eq!(
                remote.journal_fail(append, "disk full", false),
                Err(Error::OutOfMemory)
            );
            // Not consumed: the failure applies now, once.
            assert_eq!(
                remote.journal_fail(append, "disk full", false),
                Ok(Outcome::Applied)
            );
            assert_eq!(
                remote.journal_fail(append, "disk full", false),
                Ok(Outcome::Stale)
            );
        });
        assert_eq!(barriers, 1);
        assert_eq!(reason, StopReason::Error);
        // After a journal failure the session takes no prompt.
        assert_eq!(driver.runtime.prompt("more", "f-2"), Err(Error::State));
        driver.runtime.close();
    });
}
