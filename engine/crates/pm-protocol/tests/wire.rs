//! Wire-level tests: every message kind round-trips through the codec, the
//! golden fixtures and the checked-in TypeScript are what the code generates
//! (regeneration is a no-op), and hostile bytes never panic the decoder.

use std::collections::BTreeSet;

use pm_protocol::codec::{decode, decode_unvalidated, encode};
use pm_protocol::envelope::Message;
use pm_protocol::samples;
use pm_protocol::Envelope;
use proptest::prelude::*;

#[test]
fn every_sample_round_trips_through_messagepack_and_json() {
    for (name, envelope) in samples::all() {
        let bytes = encode(&envelope).unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(back, envelope, "{name} changed through MessagePack");
        let json = serde_json::to_string(&envelope).unwrap();
        let back: Envelope = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{name} JSON: {e}"));
        assert_eq!(back, envelope, "{name} changed through JSON");
    }
}

#[test]
fn samples_cover_every_message_kind() {
    let mut requests = BTreeSet::new();
    let mut responses = BTreeSet::new();
    let mut events = BTreeSet::new();
    let mut control = BTreeSet::new();
    for (_, envelope) in samples::all() {
        match &envelope.message {
            Message::Request { body, .. } => {
                requests.insert(samples::request_index(body));
            }
            Message::Response { body, .. } => {
                responses.insert(samples::response_index(body));
            }
            Message::Event { body } => {
                events.insert(samples::event_index(body));
            }
            Message::Hello(_) => {
                control.insert("hello");
            }
            Message::Welcome(_) => {
                control.insert("welcome");
            }
            Message::Error { .. } => {
                control.insert("error");
            }
            Message::Cancel { .. } => {
                control.insert("cancel");
            }
        }
    }
    assert_eq!(requests.len(), samples::REQUEST_VARIANTS, "a request kind has no sample");
    assert_eq!(responses.len(), samples::RESPONSE_VARIANTS, "a response kind has no sample");
    assert_eq!(events.len(), samples::EVENT_VARIANTS, "an event kind has no sample");
    assert_eq!(control.len(), 4);
    let names: BTreeSet<String> = samples::all().into_iter().map(|(name, _)| name).collect();
    assert_eq!(names.len(), samples::all().len(), "fixture names are unique");
}

#[test]
fn every_request_kind_has_a_matching_response_sample() {
    use pm_protocol::envelope::Request;
    let responses: Vec<_> = samples::responses()
        .into_iter()
        .filter_map(|e| match e.message {
            Message::Response { body, .. } => Some(body.kind()),
            _ => None,
        })
        .collect();
    for envelope in samples::requests() {
        if let Message::Request { body, .. } = &envelope.message {
            assert!(responses.contains(&body.expects()), "{:?} has no response sample", body.expects());
            if !matches!(body, Request::OpenSession(_)) && body.session_bound() {
                assert!(envelope.session.is_some());
            }
        }
    }
}

#[test]
fn golden_fixtures_are_current() {
    let fixtures = samples::fixture_files().unwrap();
    let dir = samples::fixture_dir();
    for (name, bytes) in &fixtures {
        let on_disk = std::fs::read(dir.join(name)).unwrap_or_else(|_| panic!("missing fixture {name}; run pm-protocol-gen"));
        assert!(on_disk == *bytes, "fixture {name} is out of date; run pm-protocol-gen and review the wire change");
        decode(&on_disk).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".msgpack"))
        .collect();
    let expected: BTreeSet<String> = fixtures.keys().cloned().collect();
    assert_eq!(on_disk, expected, "stale or missing fixtures; run pm-protocol-gen");
}

#[cfg(feature = "ts")]
#[test]
fn typescript_regeneration_is_a_no_op() {
    let files = pm_protocol::ts::generate().unwrap();
    let drift = pm_protocol::ts::drift(&files, &pm_protocol::ts::output_dir()).unwrap();
    assert!(
        drift.is_empty(),
        "apps/desktop/src/shared/engine-protocol is out of date: {drift:?}\nRun: source engine/env.sh && cd engine && cargo run -p pm-protocol --bin pm-protocol-gen"
    );
}

#[cfg(feature = "ts")]
#[test]
fn typescript_has_no_bigint_and_covers_the_envelope() {
    let files = pm_protocol::ts::generate().unwrap();
    for (path, text) in &files {
        assert!(!text.contains("bigint"), "{} exposes a 64-bit integer; use a safe newtype", path.display());
        for line in text.lines().filter(|l| !l.trim_start().starts_with(['*', '/'])) {
            for token in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':')) {
                let field = token.strip_suffix(':').unwrap_or("");
                assert!(!field.contains('_'), "{} has a snake_case field {field:?}; the wire is camelCase", path.display());
            }
        }
    }
    let index = &files[std::path::Path::new("index.ts")];
    for name in ["Envelope", "Message", "Request", "Response", "Event", "Hello", "Welcome", "EngineError", "EvaluatedScene", "FrameRequest", "FrameDescriptor"] {
        assert!(index.contains(&format!("export type {{ {name} }}")), "index.ts lacks {name}");
    }
}

#[test]
fn truncated_fixtures_fail_cleanly() {
    for (name, bytes) in samples::fixture_files().unwrap() {
        for cut in [0, 1, bytes.len() / 2, bytes.len() - 1] {
            assert!(decode(&bytes[..cut]).is_err(), "{name} truncated at {cut} decoded");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2048, ..ProptestConfig::default() })]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = decode(&bytes);
    }

    #[test]
    fn mutated_fixtures_never_panic(index in 0usize..1000, position in any::<prop::sample::Index>(), value in any::<u8>()) {
        let fixtures: Vec<Vec<u8>> = samples::fixture_files().unwrap().into_values().collect();
        let mut bytes = fixtures[index % fixtures.len()].clone();
        let at = position.index(bytes.len());
        bytes[at] = value;
        if let Ok(envelope) = decode_unvalidated(&bytes) {
            // Whatever decodes re-encodes without panicking.
            let _ = pm_protocol::codec::encode_unvalidated(&envelope);
        }
    }
}
