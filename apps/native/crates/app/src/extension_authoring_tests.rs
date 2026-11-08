use super::*;
use pm_mcp::extensions::{ExtensionHost, fork};
use serde_json::json;
use std::fs;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(pm_kernel::new_id(name));
    fs::create_dir_all(&dir).unwrap();
    dir
}
fn write(dir: &Path, manifest: Value, files: &[(&str, &str)]) -> Value {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    for (name, text) in files {
        fs::write(dir.join(name), text).unwrap();
    }
    manifest::parse(&manifest).unwrap()
}

#[test]
fn the_shipped_sample_and_a_v3_extension_pass_and_broken_code_reports_why() {
    let root = scratch("pm-authoring-check");
    let sample = PACK
        .iter()
        .filter_map(|(n, t)| n.strip_prefix("samples/hello-panel/").map(|n| (n, *t)))
        .collect::<Vec<_>>();
    let raw: Value = serde_json::from_str(sample.iter().find(|(n, _)| *n == "manifest.json").unwrap().1).unwrap();
    let parsed = write(&root.join("hello-panel"), raw, &sample);
    check(&root.join("hello-panel"), &parsed).unwrap();

    let v3 = write(
        &root.join("v3-command"),
        json!({"id":"v3-command","name":"V3","version":"1.0.0","apiVersion":3,"entry":"index.ts"}),
        &[("index.ts", "export default function activate() {}")],
    );
    check(&root.join("v3-command"), &v3).unwrap();

    let broken = write(
        &root.join("broken"),
        json!({"id":"broken","name":"Broken","version":"1.0.0","apiVersion":4,"entry":"index.tsx"}),
        &[("index.tsx", "export const nothing = 1;")],
    );
    let error = check(&root.join("broken"), &broken).unwrap_err();
    assert!(error.contains("exports view() or mount(env)"), "{error}");
    fs::write(root.join("broken/index.tsx"), "export function view( {").unwrap();
    let error = check(&root.join("broken"), &broken).unwrap_err();
    assert!(error.contains("TypeScript parse"), "{error}");
    fs::write(root.join("broken/index.tsx"), "import { readFileSync } from 'node:fs'; export const view = () => String(readFileSync);").unwrap();
    assert!(check(&root.join("broken"), &broken).unwrap_err().contains("node:fs"));
    let _ = fs::remove_dir_all(root);
}

/// Every built-in panel ships its source; a fork of each compiles with the
/// native compiler and draws its panel in a real panel isolate.
#[test]
fn every_built_in_forks_compiles_and_mounts_in_place_of_the_built_in() {
    let profile = scratch("pm-authoring-forks");
    let host = AppExtensions::new(profile.clone());
    let stage = profile.join("stage");
    let builtins = host.builtins().unwrap();
    for panel in LIVE_PANELS {
        assert!(builtins.join(panel).join("manifest.json").exists(), "{panel}");
        let forked = fork(&host, panel, None, &stage).unwrap();
        let dir = PathBuf::from(forked["dir"].as_str().unwrap());
        let raw: Value = serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let parsed = host.manifest(&raw).unwrap();
        assert_eq!(replaced_panel(&parsed), Some(*panel));
        check(&dir, &parsed).unwrap_or_else(|e| panic!("{panel} fork: {e}"));
    }
    // The copy is content-addressed: a second call reuses it.
    assert_eq!(host.builtins().unwrap(), builtins);
    let _ = fs::remove_dir_all(profile);
}
