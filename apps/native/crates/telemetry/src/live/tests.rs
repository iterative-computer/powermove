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

#[test]
fn events_leave_in_batches_wait_offline_and_stop_when_sharing_is_off() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Default::default();
    let up = Arc::new(AtomicBool::new(true));
    serve(listener, seen.clone(), up.clone());
    // SAFETY: set before any other thread of this test binary reads them.
    unsafe {
        std::env::set_var("PM_TELEMETRY_DEV", "1");
        std::env::set_var("PM_TELEMETRY_HOST", format!("http://127.0.0.1:{port}"));
    }
    let profile = tempfile::tempdir().unwrap();
    let config = Config {
        profile: profile.path().to_owned(),
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

    // Repeats of one command within a second fold into one event.
    for _ in 0..3 {
        crate::capture(Event::CommandInvoked(CommandInvoked {
            command_id: "timeline.split",
            via: Via::Shortcut,
            count: 1,
        }));
    }
    crate::capture(Event::LayerAdded(LayerAdded { kind: "text" }));
    crate::launched(800, 1200);
    flush();
    let batches = seen.lock().unwrap().clone();
    let events: Vec<&Value> = batches.iter().flatten().collect();
    let command = events
        .iter()
        .find(|e| e["event"] == "command_invoked")
        .unwrap();
    assert_eq!(command["properties"]["count"], 3);
    assert_eq!(command["properties"]["is_internal"], true);
    assert_eq!(command["properties"]["app_version"], "2.0.0-test");
    let launched = events
        .iter()
        .find(|e| e["event"] == "app_launched")
        .unwrap();
    assert_eq!(launched["properties"]["launch_to_interactive_ms"], 1200);
    assert_eq!(launched["properties"]["first_run"], true);
    assert!(names(&seen).contains(&"layer_added".to_owned()));
    let id = command["distinct_id"].as_str().unwrap().to_owned();
    assert_eq!(
        prefs::load(profile.path()).install_id.as_deref(),
        Some(id.as_str())
    );

    // Offline: the batch waits in the outbox, then goes when PostHog is back.
    up.store(false, Relaxed);
    let before = seen.lock().unwrap().len();
    crate::capture(Event::ToolSelected(ToolSelected { tool_id: "pen" }));
    flush();
    let outbox = Outbox::new(profile.path());
    assert_eq!(outbox.pending(false).len(), 1);
    assert_eq!(seen.lock().unwrap().len(), before);
    up.store(true, Relaxed);
    flush();
    assert!(outbox.pending(false).is_empty());
    assert!(names(&seen).contains(&"tool_selected".to_owned()));

    // A panic on another thread arrives as an exception with its frames.
    let _ = std::thread::Builder::new()
        .name("pm-test-worker".into())
        .spawn(|| panic!("forced panic for the telemetry check"))
        .unwrap()
        .join();
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
    assert_eq!(list["value"], "forced panic for the telemetry check");
    let frames = list["stacktrace"]["frames"].as_array().unwrap();
    assert!(
        frames.iter().any(|f| f["function"]
            .as_str()
            .is_some_and(|n| n.contains("events_leave_in_batches"))),
        "{frames:?}"
    );
    assert!(!exception.to_string().contains("/Users/"));

    // Sharing off: nothing leaves and what waited is gone.
    up.store(false, Relaxed);
    crate::capture(Event::ToolSelected(ToolSelected { tool_id: "text" }));
    flush();
    assert_eq!(outbox.pending(false).len(), 1);
    crate::set_sharing(false);
    flush();
    assert!(outbox.pending(true).is_empty());
    up.store(true, Relaxed);
    let before = seen.lock().unwrap().len();
    crate::capture(Event::ToolSelected(ToolSelected { tool_id: "shape" }));
    crate::quit(true);
    flush();
    assert_eq!(
        seen.lock().unwrap().len(),
        before,
        "a request left with sharing off"
    );
    let state = prefs::load(profile.path());
    assert!(!state.sharing && !state.session_open);

    // Back on: it says so, and sends again.
    crate::set_sharing(true);
    flush();
    let last: Vec<Value> = seen.lock().unwrap().last().cloned().unwrap();
    assert!(last.iter().any(|e| e["properties"]["key"] == "telemetry"));
}
