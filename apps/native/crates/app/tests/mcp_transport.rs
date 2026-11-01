//! Real app executable, disposable profile and a read-only corpus source.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: mpsc::Receiver<String>,
    next: u64,
}
impl Client {
    fn new(profile: &std::path::Path, project: Option<&std::path::Path>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_powermove-native"));
        command.args(["--powermove-mcp", "--profile"]).arg(profile);
        if let Some(project) = project {
            command.arg(project);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input: Some(input),
            output: rx,
            next: 0,
        }
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        writeln!(
            self.input.as_mut().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params})
        )
        .unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
        let line = self
            .output
            .recv_timeout(Duration::from_secs(30))
            .expect("MCP response timed out");
        let result: Value = serde_json::from_str(&line).expect("stdout must contain JSON-RPC only");
        assert_eq!(result["id"], self.next);
        assert!(result.get("error").is_none(), "{result}");
        result["result"].clone()
    }
    fn tool(&mut self, name: &str, args: Value) -> Value {
        let r = self.rpc("tools/call", json!({"name":name,"arguments":args}));
        assert_eq!(r["isError"], false, "{r}");
        serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[test]
fn app_stdio_conformance_on_a_corpus_copy_commit_and_rollback() {
    let root = tempfile::tempdir().unwrap();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let generated=Command::new("bun").args(["-e", "import { allProjects } from './engine/corpus/src/projects'; const p=allProjects().find(p=>!p.extensions?.length && p.doc.layers?.length); if(!p) process.exit(1); process.stdout.write(JSON.stringify(p.doc));"])
        .current_dir(repo).output().expect("Bun is required for the existing corpus generator");
    assert!(generated.status.success(), "Corpus preparation failed");
    for proxy in [false, true] {
        let case = root.path().join(if proxy { "proxy" } else { "direct" });
        std::fs::create_dir_all(&case).unwrap();
        let path = case.join("project.json");
        std::fs::write(&path, &generated.stdout).unwrap();
        let original = std::fs::read(&path).unwrap();
        let profile = case.join("profile");
        let mut owner = if proxy {
            let mut owner = Client::new(&profile, Some(&path));
            owner.rpc("initialize", json!({}));
            Some(owner)
        } else {
            None
        };
        let mut client = Client::new(&profile, if proxy { None } else { Some(&path) });
        let init=client.rpc("initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
        assert_eq!(init["serverInfo"]["name"], "powermove");
        let tools = client.rpc("tools/list", json!({}));
        assert_eq!(tools["tools"], json!(pm_mcp::catalog::tools()));
        client.tool(
            "start_session",
            json!({"prompt":"Synthetic corpus transport test"}),
        );
        let baseline = client.tool("get_project_state", json!({"indexOnly":true}))["layerCount"]
            .as_u64()
            .unwrap();
        for (commit, id) in [(true, "transport-commit"), (false, "transport-rollback")] {
            if !commit {
                client.tool("start_session", json!({"prompt":"Synthetic rollback test"}));
            }
            client.tool("get_project_state", json!({"indexOnly":true}));
            client.tool("apply_commands",json!({"commands":[{"type":"add_layer","id":id,"layerType":"solid","name":"Transport test"}]}));
            client.tool("finish_session", json!({"commit":commit}));
        }
        client.tool("start_session", json!({"prompt":"Verify committed state"}));
        let state = client.tool(
            "get_project_state",
            json!({"indexOnly":true,"layerLimit":200}),
        );
        assert_eq!(state["layerCount"], baseline + 1);
        assert!(
            state["layers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["id"] == "transport-commit")
        );
        assert!(
            !state["layers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["id"] == "transport-rollback")
        );
        client.tool("finish_session", json!({"commit":true}));
        assert_eq!(
            std::fs::read(path).unwrap(),
            original,
            "MCP commit is an edit, not an implicit file save"
        );
        drop(client.input.take());
        assert!(client.child.wait().unwrap().success());
        if let Some(owner) = owner.as_mut() {
            drop(owner.input.take());
            assert!(owner.child.wait().unwrap().success());
        }
        assert!(!profile.join("mcp/connection.json").exists());
    }
}
