//! Test support only: stands in for `powermove-mcp` and for the harness CLIs so
//! the runner is exercised end to end with no app build and no model.
//!   --profile P [--exports E] [--listen]   the headless host (stdio MCP)
//!   --powermove-mcp --profile P            a harness's MCP proxy: tool calls are
//!                                          queued in P/fake-calls.ndjson and the
//!                                          host applies them on its next request
//!   exec …                                 a fake `codex exec --json`
//!   --print …                              a fake `claude --print`
//!   probe [--read P] [--connect ADDR] [--env NAME]
//!                                          try each and print what happened
//! The fake CLIs and the fake Aero talk to the model endpoint the runner gives
//! them (a replay on loopback): each turn posts {model, prompt, turn} and the
//! streamed reply carries `fake.commands` (edits to apply through the host)
//! and `fake.done` (final text and usage). Markers in the prompt pick test
//! behaviours; see `fake_cli` and `aero_run`.
//! Fake edit commands: {"op":"delete","name":…} and
//! {"op":"set","name":…,"path":"/from","value":…}.
use base64::Engine;
use pm_aero_eval::replay::{http, sse};
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
        Some("probe") => println!(
            "{}",
            probe(
                value("--read").as_deref(),
                value("--connect").as_deref(),
                value("--env").as_deref()
            )
        ),
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

/// Try a read, a TCP connection and an environment variable; report each.
fn probe(read: Option<&str>, connect: Option<&str>, env: Option<&str>) -> Value {
    let mut out = json!({});
    if let Some(p) = read {
        out["read"] = json!(match std::fs::read(p) {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("refused: {e}"),
        });
    }
    if let Some(a) = connect {
        out["connect"] = json!(match a.parse::<std::net::SocketAddr>() {
            Err(e) => format!("bad address: {e}"),
            Ok(addr) =>
                match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(3))
                {
                    Ok(_) => "ok".to_string(),
                    Err(e) => format!("refused: {e}"),
                },
        });
    }
    if let Some(n) = env {
        out["env"] = json!(std::env::var(n).ok());
    }
    out
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
    project_id: Option<String>,
    applied: usize,
    saves: usize,
    /// A hung `aero/run` (id, run id) waiting for `aero/cancel`.
    pending: Option<(Value, String)>,
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
    fn extensions(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(self.profile.join("extensions"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        v.sort();
        v
    }
    fn registered(&self) -> Vec<Value> {
        let mut out = vec![
            json!({"id": "vignette", "parameters": ["amount", "feather"], "extension": "builtin"}),
        ];
        for e in self.extensions() {
            let src = std::fs::read_to_string(e.join("index.ts")).unwrap_or_default();
            // A shader that calls the undefined luma() does not compile.
            if src.contains("luma(s.rgb)") {
                continue;
            }
            let ext = e.file_name().unwrap().to_string_lossy().into_owned();
            for part in src.split("id: '").skip(1) {
                let id = part.split('\'').next().unwrap_or("");
                if id.ends_with(".effect") {
                    out.push(json!({"id": id, "parameters": ["amount"], "extension": ext}));
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
                self.project_id = Some(id.into());
                text(json!({"opened": id}))
            }
            "get_workspace_state" => {
                let exts: Vec<Value> = self
                    .extensions()
                    .iter()
                    .map(|e| json!({"id": e.file_name().unwrap().to_string_lossy(), "enabled": true, "health": "ok"}))
                    .collect();
                text(
                    json!({"extensions": exts, "registeredEffects": self.registered(),
                    "registeredTransitions": [], "registeredPanels": []}),
                )
            }
            "render_frames" => {
                let times: Vec<f64> = args["times"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_f64)
                    .collect();
                let content: Vec<Value> = times
                    .iter()
                    .map(|&t| {
                        let png = pm_aero_eval::image::Rgba::solid(64, 36, self.colour_at(t))
                            .encode_png();
                        json!({"type": "image", "mimeType": "image/png",
                               "data": base64::engine::general_purpose::STANDARD.encode(png)})
                    })
                    .collect();
                json!({"content": content, "isError": false})
            }
            "save_project" => {
                self.saves += 1;
                let doc = self.doc.as_ref().ok_or("no project open")?.to_string();
                // A host that names a file outside the run (or a link to one).
                if let Some(target) = self.marker("[saved-outside ") {
                    return Ok(text(json!({"path": target})));
                }
                std::fs::create_dir_all(&self.exports).unwrap();
                let path = self.exports.join(format!("saved-{}.pmv", self.saves));
                if self.has_marker("[saved-symlink]") {
                    let real = self.exports.join("real.pmv");
                    std::fs::write(&real, &doc).unwrap();
                    std::os::unix::fs::symlink(&real, &path).unwrap();
                } else {
                    std::fs::write(&path, doc).unwrap();
                }
                text(json!({"path": path}))
            }
            other => return Err(format!("Unknown tool: {other}")),
        })
    }
    /// A flat frame in the colour of the topmost enabled clip at `t` (each
    /// take's media is one colour), grey when none plays.
    fn colour_at(&self, t: f64) -> [u8; 3] {
        let layers = self
            .doc
            .as_ref()
            .and_then(|d| d["proj"]["layers"].as_array().cloned())
            .unwrap_or_default();
        layers
            .iter()
            .filter(|l| l["on"] != false && l["type"] == "video")
            .filter(|l| {
                let from = l["from"].as_f64().unwrap_or(0.0);
                t >= from && t < from + l["dur"].as_f64().unwrap_or(0.0)
            })
            .find_map(|l| match l["d"]["asset"].as_str()? {
                "take-1" => Some([198, 59, 59]),
                "take-2" => Some([59, 160, 60]),
                "take-3" => Some([59, 89, 200]),
                "broll" => Some([210, 180, 40]),
                "take-4" => Some([150, 58, 169]),
                _ => None,
            })
            .unwrap_or([90, 90, 90])
    }
    fn prompt(&self) -> String {
        std::fs::read_to_string(self.profile.join("fake-prompt.txt")).unwrap_or_default()
    }
    fn has_marker(&self, m: &str) -> bool {
        self.prompt().contains(m)
    }
    /// The text after a marker up to `]`.
    fn marker(&self, m: &str) -> Option<String> {
        let p = self.prompt();
        let rest = &p[p.find(m)? + m.len()..];
        Some(rest[..rest.find(']')?].to_owned())
    }
}

const TOOLS: [&str; 5] = [
    "list_projects",
    "open_project",
    "get_workspace_state",
    "render_frames",
    "save_project",
];

fn host(profile: &Path, exports: &Path) {
    let mut h = Host {
        profile: profile.into(),
        exports: exports.into(),
        doc: None,
        project_id: None,
        applied: 0,
        saves: 0,
        pending: None,
    };
    let env: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
    std::fs::write(profile.join("host-env.txt"), env.join("\n")).unwrap();
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(m) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if m["method"] == "aero/cancel" {
            if let Some((id, run)) = h.pending.take() {
                let r = json!({"runId": run, "finalText": "", "completed": false, "reason": "cancelled"});
                writeln!(out, "{}", reply(&id, r)).unwrap();
                out.flush().unwrap();
            }
            continue;
        }
        let Some(id) = m.get("id").cloned() else {
            continue;
        };
        h.catch_up();
        let line = match m["method"].as_str() {
            Some("initialize") => reply(&id, json!({"protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}, "experimental": {"powermoveEval": {
                    "extensionHealth": true, "registrationOwners": true, "transitions": true,
                    "panels": true, "themes": true}}}})),
            Some("tools/list") => reply(&id, json!({"tools": TOOLS.iter().map(|n| json!({"name": n})).collect::<Vec<_>>()})),
            Some("tools/call") => match h.tool(m["params"]["name"].as_str().unwrap_or(""), &m["params"]["arguments"]) {
                Ok(r) => reply(&id, r),
                Err(e) => reply(&id, json!({"content": [{"type": "text", "text": e}], "isError": true})),
            },
            Some("aero/run") => match aero_run(&mut h, &m["params"], &mut out) {
                Some(r) => reply(&id, r),
                None => {
                    h.pending = Some((id, m["params"]["runId"].as_str().unwrap_or("").into()));
                    continue;
                }
            },
            _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}}).to_string(),
        };
        writeln!(out, "{line}").unwrap();
        out.flush().unwrap();
    }
}

/// The interface lanes A and B implement, faked against the replay endpoint.
/// Markers: `[aero-hang]` answers only on `aero/cancel`; `[aero-steps N]`
/// reports N tool calls first; `[aero-gate-fails]` ends with
/// `completed:false`; `[aero-skip-replay]` edits without the model.
fn aero_run(h: &mut Host, p: &Value, out: &mut impl Write) -> Option<Value> {
    let run = p["runId"].as_str().unwrap_or("").to_owned();
    let prompt = p["prompt"].as_str().unwrap_or("").to_owned();
    std::fs::write(h.profile.join("fake-prompt.txt"), &prompt).unwrap();
    let answer = |completed: bool, reason: &str, text: &str| {
        Some(json!({"runId": run, "finalText": text, "completed": completed, "reason": reason}))
    };
    if p["projectId"].as_str() != h.project_id.as_deref()
        || p["revision"]
            != h.doc
                .as_ref()
                .map_or(Value::Null, |d| d["proj"]["revision"].clone())
    {
        return answer(false, "error", "the run is not bound to the open project");
    }
    let mut seq = 0u64;
    let mut emit = |e: Value| {
        writeln!(out, "{}", json!({"jsonrpc": "2.0", "method": "aero/event", "params": {"runId": run, "seq": seq, "event": e}})).unwrap();
        out.flush().unwrap();
        seq += 1;
    };
    if let Some(n) = h.marker("[aero-steps ") {
        for _ in 0..n.parse::<u32>().unwrap() {
            emit(json!({"type": "tool-call", "name": "project", "ok": true}));
        }
        return None;
    }
    if h.has_marker("[aero-hang]") {
        return None;
    }
    if h.has_marker("[aero-skip-replay]") {
        h.apply(&json!({"arguments": {"commands": correct_cut()}}));
        emit(json!({"type": "tool-call", "name": "edit", "ok": true}));
        return answer(true, "completed", "Cut done.");
    }
    let Some(base) = p["provider"]["baseUrl"].as_str() else {
        return answer(false, "error", "no model endpoint");
    };
    let model = p["model"].as_str().unwrap_or("");
    let mut text_out = String::new();
    for turn in 0..10 {
        let reply = match turn_of(base, "/v1/responses", model, &prompt, turn) {
            Ok(r) => r,
            Err(e) => return answer(false, "error", &e),
        };
        for c in reply.commands {
            emit(json!({"type": "tool-call", "name": "edit", "ok": true}));
            h.apply(&json!({"arguments": {"commands": c}}));
        }
        if let Some(done) = reply.done {
            if let Some(u) = done.get("usage") {
                emit(
                    json!({"type": "usage", "input": u["input"], "cached": u["cached"], "output": u["output"]}),
                );
            }
            emit(json!({"type": "gate-continuation", "failures": ["plan item 1 unchecked"]}));
            text_out = done["text"].as_str().unwrap_or("").into();
            break;
        }
    }
    if h.has_marker("[aero-gate-fails]") {
        return answer(false, "gate-exhausted", "not done: checks fail");
    }
    answer(true, "completed", &text_out)
}

struct Turn {
    commands: Vec<Value>,
    done: Option<Value>,
}

/// One model turn against the endpoint: post, read the stream.
fn turn_of(base: &str, path: &str, model: &str, prompt: &str, turn: u32) -> Result<Turn, String> {
    let (origin, prefix) = match base.strip_prefix("http://").and_then(|r| r.split_once('/')) {
        Some((host, rest)) => (format!("http://{host}"), format!("/{rest}")),
        None => (base.to_owned(), String::new()),
    };
    let body = json!({"model": model, "prompt": prompt, "turn": turn}).to_string();
    let resp = http::send(
        &origin,
        "POST",
        &format!("{prefix}{path}"),
        &[("content-type".into(), "application/json".into())],
        body.as_bytes(),
    )?;
    if resp.status != 200 {
        let status = resp.status;
        return Err(format!(
            "model answered {status}: {}",
            String::from_utf8_lossy(&resp.body().unwrap_or_default())
        ));
    }
    let events = sse::parse(&resp.body()?);
    let mut t = Turn {
        commands: Vec::new(),
        done: None,
    };
    for e in events {
        match e.event.as_deref() {
            Some("fake.commands") => t
                .commands
                .push(e.data.map(|d| d["commands"].clone()).unwrap_or(Value::Null)),
            Some("fake.done") => t.done = e.data,
            _ => {}
        }
    }
    Ok(t)
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

/// Behaves by prompt. Normally it runs turns against the replay endpoint and
/// applies the edits the stream carries through the MCP proxy. Markers:
/// `[skip-replay]` edits without asking the model; `[exit N]` exits N after;
/// `[hang]` or `Tighten` sleeps 30 s; `[holder]` leaves a grandchild holding
/// its output and sleeps; `[holder-exit]` leaves one and exits at once;
/// `[steps N]` reports N tool calls, then sleeps; `[probe read=P connect=A]`
/// tries both and writes the results to home/probe.json.
fn fake_cli(args: &[String], cli: Cli) {
    let home = std::env::var("HOME").unwrap_or_default();
    let env: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
    std::fs::write(Path::new(&home).join("env.txt"), env.join("\n")).unwrap();
    std::fs::write(Path::new(&home).join("argv.json"), json!(args).to_string()).unwrap();
    let prompt = args.last().cloned().unwrap_or_default();
    let marker = |m: &str| {
        let rest = &prompt[prompt.find(m)? + m.len()..];
        Some(rest[..rest.find(']')?].to_owned())
    };
    let mut out = std::io::stdout().lock();
    if let Some(p) = marker("[probe ") {
        let field = |k: &str| {
            p.split_whitespace()
                .find_map(|w| w.strip_prefix(&format!("{k}=")))
                .map(str::to_owned)
        };
        let r = probe(field("read").as_deref(), field("connect").as_deref(), None);
        std::fs::write(Path::new(&home).join("probe.json"), r.to_string()).unwrap();
    }
    if prompt.contains("[holder") {
        // A grandchild that keeps stdout open (a distinctive duration, so a
        // test can look for survivors). Never waited on: it is meant to be
        // left behind for the runner to stop.
        #[allow(clippy::zombie_processes)]
        std::process::Command::new("/bin/sleep")
            .arg(if prompt.contains("[holder-exit]") {
                "32.3"
            } else {
                "31.7"
            })
            .stdout(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        if prompt.contains("[holder-exit]") {
            writeln!(out, r#"{{"type":"thread.started","thread_id":"th_fake"}}"#).unwrap();
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
    if let Some(n) = marker("[steps ") {
        for _ in 0..n.parse::<u32>().unwrap() {
            tool_event(&mut out, &cli, "project");
        }
        out.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
    if prompt.contains("Tighten") || prompt.contains("[hang]") {
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
    // Tell the fake host which prompt (and markers) this run has.
    if let Some(profile) = mcp_args
        .as_array()
        .and_then(|a| {
            a.iter()
                .position(|x| x == "--profile")
                .and_then(|i| a.get(i + 1))
        })
        .and_then(Value::as_str)
    {
        std::fs::write(Path::new(profile).join("fake-prompt.txt"), &prompt).unwrap();
        // An extension folder that is a link out of the run.
        if prompt.contains("[ext-symlink]") {
            let ext = Path::new(profile).join("extensions");
            std::fs::create_dir_all(&ext).unwrap();
            std::os::unix::fs::symlink("/etc", ext.join("evil")).unwrap();
        }
    }
    let edit = |commands: &Value| {
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
        for m in [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                   "params": {"name": "apply_commands", "arguments": {"commands": commands}}}),
        ] {
            writeln!(stdin, "{m}").unwrap();
            let mut l = String::new();
            stdout.read_line(&mut l).unwrap();
        }
        drop(stdin);
        child.wait().unwrap();
    };
    let model = args
        .iter()
        .position(|a| a == "--model")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_default();
    let (base, path) = match cli {
        Cli::Codex => (
            args.iter()
                .find_map(|a| a.strip_prefix("model_providers.replay="))
                .and_then(|v| v.split("base_url=\"").nth(1))
                .and_then(|v| v.split('"').next())
                .map(str::to_owned),
            "/responses",
        ),
        Cli::Claude => (std::env::var("ANTHROPIC_BASE_URL").ok(), "/v1/messages"),
    };
    let mut done = None;
    let mut failure = None;
    if prompt.contains("[skip-replay]") {
        edit(&correct_cut());
        tool_event(&mut out, &cli, "apply_commands");
        done = Some(json!({"text": "Done."}));
    } else if let Some(base) = base {
        for turn in 0..10 {
            match turn_of(&base, path, &model, &prompt, turn) {
                Err(e) => {
                    failure = Some(e);
                    break;
                }
                Ok(t) => {
                    for c in &t.commands {
                        edit(c);
                        tool_event(&mut out, &cli, "apply_commands");
                    }
                    if t.done.is_some() {
                        done = t.done;
                        break;
                    }
                }
            }
        }
    } else {
        failure = Some("no model endpoint".into());
    }
    let usage = done.as_ref().and_then(|d| d.get("usage")).cloned();
    let final_text = done
        .as_ref()
        .and_then(|d| d["text"].as_str())
        .unwrap_or("")
        .to_owned();
    match cli {
        Cli::Codex => {
            writeln!(out, r#"{{"type":"thread.started","thread_id":"th_fake"}}"#).unwrap();
            writeln!(out, r#"{{"type":"error","message":"Reconnecting... 1/5"}}"#).unwrap();
            writeln!(out, "{}", json!({"type": "item.completed", "item": {"type": "agent_message", "text": final_text}})).unwrap();
            match &failure {
                Some(f) => writeln!(
                    out,
                    "{}",
                    json!({"type": "turn.failed", "error": {"message": f}})
                )
                .unwrap(),
                None => {
                    let mut e = json!({"type": "turn.completed"});
                    if let Some(u) = &usage {
                        e["usage"] = json!({"input_tokens": u["input"], "cached_input_tokens": u["cached"], "output_tokens": u["output"]});
                    }
                    writeln!(out, "{e}").unwrap();
                }
            }
        }
        Cli::Claude => {
            let mut e = json!({"type": "result", "subtype": if failure.is_some() { "error_during_execution" } else { "success" },
                "is_error": failure.is_some(), "result": final_text});
            if let Some(u) = &usage {
                e["usage"] = json!({"input_tokens": u["input"].as_u64().unwrap_or(0) - u["cached"].as_u64().unwrap_or(0),
                    "cache_creation_input_tokens": 0, "cache_read_input_tokens": u["cached"], "output_tokens": u["output"]});
            }
            writeln!(out, "{e}").unwrap();
        }
    }
    out.flush().unwrap();
    if failure.is_some() {
        std::process::exit(1);
    }
    if let Some(code) = marker("[exit ") {
        std::process::exit(code.parse().unwrap());
    }
}

fn tool_event(out: &mut impl Write, cli: &Cli, name: &str) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let i = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    match cli {
        Cli::Codex => writeln!(out, "{}", json!({"type": "item.completed", "item": {"type": "mcp_tool_call", "server": "powermove", "tool": name}})),
        Cli::Claude => writeln!(out, "{}", json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": format!("tu_{i}"), "name": format!("mcp__powermove__{name}")}]}})),
    }
    .unwrap();
    out.flush().unwrap();
}
