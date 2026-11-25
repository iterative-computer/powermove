//! Smoke test: create a runtime, open a session, prompt against the vendored
//! fake inference server through the fetch bridge, and drain typed text
//! events to TURN_END. Without the core (no pinned Zig), the same test proves
//! the stub answers Unavailable.

mod support;

use std::sync::Arc;

use pm_aero_core::{Disposition, Error, Host, Runtime, StopReason};
use support::{Driver, Reply, Seen, config, on_worker};

struct NoTools;
impl Host for NoTools {}

#[test]
fn a_prompt_streams_typed_text_to_turn_end() {
    on_worker(|| {
        if !pm_aero_core::available() {
            // The stub build: every call answers Unavailable.
            let error =
                Runtime::create(&config("http://127.0.0.1:1/fx", &[]), Arc::new(NoTools)).err();
            assert_eq!(error, Some(Error::Unavailable));
            eprintln!("pm-aero-core built without the libfx core: checked the stub only");
            return;
        }
        let server = support::FakeServer::start();
        let endpoint = format!("http://127.0.0.1:{}/fx", server.port);
        let runtime = Runtime::create(&config(&endpoint, &[]), Arc::new(NoTools)).expect("create");
        let mut driver = Driver::new(runtime, Arc::new(|_| Reply::Server), &endpoint);
        let session = driver.open();
        assert!(!session.is_empty());
        let (turn, reason) = driver.turn("say it", "prompt-1");
        assert_eq!(reason, StopReason::EndTurn);
        assert_eq!(driver.text_of(turn), "xxxxx");
        // Order: the turn's start, its text, its end.
        let start = driver
            .seen
            .iter()
            .position(|e| matches!(e, Seen::TurnStart { turn: t, message_id } if *t == turn && message_id == "prompt-1"))
            .expect("TURN_START");
        let text = driver
            .seen
            .iter()
            .position(|e| matches!(e, Seen::Text { .. }))
            .expect("a text delta");
        assert!(start < text);
        assert!(matches!(driver.seen.last(), Some(Seen::TurnEnd { .. })));
        assert!(
            driver
                .fetches
                .lock()
                .unwrap()
                .iter()
                .any(|f| f.method == "POST" && f.url == endpoint && f.turn == turn)
        );
        let remote = driver.remote();
        driver.runtime.close();
        // A destroyed handle answers Closed, never touching freed memory.
        assert_eq!(remote.cancel(turn), Err(Error::Closed));
        assert_eq!(remote.fetch_disposition(1), Disposition::Retired);
    });
}
