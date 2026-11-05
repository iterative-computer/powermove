mod common;
mod pro_fixture;
use pm_kernel::{
    Caller, HistoryCall, KernelActor, OpenSource, Request,
    pro_mcp::{ProMcp, TOOL},
};
use pm_model::Value;
use pro_fixture::*;
use serde_json::{Value as Json, json};
use std::{
    future::Future,
    io::Write,
    pin::pin,
    process::{Command, Stdio},
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};
fn value(v: Json) -> Value {
    pm_model::json::parse(&v.to_string()).unwrap()
}
fn rpc(id: u32, args: Json) -> Json {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":TOOL,"arguments":args}})
}
fn decoded(v: &Json) -> Json {
    serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn block_on<F: Future>(f: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let w = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&w);
    let mut f = pin!(f);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}
fn process(messages: Vec<Json>, layers: Vec<Json>) -> Vec<Json> {
    let temp = common::TempDir::new("pro-mcp");
    let path = fs_canonical(temp.path("project.json"));
    let source = pm_model::json::to_canonical_string(&pm_model::canonical::realize_project(root(
        &store(layers),
    )));
    std::fs::write(&path, &source).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_pm-pro-edit-mcp"))
        .arg("--project")
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for message in messages {
        writeln!(input, "{message}").unwrap();
    }
    drop(input);
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        source,
        "standalone session never writes the input"
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn fs_canonical(path: std::path::PathBuf) -> std::path::PathBuf {
    path.parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(path.file_name().unwrap())
}
#[test]
fn real_stdio_mcp_drives_link_ripple_inspect_undo_and_redo() {
    let replies = process(
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"synthetic","version":"1"}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            rpc(3, json!({"op":"inspect","layerIds":[]})),
            rpc(
                4,
                json!({"op":"link","videoId":"v","audioId":"a","linkId":"pair"}),
            ),
            rpc(
                5,
                json!({"op":"rippleTrim","layerId":"v","edge":"out","delta":0.375,"tracks":[{"track":0},{"track":1}],"linked":true}),
            ),
            rpc(6, json!({"op":"inspect","layerIds":["v","a","tail"]})),
            rpc(7, json!({"op":"undo"})),
            rpc(8, json!({"op":"inspect","layerIds":["v","a","tail"]})),
            rpc(9, json!({"op":"redo"})),
            rpc(10, json!({"op":"inspect","layerIds":["v","a","tail"]})),
        ],
        vec![
            clip("v", 0., 4., 0),
            audio("a", 0., 4., 1),
            clip("tail", 4., 4., 0),
        ],
    );
    assert_eq!(replies.len(), 10);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[1]["result"]["tools"][0]["name"], TOOL);
    for reply in &replies[2..] {
        assert_eq!(reply["result"]["isError"], false, "{reply}");
    }
    let edited = decoded(&replies[5]);
    assert_eq!(edited["layers"][0]["dur"], 4.375);
    assert_eq!(edited["layers"][1]["dur"], 4.375);
    assert_eq!(edited["layers"][2]["from"], 4.375);
    assert_eq!(edited["syncBadges"].as_array().unwrap().len(), 2);
    let undone = decoded(&replies[7]);
    assert_eq!(undone["layers"][0]["dur"], 4.);
    assert_eq!(undone["layers"][2]["from"], 4.);
    assert_eq!(decoded(&replies[9])["layers"], edited["layers"]);
}
#[test]
fn real_stdio_source_marks_av_insert_and_explicit_decline() {
    let replies = process(
        vec![
            rpc(
                1,
                json!({"op":"sourceMarks","assetId":"src","sourceIn":2.,"sourceOut":4.,"timelineIn":4.}),
            ),
            rpc(
                2,
                json!({"op":"sourceEdit","mode":"insert","targets":[{"media":"video","track":{"track":0},"layerId":"new-v"},{"media":"audio","track":{"track":1},"layerId":"new-a"}],"linkId":"source-pair","splits":[]}),
            ),
            rpc(
                3,
                json!({"op":"inspect","layerIds":["new-v","new-a","tail"]}),
            ),
            rpc(4, json!({"op":"shuttle","key":"reverse"})),
            rpc(5, json!({"op":"undo"})),
            rpc(6, json!({"op":"inspect","layerIds":[]})),
        ],
        vec![clip("tail", 4., 4., 0)],
    );
    for reply in &replies {
        assert_eq!(reply["result"]["isError"], false, "{reply}");
    }
    let inspected = decoded(&replies[2]);
    assert_eq!(inspected["layers"][2]["from"], 6.);
    assert_eq!(inspected["sourceMonitor"]["sourceIn"], 2.);
    assert_eq!(inspected["clockAvailable"], false);
    assert_eq!(decoded(&replies[3])["declined"], true);
    assert_eq!(decoded(&replies[5])["layers"].as_array().unwrap().len(), 1);
}
#[test]
fn refusals_and_bad_rpc_are_visible_and_notifications_cannot_mutate() {
    let mut locked = clip("c", 8., 4., 0);
    locked["lock"] = json!(true);
    let replies = process(
        vec![
            rpc(
                1,
                json!({"op":"rippleDelete","layerIds":["b"],"tracks":[{"track":0}],"linked":true}),
            ),
            rpc(
                2,
                json!({"op":"move","layerIds":["b"],"delta":1.,"linked":true,"available":true}),
            ),
            json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":TOOL,"arguments":{"op":"delete","layerIds":["b"],"linked":true}}}),
            rpc(3, json!({"op":"inspect","layerIds":[]})),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"unregistered"}}),
            json!({"jsonrpc":"2.0","id":5,"method":"unknown"}),
            json!({"jsonrpc":"wrong","id":6,"method":"ping"}),
        ],
        vec![clip("a", 0., 4., 0), clip("b", 4., 4., 0), locked],
    );
    assert_eq!(replies.len(), 6);
    assert_eq!(replies[0]["result"]["isError"], true);
    assert!(decoded(&replies[0]).to_string().contains("locked"));
    assert_eq!(replies[1]["result"]["isError"], true);
    assert_eq!(decoded(&replies[2])["layers"].as_array().unwrap().len(), 3);
    assert_eq!(replies[3]["error"]["code"], -32602);
    assert_eq!(replies[4]["error"]["code"], -32601);
    assert_eq!(replies[5]["error"]["code"], -32600);
}
#[test]
fn competing_clients_require_inspection_and_reopen_never_rebinds() {
    let actor = KernelActor::spawn(Default::default());
    let h = actor.handle();
    let source = root(&store(vec![clip("a", 0., 4., 0)]));
    h.open("P", OpenSource::Untitled(source.clone()))
        .wait()
        .unwrap()
        .unwrap();
    let mut first = ProMcp::new(h.clone(), "P", Caller::native()).unwrap();
    let mut second = ProMcp::new(h.clone(), "P", Caller::native()).unwrap();
    let edit = value(rpc(
        1,
        json!({"op":"move","layerIds":["a"],"delta":1.,"linked":true}),
    ));
    let call = |m: &mut ProMcp, v: &Value| {
        let v = block_on(m.handle(v)).unwrap();
        serde_json::from_str::<Json>(&pm_model::json::to_canonical_string(&v)).unwrap()
    };
    assert_eq!(call(&mut first, &edit)["result"]["isError"], false);
    assert_eq!(call(&mut second, &edit)["result"]["isError"], true);
    let inspect = value(rpc(2, json!({"op":"inspect","layerIds":[]})));
    assert_eq!(call(&mut second, &inspect)["result"]["isError"], false);
    assert_eq!(call(&mut second, &edit)["result"]["isError"], false);
    h.call_blocking("P", Request::new(Caller::native(), HistoryCall::Undo))
        .unwrap();
    assert_eq!(call(&mut second, &edit)["result"]["isError"], true);
    h.close("P").wait().unwrap();
    h.open("P", OpenSource::Untitled(source))
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(call(&mut first, &inspect)["result"]["isError"], true);
}
#[test]
fn stdio_framing_reports_parse_errors_and_caps_input_size() {
    let temp = common::TempDir::new("pro-mcp-framing");
    let path = fs_canonical(temp.path("source.json"));
    std::fs::write(
        &path,
        pm_model::json::to_canonical_string(&pm_model::canonical::realize_project(root(&store(
            vec![],
        )))),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_pm-pro-edit-mcp"))
        .arg("--project")
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"not-json\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let lines: Vec<Json> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["error"]["code"], -32700);
    assert_eq!(lines[1]["id"], 2);
    let mut child = Command::new(env!("CARGO_BIN_EXE_pm-pro-edit-mcp"))
        .arg("--project")
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Exact-cap input has no newline and is invalid JSON; cap+1 must refuse.
    let mut input = child.stdin.take().unwrap();
    let _ = input.write_all(&vec![b'x'; 2 * 1024 * 1024 + 1]);
    drop(input);
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("exceeds 2 MiB"));
}

#[test]
fn mcp_origin_always_respects_protected_hand_edits() {
    let mut protected = clip("protected", 0., 4., 0);
    protected["locked_intent"] = json!({"timing":true});
    let replies = process(
        vec![
            rpc(
                1,
                json!({"op":"move","layerIds":["protected"],"delta":1.,"linked":true}),
            ),
            rpc(2, json!({"op":"inspect","layerIds":["protected"]})),
        ],
        vec![protected],
    );
    assert_eq!(replies[0]["result"]["isError"], true);
    assert_eq!(decoded(&replies[1])["layers"][0]["from"], 0.);
}
