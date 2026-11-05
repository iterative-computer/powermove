#[path = "../src/replay/mod.rs"]
mod replay;
mod support;
use replay::{Difference, Message, Observations};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/replay")
}
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .unwrap()
        .to_path_buf()
}

/// Drain both pipes concurrently; neither a stuck MCP process nor a full output
/// pipe can hang the gate. Always kill/reap a timeout before reporting failure.
fn execute(mut command: Command, input: Vec<u8>, timeout: Duration) -> Vec<u8> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn replay process");
    let mut stdin = child.stdin.take().unwrap();
    let write = thread::spawn(move || stdin.write_all(&input));
    let read = |mut pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut bytes = vec![];
            pipe.read_to_end(&mut bytes).unwrap();
            bytes
        })
    };
    let stdout = read(Box::new(child.stdout.take().unwrap()));
    let stderr = read(Box::new(child.stderr.take().unwrap()));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("replay process timed out");
        }
        thread::sleep(Duration::from_millis(10));
    };
    write.join().unwrap().unwrap();
    let bytes = stdout.join().unwrap();
    let err = stderr.join().unwrap();
    assert!(
        status.success(),
        "replay process failed: {}",
        String::from_utf8_lossy(&err)
    );
    bytes
}
fn mcp_binary() -> PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(build_mcp_binary).clone()
}
fn build_mcp_binary() -> PathBuf {
    // A prebuilt binary may be provided by CI. Otherwise build the real crate
    // with the inherited alternate lockfile, never add a dependency to agents.
    if let Some(p) = std::env::var_os("PM_REPLAY_MCP_BINARY") {
        return p.into();
    }
    let mut build = Command::new(env!("CARGO"));
    build.current_dir(repo().join("apps/native")).args([
        "build",
        "--locked",
        "-p",
        "pm-mcp",
        "--bin",
        "powermove-mcp",
        "--message-format=json",
    ]);
    let output = execute(build, vec![], Duration::from_secs(180));
    output
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .find_map(|v| {
            if v["reason"] == "compiler-artifact" && v["target"]["name"] == "powermove-mcp" {
                v["executable"].as_str().map(PathBuf::from)
            } else {
                None
            }
        })
        .expect("cargo did not report the native MCP executable")
}
fn mcp(fixture: &Value) -> (Observations, Observations) {
    let temp = support::Temp::new();
    let mut command = Command::new(mcp_binary());
    command
        .arg("--profile")
        .arg(temp.0.join("synthetic-profile"))
        .arg("--exports")
        .arg(temp.0.join("synthetic-exports"));
    let mut input = vec![];
    for r in fixture["requests"].as_array().unwrap() {
        input.extend_from_slice(
            if let Some(raw) = r.as_str() {
                raw.as_bytes().to_vec()
            } else {
                serde_json::to_vec(r).unwrap()
            }
            .as_slice(),
        );
        input.push(b'\n');
    }
    let output = execute(command, input, Duration::from_secs(15));
    let mut actual = Observations::default();
    for line in output.split(|b| *b == b'\n').filter(|b| !b.is_empty()) {
        actual.wire.push(Message {
            kind: "out".into(),
            after: 0,
            value: serde_json::from_slice(line).expect("non-JSON on MCP stdout"),
        });
    }
    let expected = Observations {
        wire: fixture["responses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Message {
                kind: "out".into(),
                after: 0,
                value: v.clone(),
            })
            .collect(),
        ..Observations::default()
    };
    (expected, actual)
}

/// The integration's live MCP catalog is pinned main plus the two selected
/// Studio contracts. Keep the tagged transcript immutable and its full delta
/// visible, while requiring that delta to match these exact source inputs.
fn current_mcp_catalog(reference: &Observations) -> Observations {
    let main = include_bytes!("../../mcp/fixtures/catalog-main-c7f46a056.json");
    let studio = include_bytes!("../../mcp/fixtures/scene3d-source-catalog.json");
    assert_eq!(
        replay::sha256(main),
        "8e0ae6094bd7149703ba9a49aec8e334fd21b1d58d75c4d72494fccd1a8ddddd"
    );
    assert_eq!(
        replay::sha256(studio),
        "c5c11bcbda88e89ccdd01b9ddad75787a3495f1b2cec04b084298b8b91b5c5e1"
    );
    let mut catalog: Value = serde_json::from_slice(main).unwrap();
    let studio: Value = serde_json::from_slice(studio).unwrap();
    assert_eq!(studio["tools"].as_array().unwrap().len(), 2);
    for replacement in studio["tools"].as_array().unwrap() {
        let tool = catalog["tools"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|tool| tool["name"] == replacement["name"])
            .unwrap();
        *tool = replacement.clone();
    }
    let mut expected = Observations {
        wire: reference.wire.clone(),
        events: reference.events.clone(),
        lifecycle: reference.lifecycle.clone(),
    };
    let mut lists = 0;
    for message in &mut expected.wire {
        if message.value["result"]["tools"].is_array() {
            message.value["result"]["tools"] = catalog["tools"].clone();
            lists += 1;
        }
    }
    assert_eq!(lists, 1, "the complete tools/list response is checked");
    expected
}

#[test]
fn current_catalog_delta_never_accepts_unknown_catalog_or_transport_changes() {
    let manifest = replay::load_manifest(&root());
    let case = manifest
        .cases
        .iter()
        .find(|case| case.name == "mcp-stdio")
        .unwrap();
    let fixture: Value =
        serde_json::from_slice(&fs::read(root().join(&case.file)).unwrap()).unwrap();
    let reference = Observations {
        wire: fixture["responses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| Message {
                kind: "out".into(),
                after: 0,
                value: value.clone(),
            })
            .collect(),
        ..Observations::default()
    };
    let current = current_mcp_catalog(&reference);
    let allowed = replay::report(case, &reference, &current).differences;
    assert!(
        !allowed.is_empty(),
        "tagged catalog identity is not claimed"
    );
    let list = current
        .wire
        .iter()
        .position(|message| message.value["result"]["tools"].is_array())
        .unwrap();
    for catalog_change in [true, false] {
        let mut changed = current_mcp_catalog(&reference);
        if catalog_change {
            changed.wire[list].value["result"]["tools"][0]["description"] = json!("unexpected");
        } else {
            changed.wire[0].value["unexpected"] = json!(true);
        }
        assert_ne!(
            replay::report(case, &reference, &changed).differences,
            allowed
        );
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownDifference {
    rule: String,
    reason: String,
    differences: Vec<Difference>,
}
#[test]
fn replay_all_surfaces_and_check_every_difference() {
    let m = replay::load_manifest(&root());
    let ledger: BTreeMap<String, KnownDifference> =
        serde_json::from_slice(&fs::read(root().join("differences.json")).unwrap()).unwrap();
    let mut reports = vec![];
    let mut source_deltas = BTreeMap::new();
    for c in &m.cases {
        let fixture = serde_json::from_slice(&fs::read(root().join(&c.file)).unwrap()).unwrap();
        let (expected, actual) = if c.surface == "mcp" {
            mcp(&fixture)
        } else {
            replay::provider(c, &fixture)
        };
        if c.name == "mcp-stdio" {
            source_deltas.insert(
                c.name.clone(),
                replay::report(c, &expected, &current_mcp_catalog(&expected)).differences,
            );
        }
        reports.push(replay::report(c, &expected, &actual));
    }
    let serialized = serde_json::to_string_pretty(&reports).unwrap();
    if let Some(path) = std::env::var_os("PM_REPLAY_REPORT") {
        fs::write(path, &serialized).unwrap();
    }
    println!("{serialized}");
    for r in &reports {
        if let Some(expected) = source_deltas.get(&r.name) {
            assert_eq!(
                &r.differences, expected,
                "{}: live MCP differs from pinned main/Studio source contracts",
                r.name
            );
        } else if r.identical {
            assert!(
                !ledger.contains_key(&r.name),
                "stale difference for {}",
                r.name
            );
        } else {
            let known = ledger
                .get(&r.name)
                .unwrap_or_else(|| panic!("unrecorded differences in {} (see report)", r.name));
            assert!(!known.rule.is_empty() && !known.reason.is_empty());
            assert_eq!(
                r.differences, known.differences,
                "{}: new or changed differences",
                r.name
            );
        }
    }
    for name in ledger.keys() {
        assert!(
            reports.iter().any(|r| r.name == *name),
            "orphan difference {name}"
        );
    }
    if std::env::var_os("PM_REPLAY_REQUIRE_IDENTICAL").is_some() {
        assert!(
            reports.iter().all(|r| r.identical),
            "P4 remains open: recorded differences are not an identical replay"
        );
    }
}
#[test]
fn source_hashes_pin_the_frozen_reference_and_recorder() {
    let m = replay::load_manifest(&root());
    for s in m.sources {
        let mut command = Command::new("git");
        command
            .current_dir(repo())
            .args(["show", &format!("{}:{}", replay::BASELINE, s.file)]);
        assert_eq!(
            replay::sha256(&execute(command, vec![], Duration::from_secs(10))),
            s.sha256,
            "tagged source changed: {}",
            s.file
        );
    }
    for recorder in [m.recorder, m.capture] {
        assert_eq!(
            replay::sha256(&fs::read(repo().join(recorder.file)).unwrap()),
            recorder.sha256
        );
    }
}
#[test]
fn differ_rejects_ids_schema_errors_order_omission_and_extras() {
    let golden = json!([
        {"id":1,"result":{"answers":{"colour":{"answers":["Blue"]}}}},
        {"id":"1","error":{"code":-32601,"message":"Unsupported"}}
    ]);
    for changed in [
        json!([{ "id":"1","result":{"answers":{"colour":{"answers":["Blue"]}}}},golden[1]]),
        json!([golden[1], golden[0]]),
        json!([golden[0]]),
        json!([golden[0], golden[1], golden[1]]),
        json!([golden[0],{"id":"1","error":{"code":-32602,"message":"Unsupported"}}]),
        json!([{"id":1,"result":null},golden[1]]),
    ] {
        assert!(!replay::diff(&golden, &changed).is_empty());
    }
    let d = replay::diff(&json!({"a/b~":null}), &json!({}));
    assert_eq!(d[0].path, "/a~1b~0");
    assert_eq!(d[0].expected, Some(Value::Null));
    assert_eq!(d[0].actual, None);
    assert!(replay::diff(&json!({"a":1,"b":2}), &json!({"b":2,"a":1})).is_empty());
}
#[test]
fn unknown_fixture_frames_are_refused() {
    let c = replay::Case {
        name: "unknown".into(),
        surface: "codex".into(),
        file: String::new(),
        sha256: String::new(),
    };
    assert!(std::panic::catch_unwind(||replay::provider(&c,&json!({"baseline":"v1.2.0","source":"synthetic","frames":[{"kind":"future-silently-skipped","value":{}}]}))).is_err());
}

#[test]
fn corpus_keeps_all_required_flows() {
    let m = replay::load_manifest(&root());
    let mut actual = m.cases.iter().map(|c| c.name.as_str()).collect::<Vec<_>>();
    actual.sort_unstable();
    let mut required = vec![
        "codex-happy",
        "codex-question",
        "codex-steer",
        "codex-cancel",
        "codex-compact",
        "codex-async-question",
        "codex-tools",
        "codex-retry",
        "codex-exec",
        "codex-initialize-error",
        "codex-thread-error",
        "codex-turn-error",
        "codex-missing-thread",
        "codex-missing-turn",
        "codex-turn-failed",
        "codex-context-exhausted",
        "claude-control",
        "claude-stream",
        "claude-skipped-withdrawn",
        "claude-compaction-retry",
        "claude-error",
        "mcp-stdio",
        "mcp-tool-errors",
    ];
    required.sort_unstable();
    assert_eq!(
        actual, required,
        "coverage changed: review the gate inventory"
    );
}
#[test]
fn differ_retains_absence_null_and_causal_positions_in_the_ledger() {
    let expected = json!({"a":null,"events":[{"kind":"result","after":5,"value":{"ok":false}}]});
    let actual = json!({"events":[{"kind":"result","after":6,"value":{"ok":false}}]});
    let diffs = replay::diff(&expected, &actual);
    assert_eq!(diffs.len(), 2);
    let encoded = serde_json::to_string(&diffs).unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<Difference>>(&encoded).unwrap(),
        diffs
    );
    assert_eq!(diffs[0].expected, Some(Value::Null));
    assert_eq!(diffs[0].actual, None);
    assert_eq!(diffs[1].path, "/events/0/after");
}
#[test]
fn manifest_refuses_unsupported_version_unsafe_paths_and_tampering() {
    let temp = support::Temp::new();
    let original: Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let first = original["cases"][0]["file"].as_str().unwrap();
    fs::create_dir_all(temp.0.join(first).parent().unwrap()).unwrap();
    fs::copy(root().join(first), temp.0.join(first)).unwrap();
    for changed in [
        {
            let mut v = original.clone();
            v["version"] = json!(2);
            v
        },
        {
            let mut v = original.clone();
            v["cases"][0]["file"] = json!("../outside.json");
            v
        },
        {
            let mut v = original.clone();
            v["cases"][0]["sha256"] = json!("wrong");
            v
        },
        {
            let mut v = original.clone();
            v["synthetic"] = json!(false);
            v
        },
    ] {
        fs::write(
            temp.0.join("manifest.json"),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert!(std::panic::catch_unwind(|| replay::load_manifest(&temp.0)).is_err());
    }
}
#[test]
fn process_replay_drains_full_pipes_and_kills_a_stalled_peer() {
    let mut command = Command::new("python3");
    command.args([
        "-c",
        "import sys; sys.stdout.write('x'*131072); sys.stderr.write('y'*131072)",
    ]);
    assert_eq!(
        execute(command, vec![], Duration::from_secs(5)).len(),
        131072
    );
    let before = Instant::now();
    assert!(
        std::panic::catch_unwind(|| {
            let mut stalled = Command::new("python3");
            stalled.args(["-c", "import time; time.sleep(10)"]);
            execute(stalled, vec![], Duration::from_millis(30));
        })
        .is_err()
    );
    assert!(before.elapsed() < Duration::from_secs(3));
}

#[test]
fn provider_callback_boundary_is_strictly_identical_without_losing_inspection() {
    let manifest = replay::load_manifest(&root());
    let mut cases = 0;
    let mut inspected = 0;
    for case in manifest.cases.iter().filter(|c| c.surface != "mcp") {
        let fixture = serde_json::from_slice(&fs::read(root().join(&case.file)).unwrap()).unwrap();
        let (expected, actual, inspection) = replay::provider_callbacks(case, &fixture);
        assert_eq!(expected, actual, "{} callback boundary", case.name);
        // The disjoint native stream must reconstruct every raw observation.
        let (_, raw) = replay::provider(case, &fixture);
        assert_eq!(actual.events, raw.events);
        let mut combined = actual.lifecycle.clone();
        combined.extend(inspection.lifecycle.clone());
        combined.sort_by_key(|m| m.after);
        let mut raw_lifecycle = raw.lifecycle;
        raw_lifecycle.sort_by_key(|m| m.after);
        assert_eq!(combined, raw_lifecycle, "{} inspection lost", case.name);
        cases += 1;
        inspected += inspection.lifecycle.len();
    }
    assert_eq!(cases, 21);
    assert_eq!(inspected, 28);
}
