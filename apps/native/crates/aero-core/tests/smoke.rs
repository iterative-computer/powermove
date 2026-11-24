//! Milestone-1 smoke test: create a runtime, open a session, prompt against
//! the fake inference server through the fetch bridge, and drain typed text
//! events to TURN_END. Without the core (no pinned Zig), the same test proves
//! the stub answers Unavailable.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use pm_aero_core::{Config, Disposition, Error, Event, Host, Outcome, Remote, Runtime, StopReason};

struct NoTools;
impl Host for NoTools {}

fn config(endpoint: &str) -> Config<'_> {
    Config {
        endpoint_url: endpoint,
        model: "fake/model",
        instructions: "Reply with five x characters.",
        home: "/nonexistent/aero-home",
        workspace_root: "/nonexistent/aero-workspace",
        ..Config::default()
    }
}

/// The host's fetch edge for one request: sends it to the fake server and
/// streams the answer into the core.
fn serve_fetch(
    remote: Remote,
    fetch: u64,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) {
    std::thread::spawn(move || {
        // Tests never reach the network: anything but the loopback fake (the
        // model catalog on the real Gateway, say) is answered here, Not Found.
        if !url.starts_with("http://127.0.0.1:") {
            let _ = remote.fetch_status(fetch, 404, None);
            let _ = remote.fetch_finish(fetch, true);
            return;
        }
        let live = std::cell::Cell::new(true);
        let result = support::http(
            &url,
            &method,
            &headers,
            &body,
            |status| {
                live.set(remote.fetch_status(fetch, status, None) == Ok(Outcome::Applied));
            },
            |chunk| {
                if !live.get() || remote.fetch_disposition(fetch) != Disposition::Active {
                    live.set(false);
                    return;
                }
                loop {
                    match remote.fetch_push(fetch, chunk) {
                        Ok(Outcome::Backpressure) => std::thread::sleep(Duration::from_millis(1)),
                        Ok(Outcome::Applied) => break,
                        _ => {
                            live.set(false);
                            break;
                        }
                    }
                }
            },
        );
        let _ = remote.fetch_finish(fetch, result.is_ok());
    });
}

/// Everything one drained turn produced, in order.
#[derive(Debug, Default)]
struct Seen {
    session: Option<String>,
    kinds: Vec<&'static str>,
    text: String,
    end: Option<(u64, StopReason)>,
    start: Option<(u64, String)>,
    fetches: Vec<(String, String)>,
}

fn run_turn(runtime: &mut Runtime, prompt: &str, message_id: &str) -> Seen {
    let mut seen = Seen::default();
    let fd = runtime.ready_fd();
    assert!(fd >= 0);
    let open = runtime.open_session("").expect("open the session");
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut turn = None;
    while seen.end.is_none() {
        assert!(
            Instant::now() < deadline,
            "no TURN_END within 20 s; saw {seen:?}"
        );
        support::wait_ready(fd, 50);
        let mut prompt_now = false;
        for event in runtime.drain().expect("drain") {
            match event {
                Event::SessionOpened {
                    request,
                    session_id,
                    ..
                } => {
                    assert_eq!(request, open);
                    seen.session = Some(session_id.to_string());
                    seen.kinds.push("session_opened");
                    prompt_now = true;
                }
                Event::TurnStart { turn, message_id } => {
                    seen.start = Some((turn, message_id.to_string()));
                    seen.kinds.push("turn_start");
                }
                Event::TextDelta { turn: of, text } => {
                    assert_eq!(Some(of), turn, "a text delta names its turn");
                    seen.text.push_str(text);
                    seen.kinds.push("text_delta");
                }
                Event::TurnEnd {
                    turn: of, reason, ..
                } => {
                    seen.end = Some((of, reason));
                    seen.kinds.push("turn_end");
                }
                Event::Error { message, .. } => panic!("core error: {message}"),
                Event::RequestDone {
                    outcome, message, ..
                } => panic!("request failed: {outcome:?} {message}"),
                Event::Exited { code } => panic!("core exited ({code})"),
                Event::Usage { .. } => seen.kinds.push("usage"),
                _ => {}
            }
        }
        if prompt_now {
            turn = Some(runtime.prompt(prompt, message_id).expect("prompt"));
        }
        let remote = runtime.remote();
        while let Some(request) = runtime.take_fetch().expect("take fetch") {
            seen.fetches
                .push((request.method.to_string(), request.url.to_string()));
            let headers = request
                .headers()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect();
            serve_fetch(
                remote,
                request.fetch,
                request.method.to_string(),
                request.url.to_string(),
                headers,
                request.body.to_vec(),
            );
        }
    }
    assert_eq!(seen.end.map(|(t, _)| t), turn);
    seen
}

#[test]
fn a_prompt_streams_typed_text_to_turn_end() {
    // The core is never called on the main thread: this runs on a worker.
    std::thread::spawn(|| {
        if !pm_aero_core::available() {
            // The stub build: every call answers Unavailable.
            let error = Runtime::create(&config("http://127.0.0.1:1/fx"), Arc::new(NoTools)).err();
            assert_eq!(error, Some(Error::Unavailable));
            eprintln!("pm-aero-core built without the libfx core: checked the stub only");
            return;
        }
        let server = support::FakeServer::start();
        let endpoint = format!("http://127.0.0.1:{}/fx", server.port);
        let mut runtime = Runtime::create(&config(&endpoint), Arc::new(NoTools)).expect("create");
        let seen = run_turn(&mut runtime, "say it", "prompt-1");
        eprintln!("smoke: {seen:?}");
        assert!(seen.session.as_deref().is_some_and(|id| !id.is_empty()));
        assert_eq!(
            seen.start.as_ref().map(|(_, id)| id.as_str()),
            Some("prompt-1")
        );
        assert_eq!(seen.text, "xxxxx");
        assert_eq!(
            seen.end.map(|(_, reason)| reason),
            Some(StopReason::EndTurn)
        );
        // Order: the session, the turn's start, its text, its end.
        let first_text = seen
            .kinds
            .iter()
            .position(|kind| *kind == "text_delta")
            .expect("a text delta");
        let start = seen
            .kinds
            .iter()
            .position(|kind| *kind == "turn_start")
            .unwrap();
        assert!(start < first_text);
        assert_eq!(seen.kinds.last(), Some(&"turn_end"));
        assert!(
            seen.fetches
                .iter()
                .any(|(method, url)| method == "POST" && url == &endpoint)
        );
        let remote = runtime.remote();
        drop(runtime);
        // A destroyed handle answers Closed, never touching freed memory.
        assert_eq!(remote.cancel(), Err(Error::Closed));
        assert_eq!(remote.fetch_disposition(1), Disposition::Retired);
    })
    .join()
    .expect("smoke test thread");
}

#[test]
fn bad_configs_are_refused_and_leave_nothing_behind() {
    std::thread::spawn(|| {
        if !pm_aero_core::available() {
            return;
        }
        // Not loopback http: the core refuses the endpoint.
        let error = Runtime::create(&config("https://example.invalid/fx"), Arc::new(NoTools)).err();
        assert_eq!(error, Some(Error::Invalid));
        let mut bad = config("http://127.0.0.1:1/fx");
        bad.home = "";
        assert_eq!(
            Runtime::create(&bad, Arc::new(NoTools)).err(),
            Some(Error::Invalid)
        );
        // Many creates and destroys reuse slots with new generations.
        let mut stale = Vec::new();
        for _ in 0..80 {
            let runtime = Runtime::create(&config("http://127.0.0.1:1/fx"), Arc::new(NoTools))
                .expect("create");
            stale.push(runtime.remote());
        }
        for remote in stale {
            assert_eq!(remote.cancel(), Err(Error::Closed));
        }
    })
    .join()
    .expect("config test thread");
}
