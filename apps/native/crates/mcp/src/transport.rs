//! Newline-delimited JSON-RPC stdio and the shipped authenticated IPv4
//! loopback bridge ({id,method,name,arguments}, discovered by connection.json).
use crate::{Automation, Error, McpSession, Result, catalog, failure};
use serde_json::{Value, json};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const MAX_LINE: usize = 2 * 1024 * 1024;

fn descriptor_lock(directory: &Path) -> std::io::Result<std::fs::File> {
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(directory.join("connection.lock"))?;
    // Serialize recovery and publication across processes. The OS releases this
    // lock on a crash; leave the lock file in place so all starters lock one inode.
    lock.lock()?;
    Ok(lock)
}

fn pid_is_dead(pid: i32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    // SAFETY: a positive pid and signal zero only probe existence. ESRCH is 3
    // on both supported hosts (Darwin and Linux). EPERM/unknown errors are live.
    unsafe { kill(pid, 0) == -1 && std::io::Error::last_os_error().raw_os_error() == Some(3) }
}

fn descriptor_is_stale(bytes: &[u8]) -> bool {
    let Ok(descriptor) = serde_json::from_slice::<Value>(bytes) else {
        return true;
    };
    let pid = descriptor["pid"]
        .as_u64()
        .and_then(|n| i32::try_from(n).ok())
        .filter(|pid| *pid > 0);
    if pid.is_some_and(pid_is_dead) {
        return true;
    }
    let port = descriptor["port"]
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .filter(|port| *port > 0);
    if let Some(port) = port {
        let address = (std::net::Ipv4Addr::LOCALHOST, port).into();
        // N3/N7: only an explicit refusal proves a recorded endpoint is gone.
        // Timeouts and other uncertain failures must not evict a possible owner.
        return TcpStream::connect_timeout(&address, Duration::from_millis(250))
            .is_err_and(|e| e.kind() == std::io::ErrorKind::ConnectionRefused);
    }
    // An unusable record with neither owner nor endpoint cannot serve clients.
    // A recognizable live/unknown pid is preserved even if its port is corrupt.
    pid.is_none()
}

fn recover_descriptor(file: &Path) -> Result<()> {
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if !descriptor_is_stale(&bytes) {
        return Err(Error::invalid(
            "MCP descriptor already exists; stop its owner before starting another host",
        ));
    }
    let directory = file.parent().expect("descriptor directory");
    let mut timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    loop {
        let archive = directory.join(format!("connection.stale-{timestamp}.json"));
        // N4: preserve the exact bytes, including corrupt records, and never
        // overwrite a prior archive even if the clock repeats or moves backwards.
        match std::fs::hard_link(file, archive) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => timestamp += 1,
            Err(e) => return Err(e.into()),
        }
    }
    std::fs::File::open(directory)?.sync_all()?;
    std::fs::remove_file(file)?;
    Ok(())
}

pub trait ToolConnection {
    /// The grant token of the provider run whose tool server this is, if any.
    fn bind_agent_run(&mut self, _token: &str) {}
    fn list(&mut self) -> Result<Value>;
    fn invoke(&mut self, name: &str, args: Value) -> Result<Value>;
    fn disconnect(&mut self) -> Result<()>;
}
impl ToolConnection for McpSession {
    fn bind_agent_run(&mut self, run: &str) {
        self.set_agent_run(run);
    }
    fn list(&mut self) -> Result<Value> {
        Ok(self.tools())
    }
    fn invoke(&mut self, name: &str, args: Value) -> Result<Value> {
        Ok(self.call(name, args))
    }
    fn disconnect(&mut self) -> Result<()> {
        self.close()
    }
}
pub struct Rpc<T: ToolConnection = McpSession> {
    session: T,
    initialized: bool,
}
impl<T: ToolConnection> Rpc<T> {
    pub fn new(session: T) -> Self {
        Self {
            session,
            initialized: false,
        }
    }
    pub fn handle(&mut self, m: Value) -> Option<Value> {
        let error = |id: Value, code: i32, message: &str| {
            Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
        };
        if !m.is_object() || m["jsonrpc"] != "2.0" || !m["method"].is_string() {
            return error(
                m.get("id").cloned().unwrap_or(Value::Null),
                -32600,
                "Invalid Request",
            );
        }
        let id = m.get("id")?.clone();
        if !id.is_string() && !id.is_number() {
            return error(Value::Null, -32600, "Invalid request id");
        }
        let result = match m["method"].as_str().unwrap() {
            "initialize" => {
                if self.initialized {
                    return error(id, -32600, "Already initialized");
                }
                self.initialized = true;
                let versions = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
                let version = m["params"]["protocolVersion"]
                    .as_str()
                    .filter(|v| versions.contains(v))
                    .unwrap_or(versions[0]);
                json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"powermove","version":"1.0.0"},"instructions":catalog::catalog()["instructions"]})
            }
            "ping" => json!({}),
            _ if !self.initialized => {
                return error(id, -32000, "Initialize Powermove before using tools");
            }
            "tools/list" => match self.session.list() {
                Ok(tools) => json!({"tools":tools}),
                Err(e) => return error(id, -32603, &e.message),
            },
            "tools/call" => {
                let args = m["params"]
                    .get("arguments")
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or(json!({}));
                if !args.is_object() || !m["params"]["name"].is_string() {
                    return error(id, -32602, "Invalid tool arguments");
                }
                match self
                    .session
                    .invoke(m["params"]["name"].as_str().unwrap(), args)
                {
                    Ok(v) => v,
                    Err(e) => failure(e),
                }
            }
            _ => return error(id, -32601, "Method not found"),
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
    pub fn close(&mut self) -> Result<()> {
        self.session.disconnect()
    }
}
fn line(reader: &mut impl BufRead) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = vec![];
    let n = reader
        .take((MAX_LINE + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if n == 0 {
        return Ok(None);
    }
    if bytes.len() > MAX_LINE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "MCP request exceeds 2 MiB",
        ));
    }
    Ok(Some(bytes))
}
fn write_json(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    writer.write_all(&bytes)?;
    writer.flush()
}
pub fn serve_stdio<T: ToolConnection>(
    reader: impl BufRead,
    mut writer: impl Write,
    session: T,
) -> Result<()> {
    let mut reader = reader;
    let mut rpc = Rpc::new(session);
    loop {
        let bytes = match line(&mut reader) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => break,
            Err(e) => {
                write_json(
                    &mut writer,
                    &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"MCP request exceeds 2 MiB"}}),
                )?;
                return Err(e.into());
            }
        };
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let response = match serde_json::from_slice(&bytes) {
            Ok(m) => rpc.handle(m),
            Err(_) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
            ),
        };
        if let Some(value) = response {
            write_json(&mut writer, &value)?;
        }
    }
    rpc.close()
}
fn random_hex() -> Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn constant_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}
fn connection<T: ToolConnection>(
    mut stream: TcpStream,
    token: String,
    mut session: T,
) -> Result<()> {
    // macOS accepted sockets inherit the listener's nonblocking flag.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut authorized = false;
    let outcome = (|| -> Result<()> {
        while let Some(bytes) = line(&mut reader)? {
            if bytes.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let m: Value = match serde_json::from_slice(&bytes) {
                Ok(m) => m,
                Err(_) => {
                    write_json(&mut stream, &json!({"id":null,"error":"Parse error"}))?;
                    if !authorized {
                        break;
                    }
                    continue;
                }
            };
            let id = m.get("id").cloned().unwrap_or(Value::Null);
            if !authorized {
                if m["method"] != "connect"
                    || !m["token"].as_str().is_some_and(|t| constant_eq(t, &token))
                {
                    write_json(
                        &mut stream,
                        &json!({"id":id,"error":"Unauthorized Powermove MCP connection."}),
                    )?;
                    break;
                }
                authorized = true;
                if let Some(run) = m["agentRun"].as_str() {
                    session.bind_agent_run(run);
                }
                reader.get_ref().set_read_timeout(None)?;
                write_json(&mut stream, &json!({"id":id,"result":{"connected":true}}))?;
                continue;
            }
            let reply = match m["method"].as_str() {
                Some("tools/list") => json!({"id":id,"result":session.list()?}),
                Some("tools/call") => {
                    if let (Some(name), true) = (m["name"].as_str(), m["arguments"].is_object()) {
                        json!({"id":id,"result":session.invoke(name,m["arguments"].clone())?})
                    } else {
                        json!({"id":id,"error":"Invalid tool arguments."})
                    }
                }
                _ => json!({"id":id,"error":"Unknown MCP bridge method."}),
            };
            write_json(&mut stream, &reply)?;
        }
        Ok(())
    })();
    let cleanup = session.disconnect();
    match outcome {
        Err(primary) => Err(primary),
        Ok(()) => cleanup,
    }
}
/// Lifetime guard. Explicit shutdown joins clients, rolls their sessions back,
/// and removes only this instance's descriptor. It never logs the credential.
pub struct LoopbackServer {
    stop: Arc<AtomicBool>,
    clients: Arc<Mutex<std::collections::HashMap<u64, TcpStream>>>,
    worker: Option<thread::JoinHandle<()>>,
    file: PathBuf,
    instance: String,
}
impl LoopbackServer {
    pub fn start(profile: &Path, host: Arc<Mutex<Automation>>) -> Result<Self> {
        Self::start_with(profile, move || McpSession::new(host.clone()))
    }
    /// Bind an existing application actor without introducing another kernel.
    /// The factory and all connection operations run exclusively on workers.
    pub fn start_with<T: ToolConnection + Send + 'static>(
        profile: &Path,
        factory: impl Fn() -> T + Send + Sync + 'static,
    ) -> Result<Self> {
        let factory = Arc::new(factory);
        let token = random_hex()?;
        let instance = pm_kernel::new_id("mcp-instance");
        let directory = profile.join("mcp");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        let file = directory.join("connection.json");
        let _publication = descriptor_lock(&directory)?;
        recover_descriptor(&file)?;
        // Probe the old endpoint before choosing ours: the OS may reuse its port.
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let descriptor = json!({"version":1,"instanceId":instance,"pid":std::process::id(),"port":listener.local_addr()?.port(),"token":token});
        let temp = directory.join(format!("connection-{instance}.tmp"));
        let mut fd = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        fd.write_all(descriptor.to_string().as_bytes())?;
        fd.sync_all()?;
        // hard_link atomically refuses an existing descriptor (two starters).
        if let Err(e) = std::fs::hard_link(&temp, &file) {
            let _ = std::fs::remove_file(temp);
            return Err(e.into());
        }
        std::fs::remove_file(temp)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let clients = Arc::new(Mutex::new(
            std::collections::HashMap::<u64, TcpStream>::new(),
        ));
        let sockets = clients.clone();
        let worker = thread::spawn(move || {
            let mut children: Vec<thread::JoinHandle<()>> = vec![];
            let mut next = 0u64;
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let Ok(copy) = stream.try_clone() else {
                            continue;
                        };
                        next += 1;
                        let client_id = next;
                        sockets.lock().unwrap().insert(client_id, copy);
                        let active_sockets = sockets.clone();
                        children.retain(|child| !child.is_finished());
                        let token = token.clone();
                        let factory = factory.clone();
                        children.push(thread::spawn(move || {
                            if let Err(e) = connection(stream, token, factory()) {
                                eprintln!("MCP connection ended: {e}");
                            }
                            active_sockets.lock().unwrap().remove(&client_id);
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::park_timeout(Duration::from_millis(10))
                    }
                    Err(e) => {
                        eprintln!("MCP listener: {e}");
                        break;
                    }
                }
            }
            for stream in sockets.lock().unwrap().values() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            for child in children {
                let _ = child.join();
            }
        });
        Ok(Self {
            stop,
            clients,
            worker: Some(worker),
            file,
            instance,
        })
    }
    pub fn descriptor_path(&self) -> &Path {
        &self.file
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
        self.clients.lock().unwrap().clear();
        if let Ok(_publication) = descriptor_lock(self.file.parent().expect("descriptor directory"))
            && std::fs::read(&self.file)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .is_some_and(|v| v["instanceId"] == self.instance)
        {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}
impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}
/// Client mode for --powermove-mcp, matching the shipped proxy. Native profiles
/// are explicit. Never discover or start the Electron host implicitly.
/// `agent_run`: the grant token of the provider run whose tool server this is
/// (`--agent-run`); only that run's process is given it.
pub fn proxy_stdio(profile: &Path, agent_run: Option<String>) -> Result<()> {
    serve_stdio(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        Bridge::open(profile, agent_run)?,
    )
}
fn bridge_channel(
    profile: &Path,
    agent_run: Option<&str>,
) -> Result<(TcpStream, BufReader<TcpStream>)> {
    let descriptor: Value =
        serde_json::from_slice(&std::fs::read(profile.join("mcp/connection.json"))?)
            .map_err(|_| Error::invalid("Invalid MCP descriptor"))?;
    let port = descriptor["port"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 65535)
        .ok_or_else(|| Error::invalid("Invalid MCP port"))? as u16;
    let token = descriptor["token"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| Error::invalid("Invalid MCP credential"))?;
    if descriptor["version"] != 1 {
        return Err(Error::invalid("Invalid MCP descriptor version"));
    }
    let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))?;
    let mut connect = json!({"id":0,"method":"connect","token":token});
    if let Some(run) = agent_run {
        connect["agentRun"] = json!(run);
    }
    write_json(&mut stream, &connect)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let response: Value = serde_json::from_slice(
        &line(&mut reader)?.ok_or_else(|| Error::invalid("MCP host closed"))?,
    )
    .map_err(|_| Error::invalid("Invalid MCP response"))?;
    if response["result"]["connected"] != true {
        return Err(Error::invalid("MCP host rejected connection"));
    }
    Ok((stream, reader))
}
struct Bridge {
    profile: PathBuf,
    agent_run: Option<String>,
    channel: Option<(TcpStream, BufReader<TcpStream>)>,
    next: u64,
}
impl Bridge {
    fn open(profile: &Path, agent_run: Option<String>) -> Result<Self> {
        Ok(Self {
            profile: profile.to_path_buf(),
            channel: Some(bridge_channel(profile, agent_run.as_deref())?),
            agent_run,
            next: 1,
        })
    }
    fn request(&mut self, method: &str, name: Option<&str>, args: Value) -> Result<Value> {
        let id = self.next;
        self.next += 1;
        let mut request = json!({"id":id,"method":method});
        if let Some(name) = name {
            request["name"] = json!(name);
            request["arguments"] = args;
        }
        // Reconnect only on a later explicit request. An uncertain mutation is
        // never replayed; a fresh channel creates a fresh server-side session.
        if self.channel.is_none() {
            self.channel = Some(bridge_channel(&self.profile, self.agent_run.as_deref())?);
        }
        let outcome = (|| {
            let (stream, reader) = self.channel.as_mut().expect("connected channel");
            write_json(stream, &request)?;
            let response: Value = serde_json::from_slice(&line(reader)?.ok_or_else(|| {
                Error::invalid("MCP host disconnected; inspect before repeating edits")
            })?)
            .map_err(|_| Error::invalid("Invalid MCP response"))?;
            if response["id"] != id {
                return Err(Error::invalid("MCP response id mismatch"));
            }
            Ok(response)
        })();
        if outcome.is_err()
            && let Some((stream, _)) = self.channel.take()
        {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        let response = outcome.map_err(|mut error: Error| {
            error.message=format!("{}; MCP host outcome is unknown. Inspect before repeating edits. The failed request was not replayed.",error.message);
            error
        })?;
        // A well-formed host refusal has a known outcome. Keep its exact source
        // message and the live session; only transport/protocol loss is uncertain.
        if let Some(error) = response.get("error") {
            return Err(Error::invalid(error.as_str().unwrap_or("MCP host error")));
        }
        Ok(response["result"].clone())
    }
}
impl ToolConnection for Bridge {
    fn list(&mut self) -> Result<Value> {
        self.request("tools/list", None, json!({}))
    }
    fn invoke(&mut self, name: &str, args: Value) -> Result<Value> {
        self.request("tools/call", Some(name), args)
    }
    fn disconnect(&mut self) -> Result<()> {
        if let Some((stream, _)) = self.channel.take() {
            stream.shutdown(std::net::Shutdown::Both)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod teardown_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    struct Failing {
        closed: Arc<AtomicUsize>,
    }
    impl ToolConnection for Failing {
        fn list(&mut self) -> Result<Value> {
            Ok(json!([]))
        }
        fn invoke(&mut self, _: &str, _: Value) -> Result<Value> {
            Err(Error::invalid("primary tool failure"))
        }
        fn disconnect(&mut self) -> Result<()> {
            self.closed.fetch_add(1, Ordering::SeqCst);
            Err(Error::invalid("secondary teardown failure"))
        }
    }
    #[test]
    fn tool_failure_still_disconnects_and_preserves_primary_error() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let closed = Arc::new(AtomicUsize::new(0));
        let owned = closed.clone();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            connection(stream, "synthetic".into(), Failing { closed: owned })
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write_json(
            &mut client,
            &json!({"id":0,"method":"connect","token":"synthetic"}),
        )
        .unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        line(&mut reader).unwrap().unwrap();
        write_json(
            &mut client,
            &json!({"id":1,"method":"tools/call","name":"mutate","arguments":{}}),
        )
        .unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap_err().message,
            "primary tool failure"
        );
        assert_eq!(closed.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn explicit_host_refusal_preserves_message_and_connection() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let first: Value =
                serde_json::from_slice(&line(&mut reader).unwrap().unwrap()).unwrap();
            write_json(
                &mut stream,
                &json!({"id":first["id"],"error":"source refusal"}),
            )
            .unwrap();
            let second: Value =
                serde_json::from_slice(&line(&mut reader).unwrap().unwrap()).unwrap();
            write_json(
                &mut stream,
                &json!({"id":second["id"],"result":{"ok":true}}),
            )
            .unwrap();
        });
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let reader = BufReader::new(stream.try_clone().unwrap());
        let mut bridge = Bridge {
            profile: PathBuf::new(),
            agent_run: None,
            channel: Some((stream, reader)),
            next: 1,
        };
        assert_eq!(
            bridge.invoke("mutate", json!({})).unwrap_err().message,
            "source refusal"
        );
        assert!(bridge.channel.is_some());
        assert_eq!(bridge.list().unwrap(), json!({"ok":true}));
        worker.join().unwrap();
    }
}
