//! Feed a recorded model stream back to a harness. Requests must arrive in the
//! recorded order and match the recording after normalisation. The first
//! error of any kind (a divergence, a malformed or unfinished request, a body
//! that is not JSON) is latched: it ends the replay, every later request gets
//! the same 409 naming it, and the report fails. A harness regression shows up
//! as one precise failure.
use super::http;
use super::{Difference, Exchange, Recording, Request, Response, diff, mask_volatile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// How long one connection may take to send its request or accept the reply.
pub const SOCKET_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Divergence {
    /// Index of the exchange the request was compared with.
    pub exchange: usize,
    /// `request-differs`, `unexpected-request` or `malformed-request`.
    pub kind: String,
    pub differences: Vec<Difference>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayReport {
    pub task: String,
    pub harness: String,
    pub exchanges: usize,
    pub consumed: usize,
    pub divergence: Option<Divergence>,
    /// Every recorded exchange was requested, in order, and matched.
    pub passed: bool,
}

/// The in-process replayer: Aero's Rust fetch bridge calls this directly; the
/// loopback server wraps it for CLI harnesses.
pub struct Replayer {
    recording: Recording,
    /// Working folders of this run, rewritten to the workspace placeholder.
    folders: Vec<String>,
    cursor: usize,
    divergence: Option<Divergence>,
}

impl Replayer {
    pub fn new(recording: Recording, folders: Vec<String>) -> Self {
        Self {
            recording,
            folders,
            cursor: 0,
            divergence: None,
        }
    }

    fn comparable(&self, body: &Value, normalise_paths: bool) -> Value {
        let mut b = body.clone();
        if normalise_paths {
            super::normalise_paths(&mut b, &self.folders);
        }
        mask_volatile(&mut b, &self.recording.normalise.volatile);
        b
    }

    /// Answer one request, or report why it diverges.
    pub fn respond(&mut self, req: &Request) -> Result<Response, Divergence> {
        if let Some(d) = &self.divergence {
            return Err(d.clone());
        }
        let Some(Exchange { request, response }) = self.recording.exchanges.get(self.cursor) else {
            return Err(self.fail(Divergence {
                exchange: self.cursor,
                kind: "unexpected-request".into(),
                differences: Vec::new(),
                message: format!(
                    "the recording has {} exchanges; the harness sent another ({} {})",
                    self.recording.exchanges.len(),
                    req.method,
                    req.path
                ),
            }));
        };
        let mut differences = Vec::new();
        if request.method != req.method {
            differences.push(Difference {
                path: "#method".into(),
                expected: Some(Value::from(request.method.clone())),
                actual: Some(Value::from(req.method.clone())),
            });
        }
        // Compared as it would have been recorded (unrecorded query keys
        // dropped); a path the provider never records differs outright.
        let actual = super::recorded_path(&self.recording.provider, &req.path)
            .unwrap_or_else(|_| req.path.clone());
        if request.path != actual {
            differences.push(Difference {
                path: "#path".into(),
                expected: Some(Value::from(request.path.clone())),
                actual: Some(Value::from(actual)),
            });
        }
        differences.extend(diff(
            &self.comparable(&request.body, false),
            &self.comparable(&req.body, true),
        ));
        if !differences.is_empty() {
            return Err(self.fail(Divergence {
                exchange: self.cursor,
                kind: "request-differs".into(),
                differences,
                message: String::new(),
            }));
        }
        let response = response.clone();
        self.cursor += 1;
        Ok(response)
    }

    /// Latch the first error; later ones are answered with it unchanged.
    pub fn fail(&mut self, d: Divergence) -> Divergence {
        self.divergence.get_or_insert(d).clone()
    }

    /// Latch a request that could not be read or parsed.
    pub fn malformed(&mut self, message: String) -> Divergence {
        let at = self.cursor;
        self.fail(Divergence {
            exchange: at,
            kind: "malformed-request".into(),
            differences: Vec::new(),
            message,
        })
    }

    pub fn report(&self) -> ReplayReport {
        ReplayReport {
            task: self.recording.task.clone(),
            harness: self.recording.harness.clone(),
            exchanges: self.recording.exchanges.len(),
            consumed: self.cursor,
            divergence: self.divergence.clone(),
            passed: self.divergence.is_none() && self.cursor == self.recording.exchanges.len(),
        }
    }
}

/// The replayer on a loopback port, for harnesses that call a model over HTTP
/// (point their base URL at [`ReplayServer::origin`]).
pub struct ReplayServer {
    addr: SocketAddr,
    deadline: Duration,
    stop: Arc<AtomicBool>,
    replayer: Arc<Mutex<Replayer>>,
    thread: Option<JoinHandle<()>>,
}

impl ReplayServer {
    pub fn start(replayer: Replayer) -> Result<Self, String> {
        Self::start_with_deadline(replayer, SOCKET_DEADLINE)
    }

    /// `deadline` bounds each connection's reads and writes, so a request
    /// that never finishes cannot hold the server (or its shutdown).
    pub fn start_with_deadline(replayer: Replayer, deadline: Duration) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let replayer = Arc::new(Mutex::new(replayer));
        let (s, r) = (stop.clone(), replayer.clone());
        let thread = std::thread::spawn(move || {
            for conn in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = conn else { continue };
                if stream.set_read_timeout(Some(deadline)).is_err()
                    || stream.set_write_timeout(Some(deadline)).is_err()
                {
                    r.lock()
                        .unwrap()
                        .malformed("could not set the socket deadline".into());
                    continue;
                }
                // One request per connection, served in arrival order.
                serve_one(&mut stream, &r);
            }
        });
        Ok(Self {
            addr,
            deadline,
            stop,
            replayer,
            thread: Some(thread),
        })
    }

    pub fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Stop serving and return what happened.
    pub fn finish(mut self) -> ReplayReport {
        self.shutdown();
        self.replayer.lock().unwrap().report()
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect_timeout(&self.addr, self.deadline);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ReplayServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn serve_one(stream: &mut std::net::TcpStream, replayer: &Mutex<Replayer>) {
    let answer = match http::read_request(stream) {
        Err(e) => Err(replayer.lock().unwrap().malformed(e)),
        Ok(req) => match serde_json::from_slice::<Value>(&req.body).or_else(|e| {
            if req.body.is_empty() {
                Ok(Value::Null)
            } else {
                Err(e)
            }
        }) {
            Err(e) => Err(replayer
                .lock()
                .unwrap()
                .malformed(format!("request body is not JSON: {e}"))),
            Ok(body) => replayer.lock().unwrap().respond(&Request {
                method: req.method,
                path: req.path,
                body,
            }),
        },
    };
    let written = match answer {
        Ok(resp) if resp.events.is_empty() => {
            let body = resp.body.unwrap_or(Value::Null).to_string();
            http::write_json(stream, resp.status, body.as_bytes())
        }
        Ok(resp) => http::start_sse(stream, resp.status).and_then(|()| {
            use std::io::Write;
            for e in &resp.events {
                let mut buf = Vec::new();
                e.write(&mut buf);
                stream.write_all(&buf)?;
                stream.flush()?;
            }
            Ok(())
        }),
        Err(d) => {
            let body = serde_json::json!({"error": {"type": "replay_divergence", "divergence": d}});
            http::write_json(stream, 409, body.to_string().as_bytes())
        }
    };
    if let Err(e) = written {
        // A reply the harness never received is not a replayed exchange.
        replayer
            .lock()
            .unwrap()
            .malformed(format!("the reply could not be written: {e}"));
    }
}
