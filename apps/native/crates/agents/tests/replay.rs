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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownDifference {
    rule: String,
    reason: String,
    differences: Vec<Difference>,
}
fn check_surfaces(root: PathBuf, reference: &str, strict: &str, report: &str) {
    let m = replay::load_manifest(&root);
    assert_eq!(m.baseline, reference);
    let ledger: BTreeMap<String, KnownDifference> =
        serde_json::from_slice(&fs::read(root.join("differences.json")).unwrap()).unwrap();
    let mut reports = vec![];
    for c in &m.cases {
        let fixture: Value =
            serde_json::from_slice(&fs::read(root.join(&c.file)).unwrap()).unwrap();
        assert_eq!(fixture["baseline"], reference, "{} reference", c.name);
        let (expected, actual) = if c.surface == "mcp" {
            mcp(&fixture)
        } else {
            replay::provider(c, &fixture)
        };
        reports.push(replay::report(c, &expected, &actual));
    }
    let serialized = serde_json::to_string_pretty(&reports).unwrap();
    if let Some(path) = std::env::var_os(report) {
        fs::write(path, &serialized).unwrap();
    }
    println!("{serialized}");
    for r in &reports {
        if r.identical {
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
    if std::env::var_os(strict).is_some() {
        assert!(
            reports.iter().all(|r| r.identical),
            "P4 remains open: recorded differences are not an identical replay"
        );
    }
}

#[test]
fn replay_all_surfaces_and_check_every_difference() {
    check_surfaces(
        root().parent().unwrap().join("main/replay"),
        replay::MAIN,
        "PM_REPLAY_REQUIRE_IDENTICAL",
        "PM_REPLAY_REPORT",
    );
}
#[test]
fn legacy_replay_all_surfaces_and_check_every_difference() {
    check_surfaces(
        root(),
        "v1.2.0",
        "PM_REPLAY_REQUIRE_LEGACY_IDENTICAL",
        "PM_LEGACY_REPLAY_REPORT",
    );
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

#[test]
fn pinned_main_provider_callbacks_match_without_replacing_legacy_fixtures() {
    let root = root().parent().unwrap().join("main/replay");
    let manifest = replay::load_manifest(&root);
    assert_eq!(manifest.commit, replay::MAIN);
    let mut count = 0;
    let mut inspection = 0;
    for case in manifest.cases.iter().filter(|c| c.surface != "mcp") {
        let fixture: Value =
            serde_json::from_slice(&fs::read(root.join(&case.file)).unwrap()).unwrap();
        assert_eq!(fixture["baseline"], replay::MAIN, "{} reference", case.name);
        let (expected, actual, native) = replay::provider_callbacks(case, &fixture);
        assert_eq!(expected, actual, "main case {}", case.name);
        count += 1;
        inspection += native.lifecycle.len();
    }
    assert_eq!(count, 21);
    assert_eq!(inspection, 28);
    for source in manifest.sources {
        let mut command = Command::new("git");
        command
            .current_dir(repo())
            .args(["show", &format!("{}:{}", replay::MAIN, source.file)]);
        assert_eq!(
            replay::sha256(&execute(command, vec![], Duration::from_secs(10))),
            source.sha256
        );
    }
    for recorder in [manifest.recorder, manifest.capture]
        .into_iter()
        .chain(manifest.helpers)
    {
        assert_eq!(
            replay::sha256(&fs::read(repo().join(recorder.file)).unwrap()),
            recorder.sha256
        );
    }
}
