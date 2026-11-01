use pm_mcp::{Automation, McpSession, Result, effect, store};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

fn oracle(input: Value) -> Value {
    let mut child = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/service-pairs.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn definition() -> Value {
    json!({"id":"Synthetic.effect","label":"Synthetic","group":"Test","params":[{"k":"amount","def":1}],"frag":"outColor=vec4(1.);"})
}
#[test]
fn validate_effect_matches_frozen_validator_including_error_order() {
    let mut cases = vec![
        definition(),
        json!(null),
        json!([]),
        json!({}),
        json!({"id":true}),
        json!({"id":"x"}),
    ];
    for (key, value) in [
        ("label", json!("")),
        ("group", json!(null)),
        ("backdrop", json!(1)),
        ("passes", json!(1.5)),
        ("passes", json!(0)),
        ("passes", json!(9)),
        ("passes", json!("1")),
        ("params", json!(null)),
        ("params", json!([null])),
        ("params", json!([{}])),
        ("params", json!([{"k":"Upper"}])),
        ("params", json!([{"k":"a"},{"k":"a"}])),
        ("frag", json!("")),
        ("frag", json!("x".repeat(65537))),
        ("frag", json!("😀".repeat(32768))),
        ("frag", json!("😀".repeat(32769))),
        ("passes", json!(null)),
    ] {
        let mut d = definition();
        d[key] = value;
        cases.push(d);
    }
    for d in cases {
        let expected = oracle(json!({"tool":"validate_effect","arguments":{"definition":d}}));
        match effect::validate(&d) {
            Ok(result) => assert_eq!(result, expected["result"], "{d}"),
            Err(e) => assert_eq!(e.message, expected["error"].as_str().unwrap(), "{d}"),
        }
    }
}

struct Mock(Mutex<VecDeque<(store::Request, store::Response)>>);
impl Mock {
    fn new(pairs: Vec<(store::Request, Value)>) -> Self {
        Self(Mutex::new(
            pairs
                .into_iter()
                .map(|(r, v)| {
                    (
                        r,
                        store::Response {
                            status: 200,
                            body: v.to_string().into_bytes(),
                        },
                    )
                })
                .collect(),
        ))
    }
    fn done(&self) {
        assert!(self.0.lock().unwrap().is_empty());
    }
}
impl store::StoreRead for Mock {
    fn get(&self, request: store::Request) -> Result<store::Response> {
        let (expected, response) = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected Store request");
        assert_eq!(request, expected);
        Ok(response)
    }
}
fn request(path: &str) -> store::Request {
    store::Request {
        path: path.into(),
        query: vec![],
        max_bytes: store::JSON_LIMIT,
    }
}
fn listing() -> Value {
    json!({"repoId":"synthetic-repo","slug":"synthetic","name":"Synthetic","tagline":"Fixture","category":"effects","installCount":12,"owner":{"handle":"synthetic-owner","verified":true,"privateField":"must not leak"},"permissions":["assets"],"latest":{"id":"release-1","version":"1.2.3"},"iconUrl":"ignored","futureMetadata":{"keep":"remote"}})
}
#[test]
fn store_search_matches_tag_and_maps_only_frozen_fields_and_query() {
    let page = json!({"items":[listing()],"nextCursor":"page-2"});
    let args =
        json!({"query":"hello & 世界","category":"effects","sort":"name","cursor":"next/one"});
    let mut req = request("/v1/store/extensions");
    req.query = vec![
        ("q".into(), "hello & 世界".into()),
        ("category".into(), "effects".into()),
        ("sort".into(), "name".into()),
        ("cursor".into(), "next/one".into()),
    ];
    let mock = Mock::new(vec![(req, page.clone())]);
    assert_eq!(
        store::search(&mock, &args).unwrap(),
        oracle(json!({"tool":"store_search","arguments":args,"page":page}))["result"]
    );
    mock.done();
    let mock = Mock::new(vec![(
        request("/v1/store/extensions"),
        json!({"items":[],"nextCursor":null}),
    )]);
    assert_eq!(
        store::search(&mock, &json!({"query":"","cursor":""})).unwrap(),
        json!({"items":[],"nextCursor":null})
    );
    mock.done();
}
#[test]
fn store_extension_matches_tag_with_full_release_history() {
    let mut detail = listing();
    detail["repoId"] = json!("repo");
    detail["about"] = json!("About");
    detail["releases"] = json!([{"id":"old","version":"1.0.0","apiVersion":1,"publishedAt":"2026-01-01","yankedAt":"2026-02-01","fileCount":2,"sizeBytes":123,"manifest":{"permissions":[]}},
        {"id":"release-1","version":"1.2.3","apiVersion":3,"publishedAt":"2026-03-01","yankedAt":null,"fileCount":3,"sizeBytes":456,"manifest":{"permissions":["assets"]}}]);
    let args = json!({"handle":"synthetic-owner","slug":"synthetic"});
    let mock = Mock::new(vec![(
        request("/v1/store/x/synthetic-owner/synthetic"),
        detail.clone(),
    )]);
    assert_eq!(
        store::detail(&mock, &args).unwrap(),
        oracle(json!({"tool":"store_extension","arguments":args,"detail":detail}))["result"]
    );
    mock.done();
}
#[test]
fn store_source_tree_matches_tag_without_exposing_hashes_or_extra_metadata() {
    let args = json!({"releaseId":"release-1"});
    let tree = json!({"treeSha":"tree","files":[{"path":"index.ts","size":42,"sha":"not in tool result"}]});
    let mock = Mock::new(vec![(
        request("/v1/store/releases/release-1/tree"),
        tree.clone(),
    )]);
    assert_eq!(
        store::source(&mock, &args).unwrap(),
        oracle(json!({"tool":"store_source","arguments":args,"tree":tree}))["result"]
    );
    mock.done();
}
#[test]
fn store_source_file_resolves_release_coordinate_and_encodes_path_segments() {
    let args = json!({"releaseId":"release-1","path":"src/space #?.ts"});
    let release = json!({"handle":"synthetic-owner","slug":"synthetic","version":"1.2.3"});
    let text = "// synthetic source\nexport const x = 1;\n";
    let file_request = store::Request {
        path: "/v1/store/x/synthetic-owner/synthetic/r/1.2.3/file/src/space%20%23%3F.ts".into(),
        query: vec![],
        max_bytes: store::FILE_LIMIT,
    };
    let mock = Mock(Mutex::new(VecDeque::from([
        (
            request("/v1/store/releases/release-1"),
            store::Response {
                status: 200,
                body: release.to_string().into_bytes(),
            },
        ),
        (
            file_request,
            store::Response {
                status: 200,
                body: text.as_bytes().to_vec(),
            },
        ),
    ])));
    assert_eq!(
        store::source(&mock, &args).unwrap(),
        oracle(json!({"tool":"store_source","arguments":args,"release":release,"text":text}))["result"]
    );
    mock.done();
}
#[test]
fn store_transport_errors_and_malformed_remote_results_are_errors() {
    for response in [
        store::Response {
            status: 403,
            body: b"private remote response".to_vec(),
        },
        store::Response {
            status: 200,
            body: b"not json".to_vec(),
        },
        store::Response {
            status: 200,
            body: b"{}".to_vec(),
        },
        store::Response {
            status: 200,
            body: vec![b'x'; store::JSON_LIMIT + 1],
        },
    ] {
        let mock = Mock(Mutex::new(VecDeque::from([(
            request("/v1/store/extensions"),
            response,
        )])));
        let e = store::search(&mock, &json!({})).unwrap_err();
        assert_eq!(e.code, "store_error");
        assert!(!e.message.contains("private remote"));
        mock.done();
    }
}
#[test]
fn store_source_rejects_traversal_before_transport() {
    let mock = Mock::new(vec![]);
    for path in [
        "../secret",
        "/absolute",
        "src//file",
        "src/./file",
        "src\\file",
        "nul\0file",
    ] {
        assert_eq!(
            store::source(&mock, &json!({"releaseId":"release-1","path":path}))
                .unwrap_err()
                .code,
            "invalid_arguments"
        );
    }
    for handle in [".", "..", ""] {
        assert!(store::detail(&mock, &json!({"handle":handle,"slug":"synthetic"})).is_err());
    }
    mock.done();
}
fn result(v: Value) -> Value {
    assert_eq!(v["isError"], false, "{v}");
    serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
}
#[test]
fn new_tools_obey_session_access_and_do_not_move_edit_baseline() {
    let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-wave3"));
    let mut h = Automation::new(root.clone(), root.join("exports")).unwrap();
    let id = h.create_project(&json!({"name":"Synthetic"})).unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let page = json!({"items":[],"nextCursor":null});
    h.store_reads = Some(Arc::new(Mock::new(vec![(
        request("/v1/store/extensions"),
        page.clone(),
    )])));
    let h = Arc::new(Mutex::new(h));
    let mut s = McpSession::new(h.clone());
    assert_eq!(
        s.dispatch("store_search", &json!({})).unwrap_err().code,
        "invalid_arguments"
    );
    result(s.call("start_session", json!({"prompt":"Synthetic"})));
    result(s.call("validate_effect", json!({"definition":definition()})));
    // A remote read must not make a foreign edit eligible for this run's writes.
    {
        let mut h = h.lock().unwrap();
        let r = h.revision(&id).unwrap();
        h.apply_commands(
            &id,
            vec![
                pm_kernel::CompCommand::Set {
                    patch: Box::new(pm_kernel::commands::comps::CompPatch {
                        name: Some("Foreign".into()),
                        ..Default::default()
                    }),
                }
                .into(),
            ],
            "Foreign",
            pm_kernel::Origin::Mcp,
            r,
            2,
        )
        .unwrap();
    }
    assert_eq!(result(s.call("store_search", json!({}))), page);
    assert_eq!(
        s.dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","layerType":"solid"}]})
        )
        .unwrap_err()
        .code,
        "revision_conflict"
    );
    result(s.call("finish_session", json!({"commit":true})));
    drop(s);
    drop(h);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn absent_store_host_remains_explicitly_unavailable() {
    let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-absent-store"));
    let host = Arc::new(Mutex::new(
        Automation::new(root.clone(), root.join("exports")).unwrap(),
    ));
    let mut session = McpSession::new(host.clone());
    result(session.call(
        "start_session",
        json!({"prompt":"Synthetic","context":"app"}),
    ));
    for (name, args) in [
        ("store_search", json!({})),
        (
            "store_extension",
            json!({"handle":"synthetic","slug":"synthetic"}),
        ),
        ("store_source", json!({"releaseId":"synthetic"})),
        ("fork_builtin_extension", json!({"id":"synthetic"})),
    ] {
        let e = session.dispatch(name, &args).unwrap_err();
        assert_eq!(e.code, "not_available_in_this_build");
        assert_eq!(e.backing.as_deref(), Some(pm_mcp::catalog::backing(name)));
    }
    result(session.call("finish_session", json!({"commit":true})));
    drop(session);
    drop(host);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn validator_and_store_reads_support_app_and_editor_contexts() {
    let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-app-store"));
    let mut host = Automation::new(root.clone(), root.join("exports")).unwrap();
    let page = json!({"items":[],"nextCursor":null});
    host.store_reads = Some(Arc::new(Mock::new(vec![
        (request("/v1/store/extensions"), page.clone()),
        (request("/v1/store/extensions"), page.clone()),
    ])));
    let host = Arc::new(Mutex::new(host));
    let mut session = McpSession::new(host.clone());
    result(session.call(
        "start_session",
        json!({"prompt":"Synthetic","context":"app"}),
    ));
    result(session.call("validate_effect", json!({"definition":definition()})));
    assert_eq!(result(session.call("store_search", json!({}))), page);
    result(session.call("finish_session", json!({"commit":true})));
    host.lock()
        .unwrap()
        .create_project(&json!({"name":"Synthetic"}))
        .unwrap();
    result(session.call(
        "start_session",
        json!({"prompt":"Synthetic","access":"editor"}),
    ));
    result(session.call("validate_effect", json!({"definition":definition()})));
    assert_eq!(result(session.call("store_search", json!({}))), page);
    assert_eq!(
        session
            .dispatch("fork_builtin_extension", &json!({"id":"synthetic"}))
            .unwrap_err()
            .code,
        "invalid_arguments"
    );
    result(session.call("finish_session", json!({"commit":true})));
    drop(session);
    drop(host);
    std::fs::remove_dir_all(root).unwrap();
}
