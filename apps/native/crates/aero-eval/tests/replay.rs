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
    assert!(rec.validate().unwrap_err().contains("/Users/"));
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
