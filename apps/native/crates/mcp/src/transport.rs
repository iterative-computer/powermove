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
    time::Duration,
};
const MAX_LINE: usize = 2 * 1024 * 1024;

pub trait ToolConnection {
    fn list(&mut self) -> Result<Value>;
    fn invoke(&mut self, name: &str, args: Value) -> Result<Value>;
    fn disconnect(&mut self) -> Result<()>;
}
impl ToolConnection for McpSession {
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
                let args = m["params"].get("arguments").cloned().unwrap_or(json!({}));
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
    session.disconnect()
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
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let token = random_hex()?;
        let instance = pm_kernel::new_id("mcp-instance");
        let directory = profile.join("mcp");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        let file = directory.join("connection.json");
        if file.exists() {
            return Err(Error::invalid(
                "MCP descriptor already exists; stop its owner before starting another host",
            ));
        }
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
        if std::fs::read(&self.file)
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
pub fn proxy_stdio(profile: &Path) -> Result<()> {
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
    write_json(
        &mut stream,
        &json!({"id":0,"method":"connect","token":token}),
    )?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let response: Value = serde_json::from_slice(
        &line(&mut reader)?.ok_or_else(|| Error::invalid("MCP host closed"))?,
    )
    .map_err(|_| Error::invalid("Invalid MCP response"))?;
    if response["result"]["connected"] != true {
        return Err(Error::invalid("MCP host rejected connection"));
    }
    serve_stdio(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        Bridge {
            stream,
            reader,
            next: 1,
        },
    )
}
struct Bridge {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    next: u64,
}
impl Bridge {
    fn request(&mut self, method: &str, name: Option<&str>, args: Value) -> Result<Value> {
        let id = self.next;
        self.next += 1;
        let mut request = json!({"id":id,"method":method});
        if let Some(name) = name {
            request["name"] = json!(name);
            request["arguments"] = args;
        }
        write_json(&mut self.stream, &request)?;
        let response: Value =
            serde_json::from_slice(&line(&mut self.reader)?.ok_or_else(|| {
                Error::invalid("MCP host disconnected; inspect before repeating edits")
            })?)
            .map_err(|_| Error::invalid("Invalid MCP response"))?;
        if response["id"] != id {
            return Err(Error::invalid("MCP response id mismatch"));
        }
        if let Some(e) = response.get("error") {
            return Err(Error::invalid(e.as_str().unwrap_or("MCP host error")));
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
        self.stream.shutdown(std::net::Shutdown::Both)?;
        Ok(())
    }
}
