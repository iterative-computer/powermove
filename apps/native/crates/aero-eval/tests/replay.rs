//! Replay is proved on synthetic streams: a toy harness records through the
//! recording proxy against a scripted upstream, the recording is committed,
//! and the replayer feeds it back. Regressions in the harness's requests show
//! up as one precise divergence. Set AERO_EVAL_WRITE_RECORDINGS=1 to rewrite
//! the committed recording after an intended format change.
mod support;

use pm_aero_eval::replay::recorder::{Meta, Recorder, RecordingProxy};
use pm_aero_eval::replay::replayer::{ReplayServer, Replayer};
use pm_aero_eval::replay::{Recording, Request, sse};
use serde_json::json;
use support::{Toy, openai_script, upstream};

fn committed() -> std::path::PathBuf {
    pm_aero_eval::data_root().join("replay/synthetic-toy-cut.json")
}

fn meta() -> Meta {
    Meta {
        synthetic: true,
        provider: "openai-responses".into(),
        harness: "toy".into(),
        model: "toy-model".into(),
        effort: "medium".into(),
        task: "cut-drop-flubbed-take".into(),
    }
}

fn folder() -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("run").display().to_string();
    (t, f)
}

#[test]
fn recorder_captures_a_toy_run_and_matches_the_committed_recording() {
    let up = upstream(openai_script());
    let (_t, folder) = folder();
    let proxy = RecordingProxy::start(
        &up.origin,
        Recorder::new(
            meta(),
            vec![folder.clone()],
            vec!["/prompt_cache_key".into()],
        ),
    )
    .unwrap();
    let out = Toy::new(&folder).run(&proxy.origin());
    assert_eq!(out.error, None);
    assert_eq!(out.text, "Removed Take 2 and closed the gap.");
    assert_eq!(out.tool_calls, ["project", "edit"]);
    let rec = proxy.finish().unwrap();
    assert_eq!(rec.exchanges.len(), 3);
    // Paths are normalised, credentials and headers never recorded.
    let text = serde_json::to_string(&rec).unwrap();
    assert!(!text.contains(&folder), "working folder leaked");
    assert!(text.contains("/workspace/agent/work/interview.pmv"));
    assert!(!text.contains("not-a-real-key") && !text.contains("authorization"));
    if std::env::var_os("AERO_EVAL_WRITE_RECORDINGS").is_some() {
        rec.save(&committed()).unwrap();
    }
    let want = Recording::load(&committed()).unwrap();
    let got = rec;
    assert_eq!(
        got, want,
        "recording changed; rerun with AERO_EVAL_WRITE_RECORDINGS=1 if intended"
    );
}

#[test]
fn replayer_feeds_the_recording_back_without_a_model() {
    let (_t, folder) = folder();
    let server = ReplayServer::start(Replayer::new(
        Recording::load(&committed()).unwrap(),
        vec![folder.clone()],
    ))
    .unwrap();
    let out = Toy::new(&folder).run(&server.origin());
    let report = server.finish();
    assert_eq!(out.error, None);
    assert_eq!(out.text, "Removed Take 2 and closed the gap.");
    assert_eq!(out.tool_calls, ["project", "edit"]);
    assert_eq!((out.input_tokens, out.output_tokens), (3000, 92));
    assert!(report.passed, "{report:?}");
    assert_eq!((report.consumed, report.exchanges), (3, 3));
}

#[test]
fn a_changed_system_prompt_is_one_precise_divergence() {
    let (_t, folder) = folder();
    let server = ReplayServer::start(Replayer::new(
        Recording::load(&committed()).unwrap(),
        vec![folder.clone()],
    ))
    .unwrap();
    let mut toy = Toy::new(&folder);
    toy.system = "You are a Powermove agent. Use the tools.".into();
    let out = toy.run(&server.origin());
    let report = server.finish();
    assert!(!report.passed);
    let d = report.divergence.unwrap();
    assert_eq!((d.exchange, d.kind.as_str()), (0, "request-differs"));
    assert_eq!(d.differences.len(), 1);
    assert_eq!(d.differences[0].path, "/instructions");
    // The harness was told why, and got no model reply.
    assert!(out.error.unwrap().contains("replay_divergence"));
}

#[test]
fn a_changed_tool_result_diverges_at_the_next_request() {
    // A tool result that changed (here: another working folder that is not
    // normalised) shows up in the request that carries it.
    let (_t, folder) = folder();
    let server = ReplayServer::start(Replayer::new(
        Recording::load(&committed()).unwrap(),
        vec![folder.clone()],
    ))
    .unwrap();
    let out = Toy::new("/elsewhere/run").run(&server.origin());
    let report = server.finish();
    assert!(out.error.is_some());
    let d = report.divergence.unwrap();
    assert_eq!(d.exchange, 1);
    assert!(
        d.differences.iter().any(|x| x.path.starts_with("/input/")),
        "{d:?}"
    );
}

#[test]
fn an_extra_request_and_an_early_stop_both_fail() {
    let rec = Recording::load(&committed()).unwrap();
    let (_t, folder) = folder();
    let server = ReplayServer::start(Replayer::new(rec.clone(), vec![folder.clone()])).unwrap();
    let mut toy = Toy::new(&folder);
    toy.extra_request = true;
    let out = toy.run(&server.origin());
    let report = server.finish();
    assert!(out.error.unwrap().contains("unexpected-request"));
    assert_eq!(report.divergence.unwrap().kind, "unexpected-request");
    assert!(!report.passed);

    let server = ReplayServer::start(Replayer::new(rec, vec![folder.clone()])).unwrap();
    let mut toy = Toy::new(&folder);
    toy.max_requests = 2;
    toy.run(&server.origin());
    let report = server.finish();
    assert_eq!(
        (report.consumed, report.divergence.is_none(), report.passed),
        (2, true, false)
    );
}

#[test]
fn volatile_fields_compare_by_presence_only() {
    let rec = Recording::load(&committed()).unwrap();
    let mut r = Replayer::new(rec.clone(), vec![]);
    let mut body = rec.exchanges[0].request.body.clone();
    body["prompt_cache_key"] = json!("a-different-key");
    assert!(
        r.respond(&Request {
            method: "POST".into(),
            path: "/v1/responses".into(),
            body: body.clone()
        })
        .is_ok()
    );
    let mut r = Replayer::new(rec.clone(), vec![]);
    body.as_object_mut().unwrap().remove("prompt_cache_key");
    let d = r
        .respond(&Request {
            method: "POST".into(),
            path: "/v1/responses".into(),
            body,
        })
        .unwrap_err();
    assert_eq!(d.differences[0].path, "/prompt_cache_key");
    assert_eq!(d.differences[0].actual, None);
}

#[test]
fn recordings_refuse_paths_and_keys() {
    let mut rec = Recording::load(&committed()).unwrap();
    rec.exchanges[0].request.body["leak"] = json!("/Users/someone/project.pmv");
    assert!(rec.validate().unwrap_err().contains("/users/"));
    let mut rec = Recording::load(&committed()).unwrap();
    rec.exchanges[0].request.body["leak"] = json!("sk-abcdefghijklmnopqrstuvwxyz0123");
    assert!(rec.validate().unwrap_err().contains("API key"));
    // Ordinary words with the same letters are fine.
    let mut rec = Recording::load(&committed()).unwrap();
    rec.exchanges[0].request.body["note"] = json!("a task-list with risk-free desk-work");
    rec.validate().unwrap();
}

#[test]
fn the_proxy_refuses_tls_upstreams() {
    let err = RecordingProxy::start(
        "https://api.example.invalid",
        Recorder::new(meta(), vec![], vec![]),
    )
    .err()
    .unwrap();
    assert!(err.contains("plain HTTP"));
}

#[test]
fn sse_round_trips_split_and_multiline_data() {
    let events = vec![
        sse::Event::json(Some("a"), json!({"x": 1})),
        sse::Event {
            event: None,
            data: None,
            text: Some("line one\nline two".into()),
        },
        sse::Event {
            event: None,
            data: None,
            text: Some("[DONE]".into()),
        },
    ];
    let mut bytes = Vec::new();
    for e in &events {
        e.write(&mut bytes);
    }
    // Feed one byte at a time: events only complete at blank lines.
    let mut p = sse::Parser::default();
    let mut got = Vec::new();
    for b in &bytes {
        got.extend(p.feed(std::slice::from_ref(b)));
    }
    assert_eq!(got, events);
    assert_eq!(
        sse::parse(b": keep-alive\n\nevent: x\ndata: {\"y\":2}"),
        vec![sse::Event::json(Some("x"), json!({"y": 2}))]
    );
}

#[test]
fn the_in_process_replayer_serves_aero_without_http() {
    // Aero's Rust fetch bridge calls the replayer directly.
    let rec = Recording::load(&committed()).unwrap();
    let mut r = Replayer::new(rec.clone(), vec![]);
    for e in &rec.exchanges {
        let resp = r.respond(&e.request).unwrap();
        assert_eq!(resp, e.response);
    }
    assert!(r.report().passed);
}

fn raw(origin: &str, bytes: &[u8]) -> String {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(origin.trim_start_matches("http://")).unwrap();
    s.write_all(bytes).unwrap();
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

#[test]
fn the_first_replay_error_of_any_kind_is_latched() {
    // Review scenario: a malformed request got 409, then three valid requests
    // got 200 and the report passed.
    let rec = Recording::load(&committed()).unwrap();
    let (_t, folder) = folder();
    let server = ReplayServer::start(Replayer::new(rec.clone(), vec![folder.clone()])).unwrap();
    let answer = raw(&server.origin(), b"NONSENSE\r\n\r\n");
    assert!(answer.contains("409"), "{answer}");
    let out = Toy::new(&folder).run(&server.origin());
    assert!(out.error.unwrap().contains("malformed-request"));
    let report = server.finish();
    assert!(!report.passed);
    assert_eq!(report.consumed, 0);
    assert_eq!(report.divergence.unwrap().kind, "malformed-request");

    // A body that is not JSON after a divergence does not replace it.
    let server = ReplayServer::start(Replayer::new(rec, vec![folder.clone()])).unwrap();
    let mut toy = Toy::new(&folder);
    toy.system = "changed".into();
    toy.run(&server.origin());
    let bad = b"POST /v1/responses HTTP/1.1\r\ncontent-length: 3\r\n\r\n{x}";
    assert!(raw(&server.origin(), bad).contains("request-differs"));
    let d = server.finish().divergence.unwrap();
    assert_eq!(
        (d.kind.as_str(), d.differences[0].path.as_str()),
        ("request-differs", "/instructions")
    );
}

#[test]
fn an_unfinished_request_cannot_hold_the_replay_server() {
    use std::io::Write;
    let rec = Recording::load(&committed()).unwrap();
    let server = ReplayServer::start_with_deadline(
        Replayer::new(rec, vec![]),
        std::time::Duration::from_millis(300),
    )
    .unwrap();
    // Half a request, then silence.
    let mut s =
        std::net::TcpStream::connect(server.origin().trim_start_matches("http://")).unwrap();
    s.write_all(b"POST /v1/responses HTTP/1.1\r\ncontent-length: 100\r\n\r\n{")
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    let started = std::time::Instant::now();
    let report = server.finish();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(!report.passed);
    assert_eq!(report.divergence.unwrap().kind, "malformed-request");
    drop(s);
}

#[test]
fn sse_text_survives_a_multibyte_character_split_across_reads() {
    // Review scenario: lossy decoding per chunk changed split characters.
    let e = sse::Event::json(Some("delta"), json!({"delta": "naïve — 日本語 ✓"}));
    let mut bytes = Vec::new();
    e.write(&mut bytes);
    let mut p = sse::Parser::default();
    let mut got = Vec::new();
    for b in &bytes {
        got.extend(p.feed(std::slice::from_ref(b)));
    }
    assert_eq!(got, vec![e]);
    assert!(p.errors.is_empty());
    // A line that is not UTF-8 is an error, never rewritten.
    let mut p = sse::Parser::default();
    assert!(p.feed(b"data: \xff\xfe\n\n").is_empty());
    assert_eq!(p.errors.len(), 1);
}

#[test]
fn recordings_refuse_credentials_query_keys_and_any_home_path() {
    let base = Recording::load(&committed()).unwrap();
    let with = |f: &dyn Fn(&mut Recording)| {
        let mut r = base.clone();
        f(&mut r);
        r.validate()
    };
    // Review scenarios: a credential field, a credential query parameter and
    // a Windows home path were accepted.
    for (name, edit) in [
        (
            "api_key field",
            Box::new(|r: &mut Recording| r.exchanges[0].request.body["api_key"] = json!("abc123"))
                as Box<dyn Fn(&mut Recording)>,
        ),
        (
            "nested Authorization",
            Box::new(|r: &mut Recording| {
                r.exchanges[0].request.body["meta"] = json!({"Authorization": "x"})
            }),
        ),
        (
            "x-api-key",
            Box::new(|r: &mut Recording| r.exchanges[1].request.body["x-api-key"] = json!("x")),
        ),
        (
            "query key",
            Box::new(|r: &mut Recording| {
                r.exchanges[0].request.path = "/v1/responses?key=abc".into()
            }),
        ),
        (
            "unknown path",
            Box::new(|r: &mut Recording| r.exchanges[0].request.path = "/v1/files".into()),
        ),
        (
            "windows home",
            Box::new(|r: &mut Recording| {
                r.exchanges[0].request.body["f"] = json!("C:\\Users\\someone\\a.pmv")
            }),
        ),
        (
            "forward-slash windows home",
            Box::new(|r: &mut Recording| {
                r.exchanges[0].request.body["f"] = json!("c:/Users/someone")
            }),
        ),
        (
            "temp folder",
            Box::new(|r: &mut Recording| {
                r.exchanges[0].request.body["f"] = json!("/var/folders/x/T/run")
            }),
        ),
        (
            "key in a response",
            Box::new(|r: &mut Recording| {
                r.exchanges[0].response.events[0].text =
                    Some(format!("{}{}", "AKIA", "Q".repeat(16)))
            }),
        ),
        (
            "bearer in a response",
            Box::new(|r: &mut Recording| {
                r.exchanges[2]
                    .response
                    .events
                    .push(sse::Event::json(None, json!({"h": "Bearer abc"})))
            }),
        ),
        (
            "account id",
            Box::new(|r: &mut Recording| {
                r.exchanges[0].request.body["metadata"] = json!({"user_id": "u-1"})
            }),
        ),
    ] {
        assert!(with(&*edit).is_err(), "{name} was accepted");
    }
    // Ordinary text and empty credential-named fields are fine.
    with(&|r| r.exchanges[0].request.body["note"] = json!("a basic task-list, risk-free")).unwrap();
    with(&|r| r.exchanges[0].request.body["api_key"] = json!(null)).unwrap();
}

#[test]
fn saving_validates_and_the_recorder_drops_unrecorded_query_keys() {
    let mut rec = Recording::load(&committed()).unwrap();
    rec.exchanges[0].request.body["leak"] = json!("/Users/someone/x");
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("r.json");
    assert!(rec.save(&path).unwrap_err().contains("not saved"));
    assert!(!path.exists());

    let mut r = Recorder::new(meta(), vec![], vec![]);
    let resp = rec.exchanges[0].response.clone();
    r.push(
        Request {
            method: "POST".into(),
            path: "/v1/responses?api-version=2&key=secret".into(),
            body: json!({}),
        },
        resp.clone(),
    );
    let done = r.finish().unwrap();
    assert_eq!(done.exchanges[0].request.path, "/v1/responses");
    let mut r = Recorder::new(meta(), vec![], vec![]);
    r.push(
        Request {
            method: "POST".into(),
            path: "/v1/files".into(),
            body: json!({}),
        },
        resp,
    );
    assert!(r.finish().unwrap_err().contains("never records the path"));
    // The replayer compares a live path the way it would have been recorded.
    let mut rp = Replayer::new(Recording::load(&committed()).unwrap(), vec![]);
    let mut req = rec.exchanges[0].request.clone();
    req.body = Recording::load(&committed()).unwrap().exchanges[0]
        .request
        .body
        .clone();
    req.path = "/v1/responses?key=secret".into();
    assert!(rp.respond(&req).is_ok());
}

#[test]
fn committed_recordings_are_synthetic_or_reviewed() {
    for e in std::fs::read_dir(pm_aero_eval::data_root().join("replay")).unwrap() {
        let p = e.unwrap().path();
        let r = Recording::load(&p).unwrap();
        r.validate_for_repo()
            .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    }
    let mut real = Recording::load(&committed()).unwrap();
    real.synthetic = false;
    assert!(real.validate_for_repo().unwrap_err().contains("review"));
    real.review = Some(pm_aero_eval::replay::Review {
        by: "the Chief".into(),
        at: "2026-11-24".into(),
        note: "read every exchange".into(),
    });
    real.validate_for_repo().unwrap();
}
