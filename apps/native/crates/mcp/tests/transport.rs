use pm_mcp::{
    Result,
    transport::{LoopbackServer, ToolConnection},
};
use serde_json::{Value, json};
use std::{
    fs,
    net::{Ipv4Addr, TcpListener},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    process::Command,
    sync::{Arc, Barrier},
};

struct Connection;
impl ToolConnection for Connection {
    fn list(&mut self) -> Result<Value> {
        Ok(json!([]))
    }
    fn invoke(&mut self, _: &str, _: Value) -> Result<Value> {
        Ok(json!({}))
    }
    fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }
}

struct Profile(PathBuf);
impl Profile {
    fn new() -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("pm-mcp-stale-test"));
        fs::create_dir_all(root.join("mcp")).unwrap();
        // macOS temp roots can include system-owned symlinks.
        Self(root.canonicalize().unwrap())
    }
    fn descriptor(&self) -> PathBuf {
        self.0.join("mcp/connection.json")
    }
    fn write(&self, bytes: &[u8]) {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(self.descriptor())
            .unwrap();
        file.write_all(bytes).unwrap();
    }
    fn archives(&self) -> Vec<PathBuf> {
        fs::read_dir(self.0.join("mcp"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("connection.stale-")
            })
            .collect()
    }
    fn start(&self) -> Result<LoopbackServer> {
        LoopbackServer::start_with(&self.0, || Connection)
    }
    fn assert_recovered(&self, original: &[u8]) {
        use std::os::unix::fs::PermissionsExt;
        let mut server = self.start().unwrap();
        let archives = self.archives();
        assert_eq!(archives.len(), 1);
        assert_eq!(fs::read(&archives[0]).unwrap(), original);
        assert_eq!(
            fs::metadata(&archives[0]).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let current: Value = serde_json::from_slice(&fs::read(self.descriptor()).unwrap()).unwrap();
        assert_eq!(current["pid"], std::process::id());
        assert_ne!(fs::read(self.descriptor()).unwrap(), original);
        server.shutdown();
        assert!(!self.descriptor().exists());
        assert_eq!(fs::read(&archives[0]).unwrap(), original);
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn descriptor(pid: u32, port: u16) -> Vec<u8> {
    serde_json::to_vec(&json!({"version":1,"instanceId":"synthetic-old-instance",
        "pid":pid,"port":port,"token":"0".repeat(64)}))
    .unwrap()
}
fn exited_pid() -> u32 {
    let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
    let pid = child.id();
    assert!(child.wait().unwrap().success());
    pid
}

#[test]
fn stale_pid_is_archived_even_when_port_is_listening() {
    let profile = Profile::new();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let original = descriptor(exited_pid(), listener.local_addr().unwrap().port());
    profile.write(&original);
    profile.assert_recovered(&original);
}

#[test]
fn live_current_pid_and_listening_port_are_preserved() {
    let profile = Profile::new();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let original = descriptor(std::process::id(), listener.local_addr().unwrap().port());
    profile.write(&original);
    let error = profile.start().err().expect("a live owner must be refused");
    assert!(error.message.contains("MCP descriptor already exists"));
    assert_eq!(fs::read(profile.descriptor()).unwrap(), original);
    assert!(profile.archives().is_empty());
}

#[test]
fn refused_port_is_archived_even_when_pid_is_alive() {
    let profile = Profile::new();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let original = descriptor(std::process::id(), listener.local_addr().unwrap().port());
    drop(listener);
    profile.write(&original);
    profile.assert_recovered(&original);
}

#[test]
fn corrupt_json_is_archived_byte_for_byte() {
    let profile = Profile::new();
    let original = b"{torn synthetic descriptor\xff";
    profile.write(original);
    profile.assert_recovered(original);
}

#[test]
fn unusable_json_shapes_are_archived_without_resetting_the_profile() {
    for original in [b"null".as_slice(), b"{}", b"{\"pid\":-1,\"port\":65536}"] {
        let profile = Profile::new();
        let sentinel = profile.0.join("synthetic-user-data");
        fs::write(&sentinel, b"preserve synthetic data").unwrap();
        profile.write(original);
        profile.assert_recovered(original);
        assert_eq!(fs::read(sentinel).unwrap(), b"preserve synthetic data");
    }
}

#[test]
fn invalid_pid_never_probes_a_process_group_or_evicts_a_listening_host() {
    let profile = Profile::new();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    for pid in [json!(0), json!(-1), json!(u64::MAX), json!("invalid")] {
        let original =
            serde_json::to_vec(&json!({"pid":pid,"port":listener.local_addr().unwrap().port()}))
                .unwrap();
        profile.write(&original);
        assert!(profile.start().is_err());
        assert_eq!(fs::read(profile.descriptor()).unwrap(), original);
        assert!(profile.archives().is_empty());
    }
}

#[test]
fn live_pid_with_invalid_port_is_preserved() {
    let profile = Profile::new();
    for port in [json!(0), json!(65536), Value::Null] {
        let original = serde_json::to_vec(&json!({"pid":std::process::id(),"port":port})).unwrap();
        profile.write(&original);
        assert!(profile.start().is_err());
        assert_eq!(fs::read(profile.descriptor()).unwrap(), original);
        assert!(profile.archives().is_empty());
    }
}

#[test]
fn repeated_corrupt_recovery_keeps_all_archives() {
    let profile = Profile::new();
    for original in [
        b"first torn record".as_slice(),
        b"second torn record".as_slice(),
    ] {
        profile.write(original);
        profile.start().unwrap().shutdown();
    }
    let mut contents: Vec<_> = profile
        .archives()
        .iter()
        .map(|p| fs::read(p).unwrap())
        .collect();
    contents.sort();
    assert_eq!(
        contents,
        [
            b"first torn record".to_vec(),
            b"second torn record".to_vec()
        ]
    );
}

#[test]
fn concurrent_recovery_publishes_one_host_and_one_archive() {
    let profile = Profile::new();
    let original = b"torn synthetic descriptor";
    profile.write(original);
    let barrier = Arc::new(Barrier::new(8));
    let starters: Vec<_> = (0..8)
        .map(|_| {
            let barrier = barrier.clone();
            let root = profile.0.clone();
            std::thread::spawn(move || {
                barrier.wait();
                LoopbackServer::start_with(&root, || Connection)
            })
        })
        .collect();
    let servers: Vec<_> = starters
        .into_iter()
        .map(|s| s.join().unwrap())
        .filter_map(std::result::Result::ok)
        .collect();
    assert_eq!(servers.len(), 1);
    assert_eq!(profile.archives().len(), 1);
    assert_eq!(fs::read(&profile.archives()[0]).unwrap(), original);
    assert!(profile.descriptor().exists());
    drop(servers);
    assert!(!profile.descriptor().exists());
}
