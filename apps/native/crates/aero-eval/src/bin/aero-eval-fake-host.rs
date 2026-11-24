//! Test support only: stands in for `powermove-mcp` and for the harness CLIs so
//! the runner is exercised end to end with no app build and no model.
//!   --profile P [--exports E] [--listen]   the headless host (stdio MCP)
//!   --powermove-mcp --profile P            a harness's MCP proxy: tool calls are
//!                                          queued in P/fake-calls.ndjson and the
//!                                          host applies them on its next request
//!   exec …                                 a fake `codex exec --json`
//!   --print …                              a fake `claude --print`
//! Fake edit commands: {"op":"delete","name":…} and
//! {"op":"set","name":…,"path":"/from","value":…}.
use base64::Engine;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    match args.first().map(String::as_str) {
        Some("exec") => fake_cli(&args, Cli::Codex),
        Some("--print") => fake_cli(&args, Cli::Claude),
        _ if args.iter().any(|a| a == "--powermove-mcp") => {
            proxy(Path::new(&value("--profile").expect("--profile")))
        }
        _ => {
            let profile = PathBuf::from(value("--profile").expect("--profile"));
            let exports = value("--exports")
                .map(PathBuf::from)
                .unwrap_or_else(|| profile.join("exports"));
            host(&profile, &exports)
        }
    }
}

fn reply(id: &Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn text(v: Value) -> Value {
    json!({"content": [{"type": "text", "text": v.to_string()}], "isError": false})
}

fn proxy(profile: &Path) {
    let calls = profile.join("fake-calls.ndjson");
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(m) = serde_json::from_str::<Value>(&line.unwrap()) else {
            continue;
        };
        let Some(id) = m.get("id") else { continue };
        let r = match m["method"].as_str() {
            Some("initialize") => {
                json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}})
            }
            Some("tools/call") => {
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&calls)
                    .unwrap();
                writeln!(f, "{}", m["params"]).unwrap();
                text(json!({"status": "applied"}))
            }
            _ => json!({}),
        };
        writeln!(out, "{}", reply(id, r)).unwrap();
        out.flush().unwrap();
    }
}

struct Host {
    profile: PathBuf,
    exports: PathBuf,
    doc: Option<Value>,
    applied: usize,
    saves: usize,
}

impl Host {
    fn apply(&mut self, call: &Value) {
        let Some(doc) = self.doc.as_mut() else { return };
        let layers = doc["proj"]["layers"].as_array_mut().unwrap();
        for c in call["arguments"]["commands"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let name = c["name"].as_str().unwrap_or("");
            match c["op"].as_str() {
                Some("delete") => layers.retain(|l| l["name"] != name),
                Some("set") => {
                    for l in layers.iter_mut().filter(|l| l["name"] == name) {
                        if let Some(x) = l.pointer_mut(c["path"].as_str().unwrap_or("")) {
                            *x = c["value"].clone();
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn catch_up(&mut self) {
        let Ok(s) = std::fs::read_to_string(self.profile.join("fake-calls.ndjson")) else {
            return;
        };
        let calls: Vec<Value> = s
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        for c in calls.iter().skip(self.applied) {
            self.apply(c);
        }
        self.applied = calls.len();
    }
    fn registered(&self) -> Vec<Value> {
        let mut out = vec![json!({"id": "vignette", "parameters": ["amount", "feather"]})];
        for e in std::fs::read_dir(self.profile.join("extensions"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let src = std::fs::read_to_string(e.path().join("index.ts")).unwrap_or_default();
            // A shader that calls the undefined luma() does not compile.
            if src.contains("luma(s.rgb)") {
                continue;
            }
            for part in src.split("id: '").skip(1) {
                let id = part.split('\'').next().unwrap_or("");
                if id.ends_with(".effect") {
                    out.push(json!({"id": id, "parameters": ["amount"]}));
                }
            }
        }
        out
    }
    fn tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        Ok(match name {
            "list_projects" => text(json!({"projects": serde_json::from_str::<Value>(
                &std::fs::read_to_string(self.profile.join("store/projects.json")).map_err(|e| e.to_string())?,
            ).unwrap()})),
            "open_project" => {
                let id = args["projectId"].as_str().unwrap_or("");
                let meta: Value = serde_json::from_str(
                    &std::fs::read_to_string(
                        self.profile.join(format!("store/projectMeta.{id}.json")),
                    )
                    .map_err(|_| format!("no project {id}"))?,
                )
                .unwrap();
                let bytes =
                    std::fs::read(meta["file"].as_str().unwrap()).map_err(|e| e.to_string())?;
                self.doc = Some(pm_aero_eval::pmv::read_document(&bytes)?);
                text(json!({"opened": id}))
            }
            "get_workspace_state" => {
                let exts: Vec<Value> = std::fs::read_dir(self.profile.join("extensions"))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| json!({"id": e.file_name().to_string_lossy(), "enabled": true, "health": "ok"}))
                    .collect();
                text(json!({"extensions": exts, "registeredEffects": self.registered()}))
            }
            "get_panel_layout" => {
                text(json!({"panels": [{"id": "timeline", "title": "Timeline"}]}))
            }
            "render_frames" => {
                let png = pm_aero_eval::image::Rgba::solid(64, 36, [90, 90, 90]).encode_png();
                let data = base64::engine::general_purpose::STANDARD.encode(png);
                let n = args["times"].as_array().map_or(1, Vec::len);
                json!({"content": (0..n).map(|_| json!({"type": "image", "mimeType": "image/png", "data": data})).collect::<Vec<_>>(), "isError": false})
            }
            "save_project" => {
                self.saves += 1;
                let path = self.exports.join(format!("saved-{}.pmv", self.saves));
                std::fs::create_dir_all(&self.exports).unwrap();
                std::fs::write(
                    &path,
                    self.doc.as_ref().ok_or("no project open")?.to_string(),
                )
                .unwrap();
                text(json!({"path": path}))
            }
            other => return Err(format!("Unknown tool: {other}")),
        })
    }
}

fn host(profile: &Path, exports: &Path) {
    let mut h = Host {
        profile: profile.into(),
        exports: exports.into(),
        doc: None,
        applied: 0,
        saves: 0,
    };
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(m) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = m.get("id").cloned() else {
            continue;
        };
        h.catch_up();
        let line = match m["method"].as_str() {
            Some("initialize") => reply(&id, json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}})),
            Some("tools/call") => match h.tool(m["params"]["name"].as_str().unwrap_or(""), &m["params"]["arguments"]) {
                Ok(r) => reply(&id, r),
                Err(e) => reply(&id, json!({"content": [{"type": "text", "text": e}], "isError": true})),
            },
            // The interface lanes A and B implement, faked: one correct cut.
            Some("aero/run") => {
                for e in [json!({"type": "tool-call", "name": "project", "ok": true}),
                          json!({"type": "usage", "input": 700, "cached": 500, "output": 30}),
                          json!({"type": "tool-call", "name": "edit", "ok": true}),
                          json!({"type": "gate-continuation", "failures": ["plan item 1 unchecked"]}),
                          json!({"type": "usage", "input": 800, "cached": 600, "output": 20})] {
                    writeln!(out, "{}", json!({"jsonrpc": "2.0", "method": "aero/event", "params": e})).unwrap();
                }
                h.apply(&json!({"arguments": {"commands": correct_cut()}}));
                reply(&id, json!({"finalText": "Cut done.", "completed": true}))
            }
            _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}}).to_string(),
        };
        writeln!(out, "{line}").unwrap();
        out.flush().unwrap();
    }
}

fn correct_cut() -> Value {
    json!([{"op": "delete", "name": "Take 2"},
           {"op": "set", "name": "B-roll", "path": "/from", "value": 2},
           {"op": "set", "name": "Take 3", "path": "/from", "value": 4},
           {"op": "set", "name": "Take 4", "path": "/from", "value": 6}])
}

enum Cli {
    Codex,
    Claude,
}

/// Behaves by prompt: the flubbed-take prompt gets a correct cut through the
/// MCP proxy; the tighten prompt hangs (for the time limit); anything else
/// answers without editing.
fn fake_cli(args: &[String], cli: Cli) {
    let home = std::env::var("HOME").unwrap_or_default();
    let env: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
    std::fs::write(Path::new(&home).join("env.txt"), env.join("\n")).unwrap();
    std::fs::write(Path::new(&home).join("argv.json"), json!(args).to_string()).unwrap();
    let prompt = args.last().cloned().unwrap_or_default();
    if prompt.contains("Tighten") {
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
    let (command, mcp_args) = match cli {
        Cli::Codex => {
            let c = |key: &str| {
                args.iter()
                    .find_map(|a| a.strip_prefix(&format!("mcp_servers.powermove.{key}=")))
                    .map(|v| serde_json::from_str::<Value>(v).unwrap())
                    .unwrap()
            };
            (c("command"), c("args"))
        }
        Cli::Claude => {
            let i = args.iter().position(|a| a == "--mcp-config").unwrap();
            let cfg: Value = serde_json::from_str(&args[i + 1]).unwrap();
            let p = &cfg["mcpServers"]["powermove"];
            (p["command"].clone(), p["args"].clone())
        }
    };
    let mut calls = 0;
    if prompt.contains("Take 2 is the flubbed one") {
        let mut child = std::process::Command::new(command.as_str().unwrap())
            .args(
                mcp_args
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap()),
            )
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        for (i, m) in [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                   "params": {"name": "apply_commands", "arguments": {"commands": correct_cut()}}}),
        ]
        .iter()
        .enumerate()
        {
            writeln!(stdin, "{m}").unwrap();
            let mut l = String::new();
            stdout.read_line(&mut l).unwrap();
            calls += i;
        }
        drop(stdin);
        child.wait().unwrap();
    }
    let mut out = std::io::stdout().lock();
    match cli {
        Cli::Codex => {
            writeln!(out, r#"{{"type":"thread.started","thread_id":"th_fake"}}"#).unwrap();
            writeln!(out, r#"{{"type":"error","message":"Reconnecting... 1/5"}}"#).unwrap();
            for _ in 0..calls {
                writeln!(out, r#"{{"type":"item.completed","item":{{"type":"mcp_tool_call","server":"powermove","tool":"apply_commands"}}}}"#).unwrap();
            }
            writeln!(
                out,
                r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"Done."}}}}"#
            )
            .unwrap();
            writeln!(out, r#"{{"type":"turn.completed","usage":{{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":50}}}}"#).unwrap();
        }
        Cli::Claude => {
            for i in 0..calls {
                writeln!(out, r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"tu_{i}","name":"mcp__powermove__apply_commands"}}]}}}}"#).unwrap();
            }
            writeln!(out, r#"{{"type":"result","result":"Done.","usage":{{"input_tokens":100,"cache_creation_input_tokens":300,"cache_read_input_tokens":600,"output_tokens":40}}}}"#).unwrap();
        }
    }
}
