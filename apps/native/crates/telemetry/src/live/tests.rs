//! The whole pipeline in one process against a local stand-in for PostHog:
//! the runtime is process-global, so one test walks every step in order.
use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;

/// Batches the stand-in received, decoded.
type Seen = Arc<Mutex<Vec<Vec<Value>>>>;

fn serve(listener: TcpListener, seen: Seen, up: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if !up.load(Relaxed) {
                // Down: the connection drops without an answer.
                continue;
            }
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut length, mut gzip) = (0usize, false);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
                gzip |= lower.starts_with("content-encoding:") && lower.contains("gzip");
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            if gzip {
                let mut plain = vec![];
                flate2::read::GzDecoder::new(&body[..])
                    .read_to_end(&mut plain)
                    .unwrap();
                body = plain;
            }
            let request: Value = serde_json::from_slice(&body).unwrap();
            seen.lock()
                .unwrap()
                .push(request["batch"].as_array().cloned().unwrap_or_default());
            let reply = r#"{"results":{}}"#;
            let mut stream = stream;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
}

/// Tests that touch the process-wide telemetry state run one at a time.
pub static GLOBAL: Mutex<()> = Mutex::new(());

fn flush() {
    let (ack, done) = sync_channel(1);
    SHARED.get().unwrap().tx.send(Msg::Flush(ack)).unwrap();
    done.recv_timeout(Duration::from_secs(20)).unwrap();
}

fn names(seen: &Seen) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .flatten()
        .map(|e| e["event"].as_str().unwrap_or("").to_owned())
        .collect()
}

fn outbox_files(profile: &std::path::Path) -> usize {
    Outbox::new(profile).pending(true).len()
}

fn tool(id: &'static str) {
    crate::capture(Event::ToolSelected(ToolSelected { tool_id: id }));
}

/// The pipeline end to end against a stand-in PostHog, in order: Settings
/// decides (Sol P0), offline waits in the outbox, a panic never blocks its
/// thread and never sends its message (Sol P0, P1), opt-out drops the queue
/// and everything captured before it and stops retries (Sol P0).
#[test]
fn consent_offline_panic_and_opt_out_end_to_end() {
    let _global = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Default::default();
    let up = Arc::new(AtomicBool::new(true));
    serve(listener, seen.clone(), up.clone());
    TEST_HOST.set(format!("http://127.0.0.1:{port}")).unwrap();
    // SAFETY: read only by `decide`, on this thread.
    unsafe { std::env::set_var("PM_TELEMETRY_DEV", "1") };
    let profile = tempfile::tempdir().unwrap();
    let p = profile.path();
    // A stale file from an earlier build saying "sharing on" means nothing.
    std::fs::write(p.join("telemetry.json"), br#"{"sharing":true}"#).unwrap();
    let config = Config {
        profile: p.to_owned(),
        app_version: "2.0.0-test".into(),
        build: "test".into(),
        bundle_id: Some("com.powermove.native.dev.test".into()),
        headless: false,
        development: true,
        debug_build: true,
        displays: (2, 2.),
        metal_bytes: Some(Arc::new(|| 64 << 20)),
    };
    assert_eq!(crate::init(config), Mode::Internal);

    // Sol P0: before Settings is read nothing leaves and nothing is written.
    for _ in 0..3 {
        crate::capture(Event::CommandInvoked(CommandInvoked {
            command_id: "timeline.split",
            via: Via::Shortcut,
            count: 1,
        }));
    }
    crate::launched(800, 1200);
    flush();
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(outbox_files(p), 0);
    assert!(!crate::sharing());

    // Settings says on: what waited in memory goes, folded.
    crate::set_sharing(true);
    flush();
    let events: Vec<Value> = seen.lock().unwrap().iter().flatten().cloned().collect();
    let command = events
        .iter()
        .find(|e| e["event"] == "command_invoked")
        .unwrap();
    assert_eq!(command["properties"]["count"], 3);
    assert_eq!(command["properties"]["is_internal"], true);
    let launched = events
        .iter()
        .find(|e| e["event"] == "app_launched")
        .unwrap();
    assert_eq!(launched["properties"]["first_run"], true);

    // Sol P1: a Store id is named only after this app installed it from the
    // Store catalogue; removing anything else is not sent.
    crate::store_removed("acme.never-installed-here");
    crate::store_installed("acme.glow", true);
    flush();
    assert_eq!(
        Origin::extension("acme.glow"),
        Origin::Store("acme.glow".into())
    );
    crate::store_removed("acme.glow");
    flush();
    let store: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .filter(|e| {
            e["event"]
                .as_str()
                .is_some_and(|n| n.starts_with("store_extension"))
        })
        .map(|e| format!("{} {}", e["event"], e["properties"]["extension_id"]))
        .collect();
    assert_eq!(
        store,
        [
            r#""store_extension_installed" "acme.glow""#,
            r#""store_extension_removed" "acme.glow""#
        ]
    );
    assert_eq!(Origin::extension("acme.glow"), Origin::LOCAL);

    // Offline: the batch waits in the outbox, then goes when PostHog is back.
    up.store(false, Relaxed);
    let before = seen.lock().unwrap().len();
    tool("pen");
    flush();
    assert_eq!(Outbox::new(p).pending(false).len(), 1);
    assert_eq!(seen.lock().unwrap().len(), before);
    up.store(true, Relaxed);
    flush();
    assert!(Outbox::new(p).pending(false).is_empty());
    assert!(names(&seen).contains(&"tool_selected".to_owned()));

    // Sol P1: the panic hook doesn't wait, look up symbols or write files;
    // Sol P0: its message never leaves, only its type and location.
    let began = Instant::now();
    let _ = std::thread::Builder::new()
        .name("Jane Secret Thread".into())
        .spawn(|| panic!("Jane's Secret Launch.pmv failed"))
        .unwrap()
        .join();
    assert!(
        began.elapsed() < Duration::from_millis(500),
        "{:?}",
        began.elapsed()
    );
    assert_eq!(outbox_files(p), 0);
    flush();
    let exception = seen
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .find(|e| e["event"] == "$exception")
        .cloned()
        .unwrap();
    let list = &exception["properties"]["$exception_list"][0];
    assert_eq!(list["type"], "Panic");
    assert!(
        list["value"]
            .as_str()
            .unwrap()
            .starts_with("panic at crates/telemetry/src/live/tests.rs:")
    );
    let text = exception.to_string();
    for leak in ["Jane", "Secret", "/Users"] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }

    // w150: a hang says what was going on as it began: the last command (as
    // `command_invoked` named it), playback, the front surface and the
    // project, counted on this thread. The pending copy (sent if the app is
    // ended while it hangs) carries the same context.
    hang::reset();
    crate::capture(Event::CommandInvoked(CommandInvoked {
        command_id: "play",
        via: Via::Shortcut,
        count: 1,
    }));
    crate::note_playing(true);
    let counted_on = Arc::new(std::sync::Mutex::new(None));
    let counted = counted_on.clone();
    crate::note_editor(
        Surface::Editor,
        Some(Arc::new(move || {
            *counted.lock().unwrap() = std::thread::current().name().map(str::to_owned);
            Some((4, true))
        })),
    );
    crate::note_settings_key(true);
    crate::note_settings_key(false);
    let mut sample = Vec::new();
    backtrace::trace(|frame| {
        sample.push(frame.ip() as u64);
        sample.len() < 16
    });
    send(Msg::HangBegan(
        Box::new(hang::Context::now(false, false, sample)),
        now_ms(),
        generation(),
    ));
    flush();
    let pending: Value = serde_json::from_slice(
        &std::fs::read(Outbox::new(p).dir().join(outbox::HANG_PENDING)).unwrap(),
    )
    .unwrap();
    assert_eq!(pending[0]["properties"]["duration"], "unrecovered");
    assert_eq!(pending[0]["properties"]["last_command"], "play");
    assert!(
        pending[0]["properties"]["location"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "missing location in pending hang: {pending}"
    );
    assert_eq!(counted_on.lock().unwrap().as_deref(), Some("pm-telemetry"));
    send(Msg::HangEnded("5-10s", now_ms(), generation()));
    flush();
    let hang = seen
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .find(|e| e["event"] == "hang")
        .cloned()
        .unwrap();
    let properties = &hang["properties"];
    for (key, value) in [
        ("duration", serde_json::json!("5-10s")),
        ("last_command", serde_json::json!("play")),
        ("playing", serde_json::json!(true)),
        ("surface", serde_json::json!("editor")),
        ("main_thread_answered", serde_json::json!(false)),
        ("layer_count", serde_json::json!("2-5")),
        ("has_3d", serde_json::json!(true)),
    ] {
        assert_eq!(properties[key], value, "{key} in {hang}");
    }
    assert!(
        properties["location"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "missing location in hang: {hang}"
    );
    assert!(
        properties["stack"]
            .as_array()
            .is_some_and(|s| !s.is_empty()),
        "missing stack in hang: {hang}"
    );
    let hang_text = hang.to_string();
    for leak in ["/Users", "Jane"] {
        assert!(!hang_text.contains(leak), "{leak} in {hang_text}");
    }
    assert!(!Outbox::new(p).dir().join(outbox::HANG_PENDING).exists());
    // No project, Settings in front, a hang as playback's frames stop.
    crate::note_editor(Surface::Home, None);
    crate::note_settings_key(true);
    crate::note_playing(false);
    let context = hang::Context::now(true, true, vec![]).hang("2-5s", None);
    let value = serde_json::to_value(&context).unwrap();
    assert_eq!(value["surface"], "settings");
    assert_eq!(value["playing"], true);
    assert_eq!(value["main_thread_answered"], true);
    assert!(value.get("layer_count").is_none() && value.get("has_3d").is_none());
    assert!(value.get("location").is_none() && value.get("stack").is_none());
    hang::reset();

    // Sol P0: off drops the queue and the outbox, stops retries, and what was
    // captured before it never comes back, even when turned on again at once.
    up.store(false, Relaxed);
    tool("text");
    flush();
    assert_eq!(outbox_files(p), 1);
    tool("shape");
    crate::set_sharing(false);
    crate::set_sharing(true);
    crate::set_sharing(false);
    flush();
    assert_eq!(outbox_files(p), 0);
    up.store(true, Relaxed);
    let before = seen.lock().unwrap().len();
    tool("zoom");
    crate::quit(true);
    flush();
    assert_eq!(
        seen.lock().unwrap().len(),
        before,
        "a request left with sharing off"
    );
    assert_eq!(outbox_files(p), 0);
    let state = prefs::load(p);
    assert!(!state.session_open);
    assert!(state.sessions.last().is_some_and(|s| s.revoked));
    // Deleting telemetry's own file doesn't turn anything on.
    std::fs::remove_file(p.join("telemetry.json")).unwrap();
    tool("hand");
    flush();
    assert_eq!(seen.lock().unwrap().len(), before);
    assert!(!crate::sharing());

    // Back on: it says so; nothing from before comes back.
    crate::set_sharing(true);
    flush();
    let sent: Vec<Value> = seen.lock().unwrap()[before..]
        .iter()
        .flatten()
        .cloned()
        .collect();
    assert!(sent.iter().any(|e| e["properties"]["key"] == "telemetry"));
    for e in &sent {
        let tool = e["properties"]["tool_id"].as_str().unwrap_or("");
        assert!(!["text", "shape", "zoom", "hand"].contains(&tool), "{e}");
    }
}

/// Sol P1: a development check never sends an event not marked internal.
#[test]
fn the_dev_switch_never_sends_an_event_not_marked_internal() {
    let mut record = Record {
        uuid: "u".into(),
        event: "app_quit".into(),
        distinct_id: "d".into(),
        timestamp_ms: 0,
        properties: Map::new(),
    };
    assert!(!admissible(true, &record));
    record.properties.insert("is_internal".into(), false.into());
    assert!(!admissible(true, &record));
    record.properties.insert("is_internal".into(), true.into());
    assert!(admissible(true, &record));
    assert!(admissible(false, &record));
}

/// Sol P1: a crash leaves only from a recorded session that was sending.
#[test]
fn crashes_upload_only_from_recorded_consenting_sessions() {
    let session = |pid, started, consented, revoked| prefs::Session {
        pid,
        started,
        internal: false,
        consented,
        revoked,
    };
    let sessions = vec![
        session(10, 1000., true, false),
        session(11, 2000., false, false),
        session(12, 3000., true, true),
    ];
    assert!(sending_session(&sessions, 10, 1000.5).is_some());
    // Same pid reused by a later, unrecorded process (an MCP server, an
    // engine job): its start time differs.
    assert!(sending_session(&sessions, 10, 1500.).is_none());
    // Never sending, or turned off during it.
    assert!(sending_session(&sessions, 11, 2000.).is_none());
    assert!(sending_session(&sessions, 12, 3000.).is_none());
    // Not recorded at all.
    assert!(sending_session(&sessions, 99, 1000.).is_none());
}

/// Sol P1: only extensions this app installed from the Store are named; a
/// panel or effect whose owner merely looks like a Store id is "local".
#[test]
fn store_attribution_needs_a_verified_install() {
    let _global = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    crate::remember_store_extensions(&[]);
    assert_eq!(Origin::extension("acme.glow"), Origin::LOCAL);
    assert_eq!(Origin::extension("someone.local-thing"), Origin::LOCAL);
    crate::remember_store_extensions(&["acme.glow".into()]);
    assert_eq!(
        Origin::extension("acme.glow"),
        Origin::Store("acme.glow".into())
    );
    assert_eq!(Origin::extension("someone.local-thing"), Origin::LOCAL);
    crate::remember_store_extensions(&[]);
}
