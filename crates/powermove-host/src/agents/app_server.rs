//! A `codex app-server --stdio` connection: newline-delimited JSON-RPC.
//!
//! Requests from us settle by id. Everything else the server sends —
//! notifications, and requests it makes of us (questions, elicitations) —
//! goes to the owner's event channel, which must answer every server request.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};

use super::runtime;

const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum Incoming {
    Notification { method: String, params: Value },
    /// The server asks us something; reply with [`AppServer::respond`].
    Request { id: Value, method: String, params: Value },
    /// The process stopped; every pending request already failed.
    Exited(String),
}

type Pending = oneshot::Sender<Result<Value, String>>;

pub struct AppServer {
    writer: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, Pending>>,
    next_id: AtomicU64,
    alive: AtomicBool,
    child: tokio::sync::Mutex<Option<Child>>,
    request_timeout: Duration,
}

/// `method` matches `suffix` exactly or as its last path segments.
pub fn method_is(method: &str, suffix: &str) -> bool {
    method == suffix || method.strip_suffix(suffix).is_some_and(|head| head.ends_with('/'))
}

impl AppServer {
    /// Launch the server and complete the `initialize` handshake.
    pub async fn start(binary: &Path, env: &[(&str, OsString)], client_name: &str, experimental: bool, request_timeout: Duration) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Incoming>)> {
        let args = vec!["app-server".to_owned(), "--stdio".to_owned()];
        let mut child = runtime::command(binary, &args, None, env, &[]).spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("Codex App Server has no stdin."))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("Codex App Server has no stdout."))?;
        let stderr = child.stderr.take();
        let (writer, mut outbox) = mpsc::unbounded_channel::<String>();
        let (events, receiver) = mpsc::unbounded_channel();
        let server = Arc::new(Self {
            writer,
            pending: Mutex::default(),
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            child: tokio::sync::Mutex::new(Some(child)),
            request_timeout,
        });

        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = outbox.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });
        let tail = Arc::new(Mutex::new(String::new()));
        if let Some(mut stderr) = stderr {
            let tail = tail.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                while let Ok(read) = stderr.read(&mut buffer).await {
                    if read == 0 {
                        break;
                    }
                    let mut text = tail.lock();
                    text.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    if text.len() > 4_000 {
                        let cut = text.len() - 4_000;
                        let cut = (cut..text.len()).find(|index| text.is_char_boundary(*index)).unwrap_or(text.len());
                        text.drain(..cut);
                    }
                }
            });
        }
        let reader = Arc::downgrade(&server);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                match lines.read_until(b'\n', &mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() > MAX_LINE_BYTES => continue,
                    Ok(_) => {}
                }
                let Some(server) = reader.upgrade() else { break };
                server.receive(&line, &events);
            }
            if let Some(server) = reader.upgrade() {
                let detail = tail.lock().trim().to_owned();
                let message = if detail.is_empty() { "Codex App Server stopped.".to_owned() } else { format!("Codex App Server stopped: {detail}") };
                server.fail_all(&message);
                let _ = events.send(Incoming::Exited(message));
            }
        });

        let capabilities = if experimental { json!({ "experimentalApi": true, "requestAttestation": false }) } else { Value::Null };
        let mut params = json!({ "clientInfo": { "name": client_name, "title": "Powermove", "version": "1.0.0" } });
        if !capabilities.is_null() {
            params["capabilities"] = capabilities;
        }
        if let Err(error) = server.request("initialize", params).await {
            server.shutdown().await;
            return Err(error);
        }
        server.notify("initialized", json!({}));
        Ok((server, receiver))
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        if !self.is_alive() {
            bail!("Codex App Server is not running.");
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().insert(id, sender);
        self.write(json!({ "method": method, "id": id, "params": params }));
        match tokio::time::timeout(self.request_timeout, receiver).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(message))) => Err(anyhow!(message)),
            Ok(Err(_)) => bail!("Codex App Server stopped."),
            Err(_) => {
                self.pending.lock().remove(&id);
                bail!("Codex App Server timed out while handling {method}.")
            }
        }
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.write(json!({ "method": method, "params": params }));
    }

    pub fn respond(&self, id: &Value, result: Value) {
        self.write(json!({ "id": id, "result": result }));
    }

    pub fn respond_error(&self, id: &Value, message: &str) {
        self.write(json!({ "id": id, "error": { "code": -32601, "message": message } }));
    }

    pub async fn shutdown(&self) {
        self.alive.store(false, Ordering::Release);
        self.fail_all("Codex App Server stopped.");
        if let Some(mut child) = self.child.lock().await.take() {
            runtime::kill_tree(&mut child).await;
        }
    }

    fn write(&self, message: Value) {
        if self.is_alive() {
            let _ = self.writer.send(format!("{message}\n"));
        }
    }

    fn fail_all(&self, message: &str) {
        self.alive.store(false, Ordering::Release);
        for (_, pending) in self.pending.lock().drain() {
            let _ = pending.send(Err(message.to_owned()));
        }
    }

    fn receive(&self, line: &[u8], events: &mpsc::UnboundedSender<Incoming>) {
        let Ok(Value::Object(mut message)) = serde_json::from_slice::<Value>(line) else { return };
        let method = message.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = message.remove("id");
        let params = message.remove("params").filter(Value::is_object).unwrap_or_else(|| json!({}));
        match (id, method) {
            // A server request carries an id too; it must never settle one of ours.
            (Some(id @ (Value::Number(_) | Value::String(_))), Some(method)) => {
                let _ = events.send(Incoming::Request { id, method, params });
            }
            (Some(Value::Number(id)), None) => {
                let Some(pending) = id.as_u64().and_then(|id| self.pending.lock().remove(&id)) else { return };
                let result = match message.remove("error") {
                    Some(error) if error.is_object() => Err(app_server_error(&json!({ "error": error }), "Codex App Server request failed.")),
                    _ => Ok(message.remove("result").unwrap_or(Value::Null)),
                };
                let _ = pending.send(result);
            }
            (None, Some(method)) => {
                let _ = events.send(Incoming::Notification { method, params });
            }
            _ => {}
        }
    }
}

/// The human message inside an app-server `{ error }`.
pub fn app_server_error(value: &Value, fallback: &str) -> String {
    let error = &value["error"];
    if error["codexErrorInfo"] == "contextWindowExceeded" {
        return "This conversation is too long for the model, even after compacting it. Start a new thread to continue.".into();
    }
    match error["message"].as_str() {
        Some(message) if !message.is_empty() => super::trace::truncate(message, 4_000),
        _ => fallback.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_method_suffixes() {
        assert!(method_is("turn/completed", "turn/completed"));
        assert!(method_is("codex/event/turn/completed", "turn/completed"));
        assert!(!method_is("returnturn/completed", "turn/completed"));
    }

    #[test]
    fn routes_responses_notifications_and_server_requests() {
        let (writer, _outbox) = mpsc::unbounded_channel();
        let server = AppServer {
            writer,
            pending: Mutex::default(),
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            child: tokio::sync::Mutex::new(None),
            request_timeout: Duration::from_secs(1),
        };
        let (sender, mut receiver) = oneshot::channel();
        server.pending.lock().insert(7, sender);
        let (events, mut incoming) = mpsc::unbounded_channel();
        server.receive(br#"{"id":7,"result":{"ok":true}}"#, &events);
        assert_eq!(receiver.try_recv().unwrap().unwrap(), json!({ "ok": true }));
        server.receive(br#"{"id":3,"method":"item/tool/requestUserInput","params":{"threadId":"t"}}"#, &events);
        assert!(matches!(incoming.try_recv().unwrap(), Incoming::Request { method, .. } if method == "item/tool/requestUserInput"));
        server.receive(br#"{"method":"turn/completed","params":{"turn":{"id":"u"}}}"#, &events);
        assert!(matches!(incoming.try_recv().unwrap(), Incoming::Notification { method, .. } if method == "turn/completed"));
    }
}
