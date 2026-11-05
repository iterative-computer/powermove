use pm_agents::{Access, ApprovalMode, approvals::Authority, claude, claude_launch::Options};
use serde_json::Value;
#[test]
fn all_native_claude_permission_profiles_match_executed_main_arguments() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/main-claude-launch.json")).unwrap();
    assert_eq!(
        fixture["reference"],
        "c7f46a05641b23944d3692c9d80f9677c7e1ef1d"
    );
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    for case in cases {
        let options: Options = serde_json::from_value(case["options"].clone()).unwrap();
        let access: Access = serde_json::from_value(case["access"].clone()).unwrap();
        let approval: ApprovalMode = serde_json::from_value(case["approval"].clone()).unwrap();
        let authority = Authority::new(access, approval, access == Access::Computer).unwrap();
        let actual = options.argv(&authority).unwrap();
        let expected: Vec<String> = serde_json::from_value(case["argv"].clone()).unwrap();
        // These CLI values are parsed as JSON; property order has no meaning.
        // All flag/value positions and other bytes are compared exactly.
        assert_eq!(actual.len(), expected.len());
        for (i, (a, b)) in actual.iter().zip(&expected).enumerate() {
            if i > 0
                && ["--mcp-config", "--json-schema", "--settings"].contains(&actual[i - 1].as_str())
            {
                assert_eq!(
                    serde_json::from_str::<Value>(a).unwrap(),
                    serde_json::from_str::<Value>(b).unwrap()
                );
            } else {
                assert_eq!(a, b, "{access:?}/{approval:?} argument {i}");
            }
        }
        assert_eq!(
            claude::user_message(&options.prompt, &options.image_paths),
            case["message"]
        );
    }
}
#[test]
fn native_launch_requires_schema_instructions_and_valid_mcp_maps() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/main-claude-launch.json")).unwrap();
    let options: Options = serde_json::from_value(fixture["cases"][0]["options"].clone()).unwrap();
    let authority = Authority::new(Access::Project, ApprovalMode::Auto, false).unwrap();
    for invalid in 0..4 {
        let mut changed = options.clone();
        match invalid {
            0 => changed.schema = Value::Null,
            1 => changed.instructions.clear(),
            2 => changed.external_mcp_servers = Value::Null,
            _ => changed.native_tools = Some(Value::Null),
        }
        assert!(changed.argv(&authority).is_err());
    }
}

#[test]
fn current_main_source_and_launch_recorder_hashes_are_pinned() {
    use sha2::{Digest, Sha256};
    use std::{fs, path::Path, process::Command};
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/main-claude-launch.json")).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .unwrap();
    for source in fixture["sources"].as_array().unwrap() {
        let output = Command::new("git")
            .current_dir(root)
            .args([
                "show",
                &format!(
                    "{}:{}",
                    fixture["reference"].as_str().unwrap(),
                    source["file"].as_str().unwrap()
                ),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            format!("{:x}", Sha256::digest(&output.stdout)),
            source["sha256"].as_str().unwrap()
        );
    }
    let recorder = &fixture["recorder"];
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(fs::read(root.join(recorder["file"].as_str().unwrap())).unwrap())
        ),
        recorder["sha256"].as_str().unwrap()
    );
}
