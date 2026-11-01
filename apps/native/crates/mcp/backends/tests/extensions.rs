use pm_mcp::{Automation, McpSession, extensions::ExtensionSources};
use pm_mcp_backends::extensions::{BuiltinSources, validate_manifest};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "pm-native-fork-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn source(&self) -> PathBuf {
        let resources = self.0.join("resources");
        let source = resources.join("synthetic");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(
            source.join("manifest.json"),
            serde_json::to_string_pretty(&manifest()).unwrap(),
        )
        .unwrap();
        fs::write(source.join("index.ts"), "export const synthetic = 1;\n").unwrap();
        fs::write(source.join("src/helper.ts"), "export const helper = 2;\n").unwrap();
        fs::create_dir(source.join(".hidden")).unwrap();
        fs::write(source.join(".hidden/keep"), "source only").unwrap();
        fs::write(source.join(".DS_Store"), "skip").unwrap();
        fs::write(source.join("unit.test.ts"), "skip").unwrap();
        resources
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn manifest() -> Value {
    json!({"id":"synthetic","name":"Synthetic","version":"1.2.3","apiVersion":3,"contributes":["effects"],"unknown":{"retained":true}})
}
fn oracle(v: Value) -> Value {
    let mut c = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/fork-pairs.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin
        .take()
        .unwrap()
        .write_all(v.to_string().as_bytes())
        .unwrap();
    let out = c.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn fork_matches_actual_tagged_file_body_and_retains_pristine_tree() {
    let f = Fixture::new();
    let resources = f.source();
    let host = BuiltinSources::new(resources.clone()).unwrap();
    let result = host
        .fork_builtin("synthetic", None, &f.0.join("native"))
        .unwrap();
    let expected = oracle(
        json!({"operation":"fork","options":{"resourcesDir":resources,"id":"synthetic","targetDir":f.0.join("oracle")}}),
    );
    assert_eq!(result.fork_id, expected["result"]["forkId"]);
    assert_eq!(result.forked_from, expected["result"]["forkedFrom"]);
    assert_eq!(json!(result.files), expected["result"]["files"]);
    for file in &result.files {
        for prefix in ["", ".forked-from/"] {
            let relative = format!("{prefix}{file}");
            let native = fs::read(result.dir.join(&relative)).unwrap();
            let reference = fs::read(f.0.join("oracle/synthetic-fork").join(relative)).unwrap();
            if file == "manifest.json" {
                assert_eq!(
                    serde_json::from_slice::<Value>(&native).unwrap(),
                    serde_json::from_slice::<Value>(&reference).unwrap()
                );
            } else {
                assert_eq!(native, reference);
            }
        }
    }
    assert!(!result.dir.join(".hidden").exists());
    assert!(!result.dir.join("unit.test.ts").exists());
    assert_eq!(
        fs::read(resources.join("synthetic/.hidden/keep")).unwrap(),
        b"source only"
    );
    assert!(
        host.fork_builtin("synthetic", None, &f.0.join("native"))
            .is_err()
    );
}
#[test]
fn manifest_admission_matches_frozen_v1_to_v3_parser() {
    let mut cases = vec![manifest(), json!(null), json!([])];
    for (key, value) in [
        ("id", json!("x")),
        ("apiVersion", json!(0)),
        ("apiVersion", json!(1.5)),
        ("apiVersion", json!(4)),
        ("apiVersion", json!(1.0)),
        ("name", json!("")),
        ("name", json!("😀".repeat(41))),
        ("entry", json!("../index.ts")),
        ("entry", json!("src/index.mjs")),
        ("entry", json!("C:\\file.ts")),
        ("version", json!("1.2")),
        ("replaces", json!(["synthetic"])),
        ("contributes", json!(["bad"])),
        ("features", json!(null)),
        ("permissions", json!(["network", "network"])),
        ("permissions", json!(["project:write", "full-access"])),
        ("links", json!(["https://example.com"])),
        ("links", json!(["https://localhost"])),
        ("links", json!(["https://example.com/"])),
        ("links", json!(["https://EXAMPLE.com"])),
        ("links", json!(["https://127.0.0.1"])),
        ("links", json!(["https://example.com:443"])),
        ("links", json!(["https://xn--bcher-kva.example:8443"])),
        ("vars", json!([{"key":"KEY","label":"Key","secret":true}])),
        (
            "vars",
            json!([{"key":"KEY","label":"One"},{"key":"KEY","label":"Two"}]),
        ),
        ("author", json!("other")),
    ] {
        let mut m = manifest();
        m[key] = value;
        cases.push(m);
    }
    for api in [1, 2] {
        let mut m = manifest();
        m["apiVersion"] = json!(api);
        m["permissions"] = json!([]);
        cases.push(m);
        let mut m = manifest();
        m["apiVersion"] = json!(api);
        m["vars"] = json!([]);
        cases.push(m);
    }
    for m in cases {
        assert_eq!(
            validate_manifest(&m).is_ok(),
            oracle(json!({"operation":"manifest","manifest":m}))["valid"]
                .as_bool()
                .unwrap(),
            "{m}"
        );
    }
}
#[test]
fn fork_refuses_aliases_invalid_manifests_and_authored_target_overwrites() {
    let f = Fixture::new();
    let resources = f.source();
    let host = BuiltinSources::new(resources.clone()).unwrap();
    for id in ["../synthetic", "synthetic/child", "x"] {
        assert!(host.fork_builtin(id, None, &f.0.join("staging")).is_err());
    }
    assert!(
        host.fork_builtin("synthetic", Some("synthetic"), &f.0.join("staging"))
            .is_err()
    );
    fs::create_dir_all(f.0.join("staging/synthetic-fork")).unwrap();
    fs::write(f.0.join("staging/synthetic-fork/authored.ts"), "retain").unwrap();
    assert!(
        host.fork_builtin("synthetic", None, &f.0.join("staging"))
            .is_err()
    );
    assert_eq!(
        fs::read(f.0.join("staging/synthetic-fork/authored.ts")).unwrap(),
        b"retain"
    );
    fs::write(resources.join("synthetic/manifest.json"), "{}").unwrap();
    assert!(
        host.fork_builtin("synthetic", Some("another-fork"), &f.0.join("staging"))
            .is_err()
    );
    assert!(!f.0.join("staging/another-fork").exists());
}
#[cfg(unix)]
#[test]
fn failed_fork_retains_partial_sources_without_publishing_manifest() {
    let f = Fixture::new();
    let resources = f.source();
    std::os::unix::fs::symlink(
        "/synthetic/not-readable",
        resources.join("synthetic/zzz-link"),
    )
    .unwrap();
    let host = BuiltinSources::new(resources).unwrap();
    assert!(
        host.fork_builtin("synthetic", None, &f.0.join("staging"))
            .is_err()
    );
    assert_eq!(
        fs::read(f.0.join("staging/synthetic-fork/index.ts")).unwrap(),
        b"export const synthetic = 1;\n"
    );
    assert!(!f.0.join("staging/synthetic-fork/manifest.json").exists());
}
fn ok(v: Value) -> Value {
    assert_eq!(v["isError"], false, "{v}");
    serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
}
#[test]
fn session_fork_is_inert_and_finish_archives_the_whole_authored_tree() {
    for commit in [false, true] {
        let f = Fixture::new();
        let resources = f.source();
        let mut host = Automation::new(f.0.join("profile"), f.0.join("exports")).unwrap();
        host.create_project(&json!({"name":"Synthetic"})).unwrap();
        host.extension_sources = Some(Arc::new(BuiltinSources::new(resources).unwrap()));
        let host = Arc::new(Mutex::new(host));
        let mut session = McpSession::new(host.clone());
        let start = ok(session.call(
            "start_session",
            json!({"prompt":"Synthetic","context":if commit {"app"} else {"project"}}),
        ));
        let stage = PathBuf::from(start["stagingDirectory"].as_str().unwrap());
        let result = ok(session.call("fork_builtin_extension", json!({"id":"synthetic"})));
        assert_eq!(
            PathBuf::from(result["dir"].as_str().unwrap()),
            stage.join("synthetic-fork")
        );
        assert!(
            host.lock()
                .unwrap()
                .kernel()
                .state()
                .registries
                .values()
                .all(|r| r.is_empty())
        );
        ok(session.call("finish_session", json!({"commit":commit})));
        let archives = f.0.join("profile/mcp-archives");
        let archive = fs::read_dir(archives)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(
            archive
                .join(if commit {
                    "extensions/synthetic-fork/index.ts"
                } else {
                    "workspace/extensions/synthetic-fork/index.ts"
                })
                .is_file()
        );
        assert!(
            archive
                .join(if commit {
                    "extensions/synthetic-fork/.forked-from/manifest.json"
                } else {
                    "workspace/extensions/synthetic-fork/.forked-from/manifest.json"
                })
                .is_file()
        );
    }
}
