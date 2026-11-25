//! aero_bench: Aero's performance budgets (harness DESIGN.md 9.2), measured
//! against the linked libfx core with the model answered in process (the
//! fetch edge is this binary; no HTTP, no network, no paid model).
//!
//! Every timed row records wall time and the instructions the whole process
//! retired (`proc_pid_rusage`), so the core's threads count too. A counting
//! allocator checks the copy rows: Rust heap allocations per text delta drained
//! and per tool answer. Prints one JSON report; `tools/aero-bench-check.py`
//! reads it against a baseline.
//!
//! Run it release-built, off the main thread's way (it runs on a worker
//! thread of its own): `cargo run --release -p pm-aero-core --bin aero_bench
//! -- [--samples N] [--out report.json]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::{
    Config, Error, Event, Host, JournalAppend, OpenArgs, Outcome, Purpose, Remote, Runtime, Tool,
    ToolLease,
};

// ---------------------------------------------------------------------------
// Counting allocator: Rust heap allocations on the thread that counts
// ---------------------------------------------------------------------------

struct Counting;

thread_local! {
    static COUNTED: Cell<u64> = const { Cell::new(0) };
}

// SAFETY: forwards to the system allocator; the counter is thread-local.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = COUNTED.try_with(|count| count.set(count.get() + 1));
        // SAFETY: as GlobalAlloc::alloc.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as GlobalAlloc::dealloc.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let _ = COUNTED.try_with(|count| count.set(count.get() + 1));
        // SAFETY: as GlobalAlloc::realloc.
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn allocations() -> u64 {
    COUNTED.with(Cell::get)
}

// ---------------------------------------------------------------------------
// Measures
// ---------------------------------------------------------------------------

/// Instructions the process has retired.
fn instructions() -> u64 {
    // SAFETY: an all-zero rusage_info_v4 is valid; the call fills it.
    unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        let ok = libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            (&mut info as *mut libc::rusage_info_v4).cast(),
        );
        if ok == 0 { info.ri_instructions } else { 0 }
    }
}

fn resident() -> u64 {
    // SAFETY: as instructions(); PROC_PIDTASKINFO writes a proc_taskinfo.
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            (&mut info as *mut libc::proc_taskinfo).cast(),
            size,
        );
        info.pti_resident_size
    }
}

fn threads() -> i32 {
    // SAFETY: as resident().
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            (&mut info as *mut libc::proc_taskinfo).cast(),
            size,
        );
        info.pti_threadnum
    }
}

#[derive(Default)]
struct Samples {
    wall_ms: Vec<f64>,
    minstr: Vec<f64>,
}

impl Samples {
    fn push(&mut self, started: Instant, instr_before: u64, instr_after: u64, ended: Instant) {
        self.wall_ms.push((ended - started).as_secs_f64() * 1000.0);
        self.minstr
            .push(instr_after.saturating_sub(instr_before) as f64 / 1e6);
    }
}

fn percentiles(values: &[f64]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |q: f64| -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        sorted[((sorted.len() as f64 * q).ceil() as usize)
            .saturating_sub(1)
            .min(sorted.len() - 1)]
    };
    format!(
        "{{\"p50\":{:.4},\"p90\":{:.4},\"p99\":{:.4},\"min\":{:.4},\"max\":{:.4}}}",
        at(0.50),
        at(0.90),
        at(0.99),
        sorted.first().copied().unwrap_or(0.0),
        sorted.last().copied().unwrap_or(0.0)
    )
}

/// One row of the budget table: limits in ms (p50, then p90 or p99 if set).
struct Row {
    id: &'static str,
    what: &'static str,
    samples: Samples,
    p50_ms: f64,
    p90_ms: Option<f64>,
    p99_ms: Option<f64>,
    fx: &'static str,
}

impl Row {
    fn json(&self) -> String {
        let limit = |value: Option<f64>| value.map_or("null".to_string(), |v| format!("{v}"));
        format!(
            "\"{}\":{{\"what\":\"{}\",\"samples\":{},\"wall_ms\":{},\"minstr\":{},\"budget_ms\":{{\"p50\":{},\"p90\":{},\"p99\":{}}},\"fx\":\"{}\"}}",
            self.id,
            self.what,
            self.samples.wall_ms.len(),
            percentiles(&self.samples.wall_ms),
            percentiles(&self.samples.minstr),
            self.p50_ms,
            limit(self.p90_ms),
            limit(self.p99_ms),
            self.fx
        )
    }
}

// ---------------------------------------------------------------------------
// The in-process model and host
// ---------------------------------------------------------------------------

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;
const NAMES: [&str; 12] = [
    "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8", "t9", "t10", "t11",
];

fn tools() -> Vec<Tool<'static>> {
    NAMES
        .iter()
        .map(|name| Tool {
            name,
            description: "A host tool.",
            input_schema_json: SCHEMA,
            idempotent: true,
            writes: false,
        })
        .collect()
}

fn config<'a>(tools: &'a [Tool<'a>]) -> Config<'a> {
    Config {
        endpoint_url: ENDPOINT,
        model: "fake/model",
        instructions: "Answer briefly.",
        home: "/nonexistent/aero-home",
        workspace_root: "/nonexistent/aero-workspace",
        tools,
        ..Config::default()
    }
}

const SSE_FINISH: &str = "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\",\"raw\":\"stop\"},\"usage\":{\"inputTokens\":{\"total\":1},\"outputTokens\":{\"total\":1}}}\n\ndata: [DONE]\n\n";
const SSE_DELTA: &str = "data: {\"type\":\"text-delta\",\"id\":\"0\",\"delta\":\"xxxxx\"}\n\n";
const SSE_TOOL: &str = "data: {\"type\":\"tool-input-start\",\"id\":\"call_0\",\"toolName\":\"t3\"}\n\n\
data: {\"type\":\"tool-call\",\"toolCallId\":\"call_0\",\"toolName\":\"t3\",\"input\":\"{\\\"q\\\":\\\"a\\\"}\"}\n\n\
data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\",\"raw\":\"tool_calls\"},\"usage\":{\"inputTokens\":{\"total\":1},\"outputTokens\":{\"total\":1}}}\n\ndata: [DONE]\n\n";

/// Hands tool leases to the worker, stamped with when the host saw them.
struct Bench {
    leases: Mutex<Sender<(ToolLease, Instant, u64)>>,
    journal: Option<Mutex<Sender<(u64, String)>>>,
    /// The tool_call callback stamps itself before anything else.
    stamping: AtomicBool,
}

impl Host for Bench {
    fn tool_call(&self, call: ToolLease) {
        let (at, instr) = if self.stamping.load(Ordering::Relaxed) {
            (Instant::now(), instructions())
        } else {
            (Instant::now(), 0)
        };
        let _ = self.leases.lock().unwrap().send((call, at, instr));
    }
    fn journaled(&self) -> bool {
        self.journal.is_some()
    }
    fn journal_append(&self, append: JournalAppend<'_>) {
        if let Some(journal) = &self.journal {
            let _ = journal
                .lock()
                .unwrap()
                .send((append.append, append.events_json.to_string()));
        }
    }
}

struct Session {
    runtime: Runtime,
    leases: Receiver<(ToolLease, Instant, u64)>,
    journal: Option<Receiver<(u64, String)>>,
    stored: Vec<String>,
    host: Arc<Bench>,
}

fn session(tools: &[Tool<'static>], journaled: bool) -> Session {
    let (send, leases) = channel();
    let (journal, received) = if journaled {
        let (send, receive) = channel();
        (Some(Mutex::new(send)), Some(receive))
    } else {
        (None, None)
    };
    let host = Arc::new(Bench {
        leases: Mutex::new(send),
        journal,
        stamping: AtomicBool::new(false),
    });
    let runtime = Runtime::create(&config(tools), host.clone()).expect("create a runtime");
    Session {
        runtime,
        leases,
        journal: received,
        stored: Vec::new(),
        host,
    }
}

fn wait_ready(runtime: &Runtime, timeout_ms: i32) {
    use std::os::fd::AsRawFd;
    let Some(fd) = runtime.ready_fd() else { return };
    let fd = fd.as_raw_fd();
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd.
    unsafe { libc::poll(&mut poll, 1, timeout_ms) };
    let mut sink = [0u8; 64];
    // SAFETY: non-blocking read into a local buffer.
    while unsafe { libc::read(fd, sink.as_mut_ptr().cast(), sink.len()) } > 0 {}
}

/// What a pump found.
#[derive(Default)]
struct Found {
    opened: bool,
    turn_end: bool,
    text: bool,
    model_fetch: Option<u64>,
}

impl Session {
    fn remote(&self) -> Remote {
        self.runtime.remote()
    }

    /// One round: wait, drain, answer the catalog and journal, take a model fetch.
    fn pump(&mut self, timeout_ms: i32) -> Found {
        wait_ready(&self.runtime, timeout_ms);
        let mut found = Found::default();
        match self.runtime.events.drain() {
            Ok(events) => {
                for event in events {
                    match event {
                        Event::SessionOpened { .. } => found.opened = true,
                        Event::TurnEnd { .. } => found.turn_end = true,
                        Event::TextDelta { .. } => found.text = true,
                        Event::Exited { message, .. } => panic!("the runtime stopped: {message}"),
                        Event::RequestDone {
                            outcome: Err(error),
                            message,
                            ..
                        } => panic!("refused: {error} {message}"),
                        _ => {}
                    }
                }
            }
            Err(Error::Busy) => {}
            Err(error) => panic!("drain: {error}"),
        }
        let remote = self.remote();
        while let Ok(Some(request)) = self.runtime.fetches.take() {
            if request.purpose == Purpose::Catalog || request.url != ENDPOINT {
                let _ = remote.fetch_status(request.fetch, 404, None);
                let _ = remote.fetch_finish(request.fetch, true);
            } else {
                found.model_fetch = Some(request.fetch);
            }
        }
        if let Some(journal) = &self.journal {
            let mut last = 0;
            while let Ok((append, events)) = journal.try_recv() {
                self.stored.push(events);
                last = append;
            }
            if last > 0 {
                assert_eq!(remote.journal_ack(last), Ok(Outcome::Applied));
            }
        }
        found
    }

    fn until(&mut self, mut done: impl FnMut(&Found) -> bool) -> Found {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let found = self.pump(200);
            if done(&found) {
                return found;
            }
            assert!(Instant::now() < deadline, "the core did not answer in 30 s");
        }
    }

    fn open(&mut self) {
        self.runtime.open_session("").expect("open");
        self.until(|f| f.opened);
    }

    fn reply(&mut self, fetch: u64, body: &str) {
        let remote = self.remote();
        assert_eq!(remote.fetch_status(fetch, 200, None), Ok(Outcome::Applied));
        assert_eq!(
            remote.fetch_push(fetch, body.as_bytes()),
            Ok(Outcome::Applied)
        );
        let _ = remote.fetch_finish(fetch, true);
    }

    fn model_fetch(&mut self) -> u64 {
        self.until(|f| f.model_fetch.is_some()).model_fetch.unwrap()
    }

    /// A whole text turn, unmeasured.
    fn text_turn(&mut self, id: &str) {
        self.runtime.prompt("hello", id).expect("prompt");
        let fetch = self.model_fetch();
        self.reply(fetch, &format!("{SSE_DELTA}{SSE_FINISH}"));
        self.until(|f| f.turn_end);
    }
}

// ---------------------------------------------------------------------------
// The rows
// ---------------------------------------------------------------------------

fn run(samples: usize) -> String {
    let warm = (samples / 10).max(5);
    let tools = tools();

    // Harness start: the first core in the process.
    let started = Instant::now();
    let before = instructions();
    let first = session(&tools, false);
    let mut start_row = Row {
        id: "harness_start",
        what: "the first fx_runtime_create in a process",
        samples: Samples::default(),
        p50_ms: 1.0,
        p90_ms: None,
        p99_ms: None,
        fx: "34 ms first prompt (addon load included)",
    };
    start_row
        .samples
        .push(started, before, instructions(), Instant::now());
    first.runtime.close();

    // Session open (12 tools) and close.
    let mut open_row = Row {
        id: "session_open",
        what: "fx_runtime_create + fx_session_open to SESSION_OPENED drained, 12 tools",
        samples: Samples::default(),
        p50_ms: 0.56,
        p90_ms: Some(0.9),
        p99_ms: None,
        fx: "0.56 / 0.91 ms",
    };
    let mut close_row = Row {
        id: "close",
        what: "Runtime::close (teardown, thread joined, fd closed)",
        samples: Samples::default(),
        p50_ms: 0.1,
        p90_ms: None,
        p99_ms: None,
        fx: "0.09 ms",
    };
    let threads_before = threads();
    for index in 0..samples + warm {
        let started = Instant::now();
        let before = instructions();
        let mut one = session(&tools, false);
        one.open();
        let (after, ended) = (instructions(), Instant::now());
        let closing = Instant::now();
        let close_before = instructions();
        one.runtime.close();
        let (close_after, closed) = (instructions(), Instant::now());
        if index >= warm {
            open_row.samples.push(started, before, after, ended);
            close_row
                .samples
                .push(closing, close_before, close_after, closed);
        }
    }
    std::thread::sleep(Duration::from_millis(100));
    let threads_after = threads();

    // A warm session: prompt to request, body to first text, tool call legs.
    let mut warm_session = session(&tools, false);
    warm_session.open();
    for index in 0..warm {
        warm_session.text_turn(&format!("warm-{index}"));
    }
    let mut prompt_row = Row {
        id: "prompt_to_request",
        what: "warm fx_turn_prompt to the model request taken",
        samples: Samples::default(),
        p50_ms: 0.5,
        p90_ms: None,
        p99_ms: None,
        fx: "0.51 ms",
    };
    let mut text_row = Row {
        id: "body_to_first_text",
        what: "response bytes pushed to the first TEXT_DELTA drained by Rust",
        samples: Samples::default(),
        p50_ms: 0.19,
        p90_ms: None,
        p99_ms: None,
        fx: "0.19 ms",
    };
    let mut delta_allocations = 0u64;
    let mut deltas = 0u64;
    for index in 0..samples {
        let started = Instant::now();
        let before = instructions();
        warm_session
            .runtime
            .prompt("hello", &format!("p-{index}"))
            .expect("prompt");
        let fetch = warm_session.model_fetch();
        prompt_row
            .samples
            .push(started, before, instructions(), Instant::now());
        let remote = warm_session.remote();
        assert_eq!(remote.fetch_status(fetch, 200, None), Ok(Outcome::Applied));
        let started = Instant::now();
        let before = instructions();
        assert_eq!(
            remote.fetch_push(fetch, SSE_DELTA.as_bytes()),
            Ok(Outcome::Applied)
        );
        let counted = allocations();
        warm_session.until(|f| f.text);
        delta_allocations += allocations() - counted;
        deltas += 1;
        text_row
            .samples
            .push(started, before, instructions(), Instant::now());
        assert_eq!(
            remote.fetch_push(fetch, SSE_FINISH.as_bytes()),
            Ok(Outcome::Applied)
        );
        let _ = remote.fetch_finish(fetch, true);
        warm_session.until(|f| f.turn_end);
    }

    let mut call_row = Row {
        id: "tool_call_to_host",
        what: "the model's tool call (response bytes pushed) to the Rust tool function starting",
        samples: Samples::default(),
        p50_ms: 0.1,
        p90_ms: None,
        p99_ms: Some(0.25),
        fx: "0.68 / 1.68 ms",
    };
    let mut result_row = Row {
        id: "result_to_request",
        what: "the Rust tool result to the next model request taken (no HTTP)",
        samples: Samples::default(),
        p50_ms: 0.5,
        p90_ms: None,
        p99_ms: None,
        fx: "part of 2.6 ms",
    };
    let mut answer_allocations = 0u64;
    warm_session.host.stamping.store(true, Ordering::Relaxed);
    for index in 0..samples + warm {
        warm_session
            .runtime
            .prompt("use a tool", &format!("c-{index}"))
            .expect("prompt");
        let fetch = warm_session.model_fetch();
        let remote = warm_session.remote();
        assert_eq!(remote.fetch_status(fetch, 200, None), Ok(Outcome::Applied));
        let started = Instant::now();
        let before = instructions();
        assert_eq!(
            remote.fetch_push(fetch, SSE_TOOL.as_bytes()),
            Ok(Outcome::Applied)
        );
        let _ = remote.fetch_finish(fetch, true);
        let deadline = Instant::now() + Duration::from_secs(30);
        let (lease, at, instr) = loop {
            if let Ok(item) = warm_session.leases.try_recv() {
                break item;
            }
            warm_session.pump(1);
            assert!(Instant::now() < deadline, "no tool call");
        };
        if index >= warm {
            call_row.samples.push(started, before, instr, at);
        }
        let started = Instant::now();
        let before = instructions();
        let counted = allocations();
        lease.answer("found", false).expect("answer");
        if index >= warm {
            answer_allocations += allocations() - counted;
        }
        let next = warm_session.model_fetch();
        if index >= warm {
            result_row
                .samples
                .push(started, before, instructions(), Instant::now());
        }
        warm_session.reply(next, &format!("{SSE_DELTA}{SSE_FINISH}"));
        warm_session.until(|f| f.turn_end);
    }
    warm_session.runtime.close();

    // Resume from a 200-turn journal (fx's bench-durable --history 200: 200
    // text turns, then one tool turn), opened without a checkpoint.
    let mut source = session(&tools, true);
    source.open();
    for index in 0..200 {
        source.text_turn(&format!("h-{index}"));
    }
    source
        .runtime
        .prompt("use a tool", "h-tool")
        .expect("prompt");
    let fetch = source.model_fetch();
    source.reply(fetch, SSE_TOOL);
    let deadline = Instant::now() + Duration::from_secs(30);
    let lease = loop {
        if let Ok((lease, _, _)) = source.leases.try_recv() {
            break lease;
        }
        source.pump(1);
        assert!(Instant::now() < deadline, "no tool call");
    };
    lease.answer("found", false).expect("answer");
    let fetch = source.model_fetch();
    source.reply(fetch, &format!("{SSE_DELTA}{SSE_FINISH}"));
    source.until(|f| f.turn_end);
    for _ in 0..10 {
        source.pump(10);
    }
    let parts: Vec<&str> = source
        .stored
        .iter()
        .filter_map(|events| {
            events
                .trim()
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
        })
        .filter(|inner| !inner.trim().is_empty())
        .collect();
    let journal = format!("[{}]", parts.join(","));
    let journal_bytes = journal.len();
    source.runtime.close();
    let mut resume_row = Row {
        id: "resume_200",
        what: "fx_runtime_create + fx_session_open from a 200-turn journal to SESSION_OPENED",
        samples: Samples::default(),
        p50_ms: 0.0,
        p90_ms: None,
        p99_ms: None,
        fx: "measured: bench-durable --history 200 journal_restore_create_ms",
    };
    for index in 0..(samples / 4).max(10) + 3 {
        let started = Instant::now();
        let before = instructions();
        let mut restored = session(&tools, true);
        restored
            .runtime
            .open(&OpenArgs {
                journal_json: &journal,
                ..OpenArgs::default()
            })
            .expect("open from the journal");
        restored.until(|f| f.opened);
        let (after, ended) = (instructions(), Instant::now());
        restored.runtime.close();
        if index >= 3 {
            resume_row.samples.push(started, before, after, ended);
        }
    }

    // Memory per session: 25 sessions, resident after open and after one
    // prompt each, over the baseline, as bench-capacity measures it.
    std::thread::sleep(Duration::from_millis(100));
    let base = resident();
    let mut many: Vec<Session> = (0..25).map(|_| session(&tools, false)).collect();
    for one in &mut many {
        one.open();
    }
    let idle = (resident().saturating_sub(base)) / 25;
    for (index, one) in many.iter_mut().enumerate() {
        one.text_turn(&format!("m-{index}"));
    }
    let prompted = (resident().saturating_sub(base)) / 25;
    for one in many {
        one.runtime.close();
    }

    let rows = [
        start_row, open_row, resume_row, close_row, prompt_row, text_row, call_row, result_row,
    ];
    let body: Vec<String> = rows.iter().map(Row::json).collect();
    format!(
        "{{\"schema\":1,\"samples\":{samples},\"rows\":{{{}}},\"copies\":{{\"rust_allocations_per_text_delta\":{:.3},\"rust_allocations_per_tool_answer\":{:.3},\"tool_input\":\"borrowed from the core (ToolLease::input_ptr; tests/tools.rs)\",\"tool_result\":\"one copy, into the core (fx_tool_result)\"}},\"memory\":{{\"resident_per_session_idle_bytes\":{idle},\"resident_per_session_after_prompt_bytes\":{prompted},\"budget_idle_bytes\":358400,\"budget_after_prompt_bytes\":819200}},\"threads\":{{\"before\":{threads_before},\"after_close\":{threads_after}}},\"resume_journal_bytes\":{journal_bytes}}}",
        body.join(","),
        delta_allocations as f64 / deltas.max(1) as f64,
        answer_allocations as f64 / samples.max(1) as f64,
    )
}

fn main() {
    if !pm_aero_core::available() {
        eprintln!("aero_bench: built without the libfx core (no pinned Zig): nothing to measure");
        std::process::exit(2);
    }
    let mut samples = 200usize;
    let mut out: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--samples" => {
                samples = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--samples N")
            }
            "--out" => out = args.next(),
            other => panic!("unknown argument {other}"),
        }
    }
    // The core is never called on the main thread.
    let report = std::thread::spawn(move || run(samples))
        .join()
        .expect("bench thread");
    match out {
        Some(path) => std::fs::write(&path, format!("{report}\n")).expect("write the report"),
        None => println!("{report}"),
    }
}
