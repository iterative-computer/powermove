//! The shared host-services contract sequence (contract 14 invariant 9),
//! also run against Electron main's implementation
//! (apps/desktop/src/main/engine/host-contract.test.ts).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pm_host::arbiter::MemoryArbiter;
use pm_host::{CliHost, CliHostConfig};
use pm_protocol::common::SessionId;
use pm_protocol::envelope::Request;
use pm_protocol::error::EngineProcess;
use pm_protocol::host::PortId;
use serde_json::Value;

fn substitute(value: &Value, vars: &HashMap<String, Value>) -> Value {
    match value {
        Value::String(s) if s.starts_with('$') => vars
            .get(&s[1..])
            .unwrap_or_else(|| panic!("unknown variable {s}"))
            .clone(),
        Value::Array(items) => Value::Array(items.iter().map(|v| substitute(v, vars)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), substitute(v, vars)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn process(value: &Value) -> EngineProcess {
    serde_json::from_value(value.clone()).expect("process")
}

fn relative(root: &Path, path: &str) -> String {
    Path::new(path)
        .strip_prefix(root)
        .expect("under the test root")
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

/// A minimal regex for the fixture's one pattern: `^Powermove/\.powermove-[0-9a-f]{16}\.part$`.
fn matches_temp_pattern(pattern: &str, text: &str) -> bool {
    assert_eq!(pattern, r"^Powermove/\.powermove-[0-9a-f]{16}\.part$");
    text.strip_prefix("Powermove/.powermove-")
        .and_then(|rest| rest.strip_suffix(".part"))
        .is_some_and(|hex| {
            hex.len() == 16
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

#[test]
fn the_cli_host_serves_the_shared_sequence() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/host-contract.json")).unwrap();
    let root: PathBuf =
        std::env::temp_dir().join(format!("pm-host-contract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut host = CliHost::new(CliHostConfig {
        cache_directory: root.join("cache"),
        automatic_output_directory: root.join("Powermove"),
        physical_memory: 0,
    });
    *host.memory_mut() = MemoryArbiter::new(10_000);
    let mut vars: HashMap<String, Value> = HashMap::new();

    for (index, raw) in fixture["steps"].as_array().unwrap().iter().enumerate() {
        let step = substitute(raw, &vars);
        let op = step["op"].as_str().unwrap();
        let place = format!("step {index} ({op} {})", step["request"]["type"]);
        match op {
            "request" => {
                let request: Request = serde_json::from_value(step["request"].clone())
                    .unwrap_or_else(|e| panic!("{place}: {e}"));
                let session = step["session"].as_u64().map(|s| SessionId::new(s).unwrap());
                let port = PortId(step["port"].as_u64().unwrap() as u32);
                let result = host.serve(process(&step["process"]), port, &request, session);
                let expect = &step["expect"];
                if let Some(code) = expect["error"].as_str() {
                    let error = result.expect_err(&place);
                    assert_eq!(
                        serde_json::to_value(error.code).unwrap(),
                        code,
                        "{place}: {}",
                        error.message
                    );
                    continue;
                }
                let response = serde_json::to_value(
                    result.unwrap_or_else(|e| panic!("{place}: {:?} {}", e.code, e.message)),
                )
                .unwrap();
                assert_eq!(response["type"], expect["response"], "{place}");
                if let Some(fields) = expect["fields"].as_object() {
                    for (field, value) in fields {
                        assert_eq!(&response[field], value, "{place} {field}");
                    }
                }
                if let Some(suffix) = expect["pathEndsWith"].as_str() {
                    let path = relative(&root, response["path"].as_str().unwrap());
                    assert!(path.ends_with(suffix), "{place}: {path}");
                }
                if let Some(pattern) = expect["pathMatches"].as_str() {
                    let path = relative(&root, response["path"].as_str().unwrap());
                    assert!(matches_temp_pattern(pattern, &path), "{place}: {path}");
                }
                if let Some(mode) = expect["mode"].as_u64() {
                    use std::os::unix::fs::PermissionsExt;
                    let meta = std::fs::metadata(response["path"].as_str().unwrap()).unwrap();
                    assert_eq!(
                        u64::from(meta.permissions().mode() & 0o777),
                        mode,
                        "{place}"
                    );
                }
                if let Some(saves) = step["save"].as_object() {
                    for (name, field) in saves {
                        vars.insert(name.clone(), response[field.as_str().unwrap()].clone());
                    }
                }
            }
            "write" => {
                let bytes = vec![1u8; step["bytes"].as_u64().unwrap() as usize];
                std::fs::write(step["path"].as_str().unwrap(), bytes).unwrap();
            }
            "exists" => {
                let exists = Path::new(step["path"].as_str().unwrap()).exists();
                assert_eq!(Value::Bool(exists), step["expect"], "{place}");
            }
            "initialLeases" => {
                host.memory_mut().initial(EngineProcess::Interactive);
                host.memory_mut().initial(EngineProcess::Job);
            }
            "endLaunch" => {
                let removed = host.end_launch(
                    process(&step["process"]),
                    PortId(step["port"].as_u64().unwrap() as u32),
                );
                assert_eq!(Value::from(removed), step["expectRemoved"], "{place}");
            }
            "listOutputs" => {
                let mut names: Vec<String> = std::fs::read_dir(root.join("Powermove"))
                    .unwrap()
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                assert_eq!(
                    serde_json::to_value(names).unwrap(),
                    step["expect"],
                    "{place}"
                );
            }
            other => panic!("unknown op {other}"),
        }
    }
    assert_eq!(host.pending_outputs(), 0);
    std::fs::remove_dir_all(&root).unwrap();
}
