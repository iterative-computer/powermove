//! Synthetic kernel-side server. No AppKit objects or main-thread work here.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub struct State {
    pub s5b: crate::s5b::Surfaces,
    pub token: String,
    pub port: u16,
    pub results: Mutex<Vec<Value>>,
    pub done: AtomicBool,
    pub blocked: AtomicBool,
    pub pings: AtomicUsize,
    pub failures: AtomicUsize,
    pub life_phase: AtomicUsize,
    pub life_ready: AtomicBool,
    pub loop_started: AtomicBool,
    pub terminated: AtomicBool,
    pub restarted: AtomicBool,
    pub cancelled_tasks: Mutex<std::collections::HashSet<usize>>,
    pub epoch: Instant,
    pub pixels: Vec<u8>,
    pub renders: Mutex<crate::pixels::Renders>,
}
impl State {
    pub fn ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }
    pub fn record(&self, v: Value) {
        println!("{}", v);
        self.results.lock().unwrap().push(v);
    }
}
pub fn start() -> Arc<State> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut random = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .unwrap()
        .read_exact(&mut random)
        .unwrap();
    let state = Arc::new(State {
        s5b: crate::s5b::Surfaces::new(),
        token: random.iter().map(|b| format!("{b:02x}")).collect(),
        port: listener.local_addr().unwrap().port(),
        results: Mutex::new(vec![]),
        done: AtomicBool::new(false),
        blocked: AtomicBool::new(false),
        pings: AtomicUsize::new(0),
        failures: AtomicUsize::new(0),
        life_phase: AtomicUsize::new(1),
        life_ready: AtomicBool::new(false),
        loop_started: AtomicBool::new(false),
        terminated: AtomicBool::new(false),
        restarted: AtomicBool::new(false),
        cancelled_tasks: Mutex::new(std::collections::HashSet::new()),
        epoch: Instant::now(),
        pixels: crate::pixels::frame(),
        renders: Mutex::new(crate::pixels::Renders::default()),
    });
    let s = state.clone();
    std::thread::Builder::new()
        .name("s5-kernel-accept".into())
        .spawn(move || {
            let connections = Arc::new(AtomicUsize::new(0));
            for stream in listener.incoming().flatten() {
                if connections.fetch_add(1, Ordering::SeqCst) >= 16 {
                    connections.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                let s = s.clone();
                let c = connections.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, s);
                    c.fetch_sub(1, Ordering::SeqCst);
                });
            }
        })
        .unwrap();
    state
}
fn serve(stream: TcpStream, s: Arc<State>) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let route = line.split_whitespace().nth(1).unwrap_or("").to_string();
        let mut length = 0usize;
        let mut host = String::new();
        let mut origin = String::new();
        let mut ws_key = None;
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            if line == "\r\n" || line.is_empty() {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("content-length:") {
                length = line
                    .split(':')
                    .nth(1)
                    .unwrap_or("")
                    .trim()
                    .parse()
                    .unwrap_or(usize::MAX);
            }
            if lower.starts_with("host:") {
                host = line[5..].trim().to_string();
            }
            if lower.starts_with("sec-websocket-key:") {
                ws_key = Some(line[18..].trim().to_string());
            }
            if lower.starts_with("origin:") {
                origin = line[7..].trim().to_string();
            }
        }
        if length > 20 * 1024 * 1024 {
            return Ok(());
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        let expected_host = format!("127.0.0.1:{}", s.port);
        let expected_origin = format!("http://{expected_host}");
        let base = format!("/{}/", s.token);
        if host != expected_host
            || (!origin.is_empty() && origin != expected_origin)
            || !route.starts_with(&base)
        {
            respond(reader.get_mut(), 403, "text/plain", b"denied")?;
            continue;
        }
        let path = route[base.len()..].split('?').next().unwrap_or("");
        if path == "ws" {
            if let Some(key) = ws_key {
                let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                )?;
                reader.get_mut().flush()?;
                return serve_ws(reader.into_inner(), s);
            }
            respond(
                reader.get_mut(),
                400,
                "text/plain",
                b"missing websocket key",
            )?;
            continue;
        }
        if path.starts_with("s5b/") {
            let (mime, bytes) = s.s5b.request(&format!("/{path}"));
            respond(reader.get_mut(), 200, mime, &bytes)?;
            continue;
        }
        match path {
            "" => {
                let page = include_str!("page.html")
                    .replace("__BASE__", &base)
                    .replace("__PIXELS_JS__", include_str!("pixels.js"))
                    .replace("__S5B_JS__", include_str!("s5b.js"))
                    .replace("__LIFECYCLE_JS__", include_str!("lifecycle.js"))
                    .replace(
                        "__RESTARTED__",
                        if s.restarted.load(Ordering::SeqCst) {
                            "true"
                        } else {
                            "false"
                        },
                    )
                    .replace(
                        "__CASE__",
                        &std::env::var("S5_CASE").unwrap_or_else(|_| "http".into()),
                    );
                respond(
                    reader.get_mut(),
                    200,
                    "text/html; charset=utf-8",
                    page.as_bytes(),
                )?;
            }
            "life-next" => {
                let after: usize = route.split('?').nth(1).unwrap_or("0").parse().unwrap_or(0);
                let start = Instant::now();
                while s.life_phase.load(Ordering::SeqCst) <= after
                    && start.elapsed() < Duration::from_secs(100)
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                respond(
                    reader.get_mut(),
                    200,
                    "application/json",
                    json!({"phase":s.life_phase.load(Ordering::SeqCst)})
                        .to_string()
                        .as_bytes(),
                )?;
            }
            "life-ready" => {
                s.life_ready.store(true, Ordering::SeqCst);
                respond(reader.get_mut(), 200, "text/plain", b"ok")?;
            }
            "loop-start" => {
                s.loop_started.store(true, Ordering::SeqCst);
                respond(reader.get_mut(), 200, "text/plain", b"ok")?;
            }
            "kernel-snapshot" => respond(
                reader.get_mut(),
                200,
                "application/json",
                b"{\"revision\":7,\"acknowledged\":true}",
            )?,
            "outer-pixels" => {
                let renders = s.renders.lock().unwrap();
                respond(
                    reader.get_mut(),
                    200,
                    "text/plain; charset=x-user-defined",
                    renders.output.as_deref().unwrap_or(&[]),
                )?;
            }
            "render-override" => {
                let query = route.split('?').nth(1).unwrap_or("");
                let parts: Vec<_> = query.split(',').collect();
                let id = parts.first().and_then(|x| x.parse().ok()).unwrap_or(0);
                let rev = parts.get(1).and_then(|x| x.parse().ok()).unwrap_or(0);
                let ok = s.renders.lock().unwrap().upload(id, rev, &body);
                respond(
                    reader.get_mut(),
                    200,
                    "application/json",
                    json!({"ok":ok}).to_string().as_bytes(),
                )?;
            }
            "pixels" => respond(
                reader.get_mut(),
                200,
                "text/plain; charset=x-user-defined",
                &s.pixels,
            )?,
            "pixels-upload" | "pixels-text-upload" => {
                let valid = if path == "pixels-upload" {
                    crate::pixels::valid(&body)
                } else {
                    match std::str::from_utf8(&body) {
                        Ok(text) => crate::pixels::valid(
                            &text.chars().map(|c| c as u32 as u8).collect::<Vec<_>>(),
                        ),
                        Err(_) => false,
                    }
                };
                respond(
                    reader.get_mut(),
                    200,
                    "application/json",
                    json!({"ok":valid,"bytes":crate::pixels::BYTES})
                        .to_string()
                        .as_bytes(),
                )?;
            }
            "frame-ref" | "consume-ref" => {
                let ok = path == "frame-ref" || body == b"frame0";
                respond(
                    reader.get_mut(),
                    200,
                    "application/json",
                    json!({"ok":ok,"id":"frame0","bytes":s.pixels.len(),"revision":7})
                        .to_string()
                        .as_bytes(),
                )?;
            }
            "render-begin" | "render-nested" => {
                let result = s.renders.lock().unwrap().begin(path == "render-nested");
                respond(
                    reader.get_mut(),
                    200,
                    "application/json",
                    result.to_string().as_bytes(),
                )?;
            }
            "render-resume" => {
                let input: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let result = s.renders.lock().unwrap().resume(
                    input["render"].as_u64().unwrap_or(0),
                    input["revision"].as_u64().unwrap_or(0),
                    input["dependency"].as_bool().unwrap_or(false),
                );
                respond(
                    reader.get_mut(),
                    200,
                    "application/json",
                    result.to_string().as_bytes(),
                )?;
            }
            "ping" => {
                if s.pings.fetch_add(1, Ordering::SeqCst) == 0 {
                    s.record(json!({"test":"first_http_ping"}));
                }
                respond(reader.get_mut(), 200, "application/json", b"{\"ok\":true}")?;
            }
            "report" => {
                if let Ok(v) = serde_json::from_slice::<Value>(&body) {
                    if v["test"] == "fatal"
                        || v["test"] == "script_error"
                        || v["status"] == "failed"
                        || v["status"] == "fail"
                    {
                        s.failures.fetch_add(1, Ordering::SeqCst);
                    }
                    s.record(v);
                }
                respond(reader.get_mut(), 200, "text/plain", b"ok")?;
            }
            "done" => {
                s.done.store(true, Ordering::SeqCst);
                respond(reader.get_mut(), 200, "text/plain", b"ok")?;
            }
            "hold" => {
                s.blocked.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(300));
                s.blocked.store(false, Ordering::SeqCst);
                respond(reader.get_mut(), 200, "text/plain", b"released")?;
            }
            "poll" => {
                std::thread::sleep(Duration::from_millis(2));
                respond(
                    reader.get_mut(),
                    200,
                    "text/plain",
                    s.ns().to_string().as_bytes(),
                )?;
            }
            "ack" => {
                if let Ok(t) = String::from_utf8_lossy(&body).parse::<u64>() {
                    s.results
                        .lock()
                        .unwrap()
                        .push(json!({"poll_ack_ms":s.ns().saturating_sub(t) as f64/1e6}));
                }
                respond(reader.get_mut(), 200, "text/plain", b"ok")?;
            }
            _ => respond(reader.get_mut(), 404, "text/plain", b"missing")?,
        }
    }
}
pub fn respond(
    stream: &mut TcpStream,
    status: u16,
    mime: &str,
    body: &[u8],
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: keep-alive\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}
pub fn stats(mut a: Vec<f64>) -> Value {
    a.sort_by(f64::total_cmp);
    let n = a.len();
    if n == 0 {
        return json!({"n":0});
    }
    let p = |q: f64| {
        a[((q * n as f64).ceil() as usize)
            .saturating_sub(1)
            .min(n - 1)]
    };
    json!({"n":n,"p50_ms":p(0.5),"p95_ms":p(0.95),"p99_ms":p(0.99),"max_ms":a[n-1]})
}

fn serve_ws(stream: TcpStream, s: Arc<State>) -> std::io::Result<()> {
    // Handshake is parsed by the same authenticated HTTP path, including partial reads.
    let mut ws =
        tungstenite::WebSocket::from_raw_socket(stream, tungstenite::protocol::Role::Server, None);
    for _ in 0..400 {
        std::thread::sleep(Duration::from_millis(2));
        let start = s.ns();
        ws.send(tungstenite::Message::Text(start.to_string().into()))
            .map_err(std::io::Error::other)?;
        let msg = ws.read().map_err(std::io::Error::other)?;
        if msg.to_text().ok().and_then(|x| x.parse::<u64>().ok()) != Some(start) {
            return Err(std::io::Error::other("wrong websocket acknowledgment"));
        }
        s.results
            .lock()
            .unwrap()
            .push(json!({"ws_ack_ms":(s.ns()-start)as f64/1e6}));
    }
    let _ = ws.close(None);
    Ok(())
}
