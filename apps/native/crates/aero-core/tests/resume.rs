//! Durability: a journaled session written by Rust to a session folder
//! survives `kill -9` of its host mid-turn. A new runtime opens it, reports
//! the open turn and the call that may have half-run, resumes without running
//! that call again, and dedups a retried id; a checkpoint reopens it; an
//! image store's images are staged when the journal names them; a second
//! writer is fenced.

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::journal::{JournalWriter, SessionFolder, StoreError};
use pm_aero_core::{
    Host, Image, Input, JournalAppend, OpenArgs, Runtime, StopReason, Tool, ToolLease, sys,
};
use support::{
    Driver, Model, Reply, Seen, VISION_CATALOG, config, on_worker, sse_text, sse_tool_calls,
};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"target":{"type":"string"}}}"#;
const DEPLOY: [Tool<'static>; 1] = [Tool {
    name: "deploy",
    description: "Deploys the project. Not safe to run twice.",
    input_schema_json: SCHEMA,
    idempotent: false,
    writes: true,
}];
const CHILD: &str = "AERO_RESUME_CHILD";

/// A journaled host: appends go to the session folder's writer thread; tool
/// calls go to the test (or are held for ever, in the child).
struct Journaled {
    writer: Arc<JournalWriter>,
    calls: Mutex<Option<Sender<ToolLease>>>,
    appends: AtomicUsize,
}

impl Host for Journaled {
    fn journaled(&self) -> bool {
        true
    }
    fn journal_append(&self, append: JournalAppend<'_>) {
        self.appends.fetch_add(1, Ordering::SeqCst);
        self.writer.enqueue(&append);
    }
    fn tool_call(&self, call: ToolLease) {
        if let Some(calls) = self.calls.lock().unwrap().as_ref() {
            let _ = calls.send(call);
        }
    }
}

fn journaled(dir: &Path, calls: Option<Sender<ToolLease>>) -> Arc<Journaled> {
    // A killed writer's lock goes with its process; allow its exit a moment.
    let deadline = Instant::now() + Duration::from_secs(5);
    let folder = loop {
        match SessionFolder::open(dir) {
            Ok(folder) => break folder,
            Err(StoreError::Fenced) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(error) => panic!("hold the session folder: {error:?}"),
        }
    };
    Arc::new(Journaled {
        writer: JournalWriter::start(folder).expect("journal writer"),
        calls: Mutex::new(calls),
        appends: AtomicUsize::new(0),
    })
}

fn start(dir: &Path, model: Model, calls: Option<Sender<ToolLease>>) -> (Driver, Arc<Journaled>) {
    let host = journaled(dir, calls);
    let runtime = Runtime::create(&config(ENDPOINT, &DEPLOY), host.clone()).expect("create");
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

fn turn_end(driver: &mut Driver, turn: u64) -> StopReason {
    match driver.until(30, |e| {
        matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn) || matches!(e, Seen::Exited { .. })
    }) {
        Seen::TurnEnd { reason, .. } => reason,
        other => panic!("turn ended with {other:?}"),
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aero-resume-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The child: runs a journaled turn until the model's deploy call is made,
/// waits for every append to be stored, says READY, and holds the call
/// until it is killed.
fn child(dir: PathBuf) {
    on_worker(move || {
        let (send, calls) = channel();
        let model: Model = Arc::new(|call| match call.index {
            0 => Reply::Sse(sse_tool_calls("deploy", &[r#"{"target":"prod"}"#])),
            _ => Reply::Sse(sse_text("deployed")),
        });
        let (mut driver, host) = start(&dir, model, Some(send));
        let request = driver.runtime.open_session("").expect("open");
        assert!(matches!(opened(&mut driver, request), Seen::Opened { .. }));
        driver
            .runtime
            .prompt("deploy to prod", "m-1")
            .expect("prompt");
        let deadline = Instant::now() + Duration::from_secs(20);
        let held = loop {
            driver.pump(10);
            if let Ok(call) = calls.try_recv() {
                break call;
            }
            assert!(Instant::now() < deadline, "no deploy call");
        };
        // The journal holds the running call once its appends are stored.
        let settle = Instant::now() + Duration::from_millis(500);
        while Instant::now() < settle {
            driver.pump(10);
        }
        assert!(
            host.appends.load(Ordering::SeqCst) > 0,
            "the turn was journaled"
        );
        std::fs::write(dir.join("READY"), b"").unwrap();
        loop {
            driver.pump(50);
            let _ = &held;
        }
    });
}

#[test]
fn a_session_resumes_after_its_host_is_killed_mid_turn() {
    if let Ok(dir) = std::env::var(CHILD) {
        child(PathBuf::from(dir));
        return;
    }
    if !pm_aero_core::available() {
        return;
    }
    let dir = scratch("kill");
    let mut process = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_session_resumes_after_its_host_is_killed_mid_turn",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the child host");
    let deadline = Instant::now() + Duration::from_secs(60);
    let ready = loop {
        if dir.join("READY").exists() {
            break true;
        }
        if Instant::now() > deadline || process.try_wait().ok().flatten().is_some() {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // SAFETY: the pid is the child this test started.
    unsafe { libc::kill(process.id() as i32, libc::SIGKILL) };
    let _ = process.wait();
    assert!(ready, "the child never reached its tool call");

    let dir2 = dir.clone();
    on_worker(move || {
        let dir = dir2;
        let stored = SessionFolder::load(&dir).expect("load");
        assert!(
            !stored.journal_json.is_empty(),
            "the child's appends were stored"
        );
        let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
        let recorded = bodies.clone();
        let model: Model = Arc::new(move |call| {
            recorded.lock().unwrap().push(call.body.clone());
            Reply::Sse(sse_text(
                "I cannot tell whether the deploy ran; checking first.",
            ))
        });
        let (send, calls) = channel();
        let (mut driver, host) = start(&dir, model, Some(send));
        let request = driver
            .runtime
            .open(&OpenArgs {
                journal_json: &stored.journal_json,
                checkpoint: &stored.checkpoint,
                ..OpenArgs::default()
            })
            .expect("open from the journal");
        let opened_event = opened(&mut driver, request);
        assert!(
            matches!(
                opened_event,
                Seen::Opened {
                    resumable: true,
                    ..
                }
            ),
            "{opened_event:?}"
        );
        let recovery: Vec<(i32, String, String)> = driver
            .seen
            .iter()
            .filter_map(|e| match e {
                Seen::Recovery {
                    request: r,
                    kind,
                    text,
                    text2,
                    ..
                } if *r == request => Some((*kind, text.clone(), text2.clone())),
                _ => None,
            })
            .collect();
        assert!(
            recovery.contains(&(sys::FX_RECOVERY_OPEN_TURN, "m-1".into(), String::new())),
            "{recovery:?}"
        );
        assert!(
            recovery
                .iter()
                .any(|(kind, name, id)| *kind == sys::FX_RECOVERY_CALL
                    && name == "deploy"
                    && !id.is_empty()),
            "the call that may have half-run is reported: {recovery:?}"
        );
        // Resume: the call is never run again; the model reads that it may
        // have run, and decides.
        let turn = driver.runtime.resume().expect("resume");
        assert!(
            driver.until(
                10,
                |e| matches!(e, Seen::TurnStart { turn: t, .. } if *t == turn)
            ) == Seen::TurnStart {
                turn,
                message_id: "m-1".into(),
                repeated: false,
                resumed: true
            }
        );
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        assert!(
            calls.try_recv().is_err(),
            "the non-idempotent call was re-run"
        );
        assert!(!bodies.lock().unwrap().is_empty());
        // A retry of the resumed turn's id runs nothing.
        let retry = driver
            .runtime
            .prompt("deploy to prod", "m-1")
            .expect("retry");
        assert_eq!(turn_end(&mut driver, retry), StopReason::EndTurn);
        assert!(
            driver.seen.iter().any(
                |e| matches!(e, Seen::TurnStart { turn: t, repeated: true, .. } if *t == retry)
            )
        );

        // A checkpoint at the stored head reopens the session.
        let settle = Instant::now() + Duration::from_millis(300);
        while Instant::now() < settle {
            driver.pump(10);
        }
        let request = driver.runtime.checkpoint().expect("checkpoint");
        let Seen::Checkpoint { at_seq, bytes, .. } = driver.until(20, |e| {
            matches!(e, Seen::Checkpoint { request: r, .. } | Seen::RequestDone { request: r, .. } if *r == request)
        }) else {
            panic!("no checkpoint: {:#?}", driver.seen)
        };
        assert!(!bytes.is_empty());
        assert!(
            host.writer
                .folder()
                .save_checkpoint(at_seq, &bytes)
                .expect("store"),
            "the checkpoint covers the stored head"
        );
        driver.runtime.close();
        host.writer.stop();
        drop(host);

        let stored = SessionFolder::load(&dir).expect("load");
        assert!(!stored.checkpoint.is_empty());
        let (mut driver, host) = start(&dir, Arc::new(|_| Reply::Sse(sse_text("ok"))), None);
        let request = driver
            .runtime
            .open(&OpenArgs {
                journal_json: &stored.journal_json,
                checkpoint: &stored.checkpoint,
                ..OpenArgs::default()
            })
            .expect("open from the checkpoint");
        let event = opened(&mut driver, request);
        assert!(
            matches!(event, Seen::Opened { resumable: false, turns, .. } if turns >= 1),
            "{event:?}"
        );
        assert!(driver.seen.iter().any(|e| matches!(e,
            Seen::Recovery { kind, text, .. } if *kind == sys::FX_RECOVERY_LAST_TURN && text == "m-1")));
        let (_, reason) = driver.turn("and now?", "m-2");
        assert_eq!(reason, StopReason::EndTurn);
        driver.runtime.close();
        host.writer.stop();
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_writer_is_fenced() {
    if std::env::var(CHILD).is_ok() || !pm_aero_core::available() {
        return;
    }
    let dir = scratch("fence");
    let held = SessionFolder::open(&dir).expect("first writer");
    assert!(matches!(SessionFolder::open(&dir), Err(StoreError::Fenced)));
    // An append that is not the next one is fenced too.
    held.append(1, 2, "[{\"seq\":1},{\"seq\":2}]")
        .expect("first append");
    assert!(matches!(
        held.append(5, 5, "[{\"seq\":5}]"),
        Err(StoreError::Fenced)
    ));
    drop(held);
    SessionFolder::open(&dir).expect("free once the first writer is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_journal_naming_stored_images_opens_once_they_are_staged() {
    if std::env::var(CHILD).is_ok() {
        return;
    }
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let dir = scratch("images");
        let png = include_bytes!("support/pixel.png");
        let (mut driver, host) = start(&dir, Arc::new(|_| Reply::Sse(sse_text("a pixel"))), None);
        let request = driver
            .runtime
            .open(&OpenArgs {
                image_store: true,
                inputs_durable: true,
                ..OpenArgs::default()
            })
            .expect("open");
        assert!(matches!(opened(&mut driver, request), Seen::Opened { .. }));
        // The host stores the image under its name before handing it in.
        let name = host
            .writer
            .folder()
            .put_image(png)
            .expect("store the image");
        let image = [Image {
            mime: "image/png",
            data: png,
        }];
        let turn = driver
            .runtime
            .prompt_input(
                &Input {
                    text: "what is this?",
                    images: &image,
                },
                "m-1",
            )
            .expect("prompt");
        assert_eq!(turn_end(&mut driver, turn), StopReason::EndTurn);
        let settle = Instant::now() + Duration::from_millis(300);
        while Instant::now() < settle {
            driver.pump(10);
        }
        driver.runtime.close();
        host.writer.stop();
        drop(host);

        let stored = SessionFolder::load(&dir).expect("load");
        assert!(
            stored.journal_json.contains(&name),
            "the journal names the image instead of holding it"
        );
        let (mut driver, host) = start(&dir, Arc::new(|_| Reply::Sse(sse_text("ok"))), None);
        let args = OpenArgs {
            journal_json: &stored.journal_json,
            checkpoint: &stored.checkpoint,
            image_store: true,
            inputs_durable: true,
            ..OpenArgs::default()
        };
        let request = driver.runtime.open(&args).expect("open");
        let Seen::ImagesNeeded { names, .. } = opened(&mut driver, request) else {
            panic!(
                "the core held an image it was never given: {:#?}",
                driver.seen
            )
        };
        assert_eq!(names, vec![name.clone()]);
        let bytes = SessionFolder::image(&dir, &name).expect("read the stored image");
        // A name that does not match its bytes is refused; the right one stages.
        let wrong = "0".repeat(64);
        let refused = driver
            .runtime
            .stage_images(&[(wrong.as_str(), bytes.as_slice())])
            .expect("stage");
        let Seen::RequestDone { outcome, .. } = driver.until(
            20,
            |e| matches!(e, Seen::RequestDone { request, .. } if *request == refused),
        ) else {
            unreachable!()
        };
        assert_eq!(outcome, Err(pm_aero_core::Error::Invalid));
        let staged = driver
            .runtime
            .stage_images(&[(name.as_str(), bytes.as_slice())])
            .expect("stage");
        let Seen::RequestDone { outcome, .. } = driver.until(
            20,
            |e| matches!(e, Seen::RequestDone { request, .. } if *request == staged),
        ) else {
            unreachable!()
        };
        assert_eq!(outcome, Ok(pm_aero_core::Outcome::Applied));
        let request = driver.runtime.open(&args).expect("open again");
        let event = opened(&mut driver, request);
        assert!(matches!(event, Seen::Opened { turns: 1, .. }), "{event:?}");
        driver.runtime.close();
        host.writer.stop();
        let _ = std::fs::remove_dir_all(&dir);
    });
}
