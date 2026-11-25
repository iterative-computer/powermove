//! Write recordings. [`Recorder`] takes exchanges from any transport (Aero's
//! Rust fetch bridge pushes them directly); [`RecordingProxy`] sits between a
//! CLI harness and a plain-HTTP model endpoint and records what passes.
//! Real provider endpoints are HTTPS: recording them needs a TLS upstream,
//! which only an approved batch sets up (see docs/native/aero-eval.md).
use super::http;
use super::sse;
use super::{Exchange, FORMAT, Normalise, Recording, Request, Response, normalise_paths};
use serde_json::Value;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// What a recording is of.
#[derive(Clone, Debug)]
pub struct Meta {
    pub synthetic: bool,
    pub provider: String,
    pub harness: String,
    pub model: String,
    pub effort: String,
    pub task: String,
}

pub struct Recorder {
    meta: Meta,
    folders: Vec<String>,
    normalise: Normalise,
    exchanges: Vec<Exchange>,
    /// Anything that could not be recorded faithfully; `finish` fails on it.
    errors: Vec<String>,
}

impl Recorder {
    /// `folders` are this run's generated working folders (rewritten to the
    /// workspace placeholder); `volatile` the per-run request fields.
    pub fn new(meta: Meta, folders: Vec<String>, volatile: Vec<String>) -> Self {
        Self {
            meta,
            folders,
            normalise: Normalise { volatile },
            exchanges: Vec::new(),
            errors: Vec::new(),
        }
    }
    /// Record one exchange. Only the provider's known paths and query keys
    /// are kept; a request to any other path makes the recording fail.
    pub fn push(&mut self, mut request: Request, mut response: Response) {
        match super::recorded_path(&self.meta.provider, &request.path) {
            Ok(p) => request.path = p,
            Err(e) => {
                self.errors.push(e);
                return;
            }
        }
        normalise_paths(&mut request.body, &self.folders);
        // Stored masked, so a re-recording of the same run is byte-identical.
        super::mask_volatile(&mut request.body, &self.normalise.volatile);
        for e in &mut response.events {
            if let Some(d) = e.data.as_mut() {
                normalise_paths(d, &self.folders);
            }
        }
        if let Some(b) = response.body.as_mut() {
            normalise_paths(b, &self.folders);
        }
        self.exchanges.push(Exchange { request, response });
    }
    /// A stream that could not be captured exactly fails the recording.
    pub fn error(&mut self, e: String) {
        self.errors.push(e);
    }
    pub fn finish(self) -> Result<Recording, String> {
        if let Some(e) = self.errors.first() {
            return Err(format!("the recording is incomplete: {e}"));
        }
        let r = Recording {
            format: FORMAT.into(),
            version: 1,
            synthetic: self.meta.synthetic,
            provider: self.meta.provider,
            harness: self.meta.harness,
            model: self.meta.model,
            effort: self.meta.effort,
            task: self.meta.task,
            normalise: self.normalise,
            exchanges: self.exchanges,
            review: None,
        };
        r.validate()?;
        Ok(r)
    }
}

/// A loopback proxy that forwards to `upstream` (plain HTTP) and records.
/// Request headers are forwarded but never recorded.
pub struct RecordingProxy {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    recorder: Arc<Mutex<Option<Recorder>>>,
    errors: Arc<Mutex<Vec<String>>>,
    thread: Option<JoinHandle<()>>,
}

impl RecordingProxy {
    pub fn start(upstream: &str, recorder: Recorder) -> Result<Self, String> {
        if !upstream.starts_with("http://") {
            return Err(format!(
                "{upstream}: the recording proxy forwards to plain HTTP only; a TLS upstream is part of an approved batch's setup"
            ));
        }
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let recorder = Arc::new(Mutex::new(Some(recorder)));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let (s, rec, errs, up) = (
            stop.clone(),
            recorder.clone(),
            errors.clone(),
            upstream.to_owned(),
        );
        let thread = std::thread::spawn(move || {
            for conn in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = conn else { continue };
                let deadline = Some(super::replayer::SOCKET_DEADLINE);
                if let Err(e) = stream
                    .set_read_timeout(deadline)
                    .and_then(|()| stream.set_write_timeout(deadline))
                    .map_err(|e| e.to_string())
                    .and_then(|()| forward(&mut stream, &up, &rec))
                {
                    errs.lock().unwrap().push(e);
                }
            }
        });
        Ok(Self {
            addr,
            stop,
            recorder,
            errors,
            thread: Some(thread),
        })
    }
    pub fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }
    /// Stop and return the recording (or the first forwarding error).
    pub fn finish(mut self) -> Result<Recording, String> {
        self.shutdown();
        if let Some(e) = self.errors.lock().unwrap().first() {
            return Err(e.clone());
        }
        self.recorder
            .lock()
            .unwrap()
            .take()
            .ok_or("recording already taken")?
            .finish()
    }
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect_timeout(&self.addr, super::replayer::SOCKET_DEADLINE);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for RecordingProxy {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn forward(
    client: &mut std::net::TcpStream,
    upstream: &str,
    recorder: &Mutex<Option<Recorder>>,
) -> Result<(), String> {
    use std::io::Write;
    let req = http::read_request(client)?;
    let body: Value = if req.body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&req.body).map_err(|e| format!("request body is not JSON: {e}"))?
    };
    let resp = http::send(upstream, &req.method, &req.path, &req.headers, &req.body)?;
    let status = resp.status;
    let response = if resp.is_sse() {
        http::start_sse(client, status).map_err(|e| e.to_string())?;
        let mut parser = sse::Parser::default();
        let mut events = Vec::new();
        resp.for_each_piece(|piece| {
            client
                .write_all(piece)
                .and_then(|()| client.flush())
                .map_err(|e| e.to_string())?;
            events.extend(parser.feed(piece));
            Ok(())
        })?;
        events.extend(parser.finish());
        if let Some(e) = parser.errors.first()
            && let Some(r) = recorder.lock().unwrap().as_mut()
        {
            r.error(e.clone());
        }
        Response {
            status,
            events,
            body: None,
        }
    } else {
        let bytes = resp.body()?;
        http::write_json(client, status, &bytes).map_err(|e| e.to_string())?;
        Response {
            status,
            events: Vec::new(),
            body: Some(serde_json::from_slice(&bytes).unwrap_or(Value::Null)),
        }
    };
    if let Some(r) = recorder.lock().unwrap().as_mut() {
        r.push(
            Request {
                method: req.method,
                path: req.path,
                body,
            },
            response,
        );
    }
    Ok(())
}
