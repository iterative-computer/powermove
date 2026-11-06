mod support;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
fn fixture() -> Value {
    serde_json::from_slice(
        &fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/main/contracts.json"))
            .unwrap(),
    )
    .unwrap()
}
#[test]
fn main_reference_sources_and_recorder_are_pinned() {
    use sha2::{Digest, Sha256};
    let fixture = fixture();
    assert_eq!(
        fixture["reference"],
        "c7f46a05641b23944d3692c9d80f9677c7e1ef1d"
    );
    assert_eq!(fixture["synthetic"], true);
    for source in fixture["sources"].as_array().unwrap() {
        let output = Command::new("git")
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
            format!("{:x}", Sha256::digest(output.stdout)),
            source["sha256"]
        );
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(
                fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("scripts/capture-main-contracts.cjs")
                )
                .unwrap()
            )
        ),
        fixture["captureSha256"]
    );
}
#[test]
fn claude_main_auto_classifier_mode_keeps_handed_back_prompts_and_editor_sandbox() {
    use pm_agents::{Access, ApprovalMode, approvals::Authority, claude};
    for (access, approval, expected) in [
        (Access::Editor, ApprovalMode::Auto, "default"),
        (Access::Project, ApprovalMode::Auto, "auto"),
        (Access::Project, ApprovalMode::Supervised, "acceptEdits"),
    ] {
        let options = claude::Options {
            authority: Authority::new(access, approval, false).unwrap(),
            schema: json!({}),
            model: None,
            effort: None,
            session: None,
            mcp: json!({"mcpServers":{}}),
            extensions: None,
            instructions: String::new(),
            project_tools: vec![],
            editor_tools: vec![],
        };
        let argv = options.argv().unwrap();
        let mode = argv.iter().position(|s| s == "--permission-mode").unwrap();
        assert_eq!(argv[mode + 1], expected);
        assert!(argv.contains(&"--permission-prompt-tool".into()));
        assert!(!argv.contains(&"--dangerously-skip-permissions".into()));
        if access == Access::Project {
            assert!(options.authority.claude_asks());
        }
    }
}

#[test]
fn main_claude_adapter_permission_modes_and_sandbox_settings_match_actual_source() {
    use pm_agents::{Access, ApprovalMode, approvals::Authority, claude};
    for case in fixture()["claude"].as_array().unwrap() {
        let access: Access = serde_json::from_value(case["input"]["access"].clone()).unwrap();
        let approval: ApprovalMode =
            serde_json::from_value(case["input"]["approval"].clone()).unwrap();
        let options = claude::Options {
            authority: Authority::new(access, approval, access == Access::Computer).unwrap(),
            schema: json!({}),
            model: None,
            effort: None,
            session: None,
            mcp: json!({"mcpServers":{}}),
            extensions: None,
            instructions: String::new(),
            project_tools: vec![],
            editor_tools: vec![],
        };
        let argv = options.argv().unwrap();
        let flag = |name: &str| {
            argv.iter()
                .position(|s| s == name)
                .map(|i| argv[i + 1].clone())
        };
        let settings = flag("--settings").map(|s| serde_json::from_str::<Value>(&s).unwrap());
        assert_eq!(
            json!({"mode":flag("--permission-mode"),"settings":settings,"allowed":flag("--allowedTools"),"disallowed":flag("--disallowedTools"),"skip":argv.contains(&"--dangerously-skip-permissions".into())}),
            case["output"]
        );
    }
}
