mod support;
use pm_agents::{
    Access, ApprovalMode, Provider,
    launch::{Environment, Request},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    thread,
    time::{Duration, Instant},
};
use support::Temp;

#[test]
fn fixture_provider_launches_receive_user_servers_without_starting_them() {
    for (provider, access) in [
        (Provider::Claude, Access::Editor),
        (Provider::Claude, Access::Project),
        (Provider::Claude, Access::Computer),
        (Provider::Chatgpt, Access::Editor),
        (Provider::Chatgpt, Access::Project),
        (Provider::Chatgpt, Access::Computer),
    ] {
        let t = Temp::new();
        let fake = t.0.join("runtime");
        let capture = t.0.join("capture.json");
        fs::write(
            &fake,
            r##"#!/usr/bin/python3
import json, os, sys
args=sys.argv[1:]
path=os.environ['CAPTURE']
if 'app-server' not in args:
    with open(path,'w') as f: json.dump(args,f)
    for line in sys.stdin:
        pass
    sys.exit(0)
for line in sys.stdin:
    msg=json.loads(line)
    if 'app-server' in args and msg.get('method')=='thread/start':
        with open(path,'w') as f: json.dump(msg['params'],f)
    if 'id' in msg:
        result={'thread':{'id':'fixture-thread'}} if msg.get('method')=='thread/start' else {}
        print(json.dumps({'id':msg['id'],'result':result}),flush=True)
"##,
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(t.0.join(".claude.json"),json!({"mcpServers":{"fixture":{"command":"fixture-only-never-execute","env":{"MODE":"test"}},"powermove":{"command":"wrong"}}}).to_string()).unwrap();
        fs::write(t.0.join("config.toml"),"[mcp_servers.fixture]\ncommand='fixture-only-never-execute'\nrequired=true\n[mcp_servers.powermove]\ncommand='wrong'\n").unwrap();
        let env = Environment {
            profile: t.0.join("profile"),
            executable: "/fixture/powermove".into(),
            vars: BTreeMap::from([
                ("HOME".into(), t.0.display().to_string()),
                ("CODEX_HOME".into(), t.0.display().to_string()),
                ("CLAUDE_CONFIG_DIR".into(), t.0.display().to_string()),
                ("CLAUDE_BINARY".into(), fake.display().to_string()),
                ("CODEX_BINARY".into(), fake.display().to_string()),
                ("CAPTURE".into(), capture.display().to_string()),
            ]),
            tools: vec!["start_session".into()],
            composition_tools: vec!["get_project_state".into()],
            ..Default::default()
        };
        let h = pm_agents::launch::launch(
            &env,
            Request {
                run: "fixture-run".into(),
                provider,
                access,
                approval: ApprovalMode::Auto,
                computer_consent: access == Access::Computer,
                workspace: t.0.clone(),
                prompt: "Fixture request".into(),
                model: None,
                effort: None,
                images: vec![],
                files: vec![],
                role: None,
                project: None,
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let value: Value = loop {
            if let Ok(s) = fs::read_to_string(&capture) {
                if let Ok(v) = serde_json::from_str(&s) {
                    break v;
                }
            }
            assert!(
                Instant::now() < deadline,
                "{provider:?}/{access:?}: no launch capture"
            );
            thread::sleep(Duration::from_millis(5));
        };
        if provider == Provider::Claude {
            let args = value.as_array().unwrap();
            let pos = args.iter().position(|v| v == "--mcp-config").unwrap();
            let config: Value = serde_json::from_str(args[pos + 1].as_str().unwrap()).unwrap();
            if access == Access::Editor {
                assert!(config["mcpServers"].get("fixture").is_none());
            } else {
                assert_eq!(
                    config["mcpServers"]["fixture"]["command"],
                    "fixture-only-never-execute"
                );
            }
            assert_eq!(
                config["mcpServers"]["powermove"]["command"],
                "/fixture/powermove"
            );
            if access != Access::Computer {
                assert_eq!(
                    args.iter()
                        .any(|v| v.as_str().is_some_and(|s| s.contains("mcp__fixture__*"))),
                    access == Access::Project
                );
            }
        } else if access == Access::Editor {
            assert_eq!(value["config"]["mcp_servers"]["fixture"]["required"], false);
            assert_eq!(
                value["config"]["mcp_servers"]["powermove"]["required"],
                true
            );
        } else {
            let args = value.as_array().unwrap();
            let pair = args
                .iter()
                .filter_map(Value::as_str)
                .find(|s| s.starts_with("mcp_servers.\"fixture\"="))
                .unwrap();
            let doc: toml_edit::DocumentMut = pair.parse().unwrap();
            assert_eq!(
                doc["mcp_servers"]["fixture"]["required"].as_bool(),
                Some(false)
            );
        }
        drop(h);
    }
}
