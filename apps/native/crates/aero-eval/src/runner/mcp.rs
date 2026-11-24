//! A small MCP client (newline-delimited JSON-RPC 2.0) for the runner's own
//! connection to the headless `powermove-mcp` host: seeding, collection and
//! panel probes. Generic over the pipes so tests drive it with a fake host.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct McpClient<R: Read, W: Write> {
    reader: BufReader<R>,
    writer: W,
    next_id: u64,
    /// Notifications received while waiting for responses, in order.
    pub notifications: Vec<Value>,
}

/// A tool result: its content blocks and whether the host flagged an error.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    /// The first text block parsed as JSON (most Powermove tools answer so).
    pub fn json(&self) -> Option<Value> {
        self.content
            .iter()
            .filter(|c| c["type"] == "text")
            .find_map(|c| serde_json::from_str(c["text"].as_str()?).ok())
    }
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
    /// Decoded image blocks with their MIME types.
    pub fn images(&self) -> Vec<(String, Vec<u8>)> {
        use base64::Engine;
        self.content
            .iter()
            .filter(|c| c["type"] == "image")
            .filter_map(|c| {
                let data = base64::engine::general_purpose::STANDARD
                    .decode(c["data"].as_str()?)
                    .ok()?;
                Some((c["mimeType"].as_str().unwrap_or("").to_owned(), data))
            })
            .collect()
    }
}

impl<R: Read, W: Write> McpClient<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
            notifications: Vec::new(),
        }
    }

    /// Send a request and wait for its response; returns `result` or the
    /// JSON-RPC error as text.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string();
        writeln!(self.writer, "{line}")
            .and_then(|()| self.writer.flush())
            .map_err(|e| format!("{method}: host closed its input ({e})"))?;
        loop {
            let mut buf = String::new();
            let n = self
                .reader
                .read_line(&mut buf)
                .map_err(|e| format!("{method}: {e}"))?;
            if n == 0 {
                return Err(format!("{method}: host exited before answering"));
            }
            let Ok(m) = serde_json::from_str::<Value>(&buf) else {
                return Err(format!("{method}: host wrote a line that is not JSON"));
            };
            if m.get("id") == Some(&json!(id)) {
                if let Some(e) = m.get("error") {
                    return Err(format!(
                        "{method}: {} ({})",
                        e["message"].as_str().unwrap_or("error"),
                        e["code"]
                    ));
                }
                return Ok(m.get("result").cloned().unwrap_or(Value::Null));
            }
            if m.get("method").is_some() && m.get("id").is_none() {
                self.notifications.push(m);
            }
        }
    }

    pub fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let line = json!({"jsonrpc":"2.0","method":method,"params":params}).to_string();
        writeln!(self.writer, "{line}")
            .and_then(|()| self.writer.flush())
            .map_err(|e| e.to_string())
    }

    pub fn initialize(&mut self) -> Result<Value, String> {
        let r = self.request(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"aero-eval","version":"1"}}),
        )?;
        self.notify("notifications/initialized", json!({}))?;
        Ok(r)
    }

    /// Call a tool. A host-flagged error comes back as `Err` with its text.
    pub fn call(&mut self, name: &str, args: Value) -> Result<ToolResult, String> {
        let r = self.request("tools/call", json!({"name":name,"arguments":args}))?;
        let result = ToolResult {
            content: r["content"].as_array().cloned().unwrap_or_default(),
            is_error: r["isError"].as_bool().unwrap_or(false),
        };
        if result.is_error {
            return Err(format!("{name}: {}", result.text()));
        }
        Ok(result)
    }

    pub fn tool_names(&mut self) -> Result<Vec<String>, String> {
        let r = self.request("tools/list", json!({}))?;
        Ok(r["tools"]
            .as_array()
            .map(|t| {
                t.iter()
                    .filter_map(|t| t["name"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// The headless host as a child process, with the runner's stdio client.
pub struct Host {
    pub child: Child,
    pub client: McpClient<ChildStdout, ChildStdin>,
}

impl Host {
    /// `powermove-mcp --profile P --exports E --listen`: the runner talks over
    /// stdio; harnesses attach with `--powermove-mcp --profile P`.
    pub fn spawn(
        bin: &std::path::Path,
        profile: &std::path::Path,
        exports: &std::path::Path,
        log: &std::path::Path,
    ) -> Result<Self, String> {
        let stderr = std::fs::File::create(log).map_err(|e| e.to_string())?;
        let mut child = Command::new(bin)
            .arg("--profile")
            .arg(profile)
            .arg("--exports")
            .arg(exports)
            .arg("--listen")
            // Never the owner's home: the host sees only the lane profile.
            .env("HOME", profile)
            .env("PM_NATIVE_NO_ORIGINAL", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(|e| format!("{}: {e}", bin.display()))?;
        let client = McpClient::new(child.stdout.take().unwrap(), child.stdin.take().unwrap());
        let mut host = Host { child, client };
        host.client.initialize()?;
        Ok(host)
    }

    /// Close stdin (the host's clean shutdown) and reap it, killing it if it
    /// does not exit within `grace`.
    pub fn close(self, grace: std::time::Duration) -> Option<i32> {
        let Host { mut child, client } = self;
        drop(client);
        let deadline = std::time::Instant::now() + grace;
        loop {
            if let Ok(Some(s)) = child.try_wait() {
                return s.code();
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                return child.wait().ok().and_then(|s| s.code());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
