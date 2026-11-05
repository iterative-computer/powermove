use pm_runtime_v8::*;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Default)]
struct Kernel {
    storage: Mutex<HashMap<String, Value>>,
    revoked: Mutex<Vec<Generation>>,
    sealed: Mutex<Vec<Generation>>,
    commits: AtomicUsize,
}
impl KernelHooks for Kernel {
    fn load_storage(&self, realm: &str) -> Result<Value> {
        Ok(self
            .storage
            .lock()
            .unwrap()
            .get(realm)
            .cloned()
            .unwrap_or(json!({})))
    }
    fn commit_activation(
        &self,
        _: Option<&Generation>,
        g: &Generation,
        _: &[Value],
        s: &Value,
    ) -> Result<()> {
        self.storage
            .lock()
            .unwrap()
            .insert(g.realm.clone(), s.clone());
        self.commits.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn commit_retained(&self, _: &Generation, _: &[Value]) -> Result<()> {
        panic!("no unleased history")
    }
    fn revoke_generation(&self, g: &Generation) {
        self.revoked.lock().unwrap().push(g.clone());
    }
    fn seal_open_gestures(&self, g: &Generation) {
        self.sealed.lock().unwrap().push(g.clone());
    }
}
#[derive(Default)]
struct Host {
    cancelled: Arc<AtomicUsize>,
}
struct Request {
    cancelled: Arc<AtomicUsize>,
    never: bool,
}
impl HostRequest for Request {
    fn poll(&mut self) -> Option<Result<FetchResponse>> {
        if self.never {
            None
        } else {
            Some(Ok(FetchResponse {
                status: 200,
                url: "https://example.test/".into(),
                headers: vec![],
                body: b"fixture".to_vec(),
            }))
        }
    }
    fn cancel(&mut self) {
        self.cancelled.fetch_add(1, Ordering::Relaxed);
    }
}
impl HostProxy for Host {
    fn fetch(
        &self,
        _: &Generation,
        r: FetchRequest,
        _: Cancellation,
    ) -> Result<Box<dyn HostRequest>> {
        Ok(Box::new(Request {
            cancelled: self.cancelled.clone(),
            never: r.url.contains("never"),
        }))
    }
    fn console(&self, _: &Generation, _: &str, _: &str) {}
}
struct Watch;
impl WatchdogHooks for Watch {
    fn notice(&self, _: &Generation, _: u64) {}
    fn prompt(&self, _: &Generation, _: u64) -> WatchdogDecision {
        WatchdogDecision::Stop
    }
}
struct Engine;
impl EngineProxy for Engine {
    fn call(&self, _: &Generation, op: &str, input: &Value, budget: HostBudget) -> Result<Value> {
        budget.check()?;
        // Preserve the fixture's exact string with serde_json's preserve_order
        // feature either enabled or disabled by the surrounding workspace.
        Ok(json!({"input":input,"op":op}))
    }
}
fn config() -> (Config, Arc<Kernel>, Arc<Host>) {
    let kernel = Arc::new(Kernel::default());
    let host = Arc::new(Host::default());
    (
        Config {
            limits: Limits::default(),
            watchdog: WatchdogPolicy::default(),
            kernel: kernel.clone(),
            host: host.clone(),
            watchdog_hooks: Arc::new(Watch),
            cache: CodeCache::new(1024 * 1024),
            engine: Some(Arc::new(Engine)),
            admission: Admission::new(1024 * 1024 * 1024),
        },
        kernel,
        host,
    )
}
fn wait<T>(t: Ticket<T>) -> Result<T> {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(v) = t.try_take() {
            return v;
        }
        assert!(Instant::now() < end, "ticket timeout");
        thread::sleep(Duration::from_millis(2));
    }
}
fn until(mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
}
fn executable() -> std::path::PathBuf {
    // Verification may run freshly signed private copies of both executables.
    std::env::var_os("PM_STORE_HELPER")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_pm-store-helper").into())
        .into()
}
#[test]
fn production_helpers_are_cluster_scoped_confined_and_reaped_without_losing_storage() {
    let (config, kernel, _) = config();
    let parent = RuntimePool::new("trusted", config.clone()).unwrap();
    let trusted = parent.realm(RealmKind::Compat).unwrap();
    wait(trusted.eval("globalThis.answer=42").unwrap()).unwrap();
    let stores = StorePool::new(executable(), "store-window", config).unwrap();
    let a = stores.helper("a").unwrap();
    let same = stores.helper("a").unwrap();
    let b = stores.helper("b").unwrap();
    let first=wait(a.eval("storage.persisted=37;globalThis.privateValue=19;runtime.stage({kind:'register',id:'command'});37").unwrap()).unwrap();
    assert_eq!(first.value, "37");
    assert_eq!(
        wait(same.eval("privateValue").unwrap()).unwrap().value,
        "19"
    );
    assert_eq!(
        wait(b.eval("typeof privateValue").unwrap()).unwrap().value,
        "undefined"
    );
    assert_eq!(a.status().pid, same.status().pid);
    assert_ne!(a.status().pid, b.status().pid);
    #[cfg(target_os = "linux")]
    assert_eq!(a.status().confinement["seccomp"], true);
    assert!(a.status().memory["used"].as_u64().unwrap() > 0);
    assert!(a.status().resident_bytes.unwrap_or(1) > 0);
    eprintln!(
        "STORE_MEASUREMENT {}",
        json!({"rss_bytes":a.status().resident_bytes,"v8":a.status().memory,"executable_bytes":std::fs::metadata(executable()).unwrap().len(),"confinement":a.status().confinement})
    );
    a.close();
    until(|| kernel.sealed.lock().unwrap().contains(&first.generation));
    assert!(kernel.revoked.lock().unwrap().contains(&first.generation));
    let replacement = stores.helper("a").unwrap();
    let reopened = wait(replacement.eval("storage.persisted").unwrap()).unwrap();
    assert_eq!(reopened.value, "37");
    assert_ne!(first.generation, reopened.generation);
    assert_eq!(wait(trusted.eval("answer").unwrap()).unwrap().value, "42");
    b.close();
    replacement.close();
    trusted.close();
}
#[test]
fn helper_bridges_engine_fetch_abort_and_idle_timer_effects() {
    let (config, kernel, host) = config();
    let stores = StorePool::new(executable(), "io-window", config).unwrap();
    let h = stores.helper("io").unwrap();
    assert_eq!(
        wait(
            h.eval("JSON.stringify(runtime.engine('read',{id:4}))")
                .unwrap()
        )
        .unwrap()
        .value,
        "{\"input\":{\"id\":4},\"op\":\"read\"}"
    );
    assert_eq!(
        wait(
            h.eval("fetch('https://example.test/').then(r=>r.text())")
                .unwrap()
        )
        .unwrap()
        .value,
        "fixture"
    );
    wait(h.eval("globalThis.abort=new AbortController();globalThis.pending=fetch('https://example.test/never',{signal:abort.signal}).catch(e=>e.name);'started'").unwrap()).unwrap();
    assert_eq!(
        wait(h.eval("abort.abort();pending").unwrap())
            .unwrap()
            .value,
        "AbortError"
    );
    until(|| host.cancelled.load(Ordering::Relaxed) > 0);
    wait(h.eval("setTimeout(()=>storage.timer=9,20)").unwrap()).unwrap();
    until(|| {
        kernel
            .storage
            .lock()
            .unwrap()
            .values()
            .any(|v| v["timer"] == 9)
    });
    h.close();
}
#[test]
fn helper_stop_and_spawn_failure_complete_tickets() {
    let (config, _, _) = config();
    let stores = StorePool::new(executable(), "stop-window", config.clone()).unwrap();
    let h = stores.helper("hang").unwrap();
    wait(h.eval("1").unwrap()).unwrap();
    let hung = h.eval("for(;;){}").unwrap();
    let queued = h.eval("42").unwrap();
    h.close();
    assert!(wait(hung).is_err());
    assert!(wait(queued).is_err());
    let bad = StorePool::new("/nonexistent/pm-helper", "bad-window", config)
        .unwrap()
        .helper("x")
        .unwrap();
    assert!(wait(bad.eval("1").unwrap()).is_err());
}
struct FatalSandbox;
impl StoreSandbox for FatalSandbox {
    fn confine(&self) -> Result<Value> {
        PlatformStoreSandbox.confine()?;
        // Deliberately omit the production near-limit callback. This is an actual
        // fatal V8 allocation in the confined helper, not kill()/a simulated error.
        let mut isolate =
            v8::Isolate::new(v8::CreateParams::default().heap_limits(0, 16 * 1024 * 1024));
        v8::scope!(let s,&mut isolate);
        let c = v8::Context::new(s, Default::default());
        let s = &mut v8::ContextScope::new(s, c);
        let source=v8::String::new(s,"globalThis.keep=[];for(;;)keep.push(Array.from({length:65536},(_,i)=>({i,value:'fatal oom fixture'})))").unwrap();
        v8::Script::compile(s, source, None).unwrap().run(s);
        panic!("fatal V8 OOM must not return")
    }
}
#[test]
fn fatal_oom_child() {
    if std::env::var_os("PM_STORE_FATAL_TEST").is_some() {
        let _ = run_store_helper(&FatalSandbox);
        panic!("fatal helper returned");
    }
}
#[test]
fn forced_fatal_oom_only_kills_the_confined_helper() {
    use std::os::unix::process::ExitStatusExt;
    let (config, _, _) = config();
    let pool = RuntimePool::new("fatal-parent", config.clone()).unwrap();
    let parent = pool.realm(RealmKind::Compat).unwrap();
    wait(parent.eval("globalThis.answer=73").unwrap()).unwrap();
    let stores = StorePool::new(executable(), "fatal-sibling", config).unwrap();
    let sibling = stores.helper("survivor").unwrap();
    wait(sibling.eval("globalThis.answer=91").unwrap()).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fatal_oom_child", "--nocapture"])
        .env("PM_STORE_FATAL_TEST", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let init = json!({"v":1,"kind":"init","scope":"fatal-child","cluster":"fatal","limits":{"heap":64*1024*1024,"external":32*1024*1024,"source":8*1024*1024,"host_ops":64,"op_bytes":1024*1024,"ops":4096,"response":8*1024*1024,"timeout_ms":30000,"queue":64}});
    let bytes = serde_json::to_vec(&init).unwrap();
    let mut input = child.stdin.take().unwrap();
    input
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .unwrap();
    input.write_all(&bytes).unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        matches!(output.status.signal(), Some(libc::SIGABRT | libc::SIGTRAP)),
        "{:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("heap out of memory")
            || stderr.contains("Reached heap limit")
            || stderr.contains("Fatal JavaScript out of memory:"),
        "{stderr}"
    );
    assert_eq!(wait(parent.eval("answer").unwrap()).unwrap().value, "73");
    assert_eq!(wait(sibling.eval("answer").unwrap()).unwrap().value, "91");
    parent.close();
    sibling.close();
}
struct ProbeSandbox {
    root: std::path::PathBuf,
    address: std::net::SocketAddr,
}
fn require_denied(result: std::io::Result<impl Sized>, operation: &str) {
    match result {
        Err(error) if matches!(error.raw_os_error(), Some(libc::EPERM | libc::EACCES)) => {}
        Err(error) => panic!("{operation}: expected EPERM/EACCES, got {error}"),
        Ok(_) => panic!("{operation}: confinement allowed the operation"),
    }
}
impl StoreSandbox for ProbeSandbox {
    fn confine(&self) -> Result<Value> {
        let confinement = PlatformStoreSandbox.confine()?;
        require_denied(std::fs::read(self.root.join("readable")), "file read");
        require_denied(
            std::fs::write(self.root.join("forbidden"), b"fixture"),
            "file write",
        );
        require_denied(
            std::net::TcpStream::connect(self.address),
            "network connect",
        );
        require_denied(
            std::process::Command::new("/bin/sh")
                .args(["-c", "exit 77"])
                .status(),
            "process spawn",
        );
        // Same pinned optimization bits as S1. Native syntax is enabled only by
        // the test child before V8 initialization, never the production helper.
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let s, &mut isolate);
        let context = v8::Context::new(s, Default::default());
        let s = &mut v8::ContextScope::new(s, context);
        let source = v8::String::new(
            s,
            r#"
            function optimized(x) { return x * 3 + 1; }
            %PrepareFunctionForOptimization(optimized);
            for (let i = 0; i < 100; i++) optimized(i);
            %OptimizeFunctionOnNextCall(optimized);
            if (optimized(14) !== 43) throw new Error('wrong JIT value');
            const status = %GetOptimizationStatus(optimized);
            if ((status & 8) === 0 || (status & 32) === 0)
                throw new Error('TurboFan not established: ' + status);
            status;
        "#,
        )
        .unwrap();
        let status = v8::Script::compile(s, source, None)
            .expect("compile JIT probe")
            .run(s)
            .expect("confined TurboFan probe")
            .integer_value(s)
            .unwrap();
        eprintln!("CONFINEMENT {confinement} TURBOFAN_STATUS {status}");
        std::process::exit(0)
    }
}
#[test]
fn confinement_child() {
    if std::env::var_os("PM_STORE_CONFINEMENT_TEST").is_some() {
        let root = std::path::PathBuf::from(std::env::var_os("PM_STORE_PROBE_ROOT").unwrap());
        let address = std::env::var("PM_STORE_PROBE_ADDRESS")
            .unwrap()
            .parse()
            .unwrap();
        // Prove these operations work before confinement. ENOENT or a closed
        // port is never accepted as evidence of a sandbox denial.
        assert_eq!(
            std::fs::read(root.join("readable")).unwrap(),
            b"synthetic probe"
        );
        std::fs::write(root.join("writable"), b"synthetic probe").unwrap();
        std::net::TcpStream::connect(address).unwrap();
        assert_eq!(
            std::process::Command::new("/bin/sh")
                .args(["-c", "exit 77"])
                .status()
                .unwrap()
                .code(),
            Some(77)
        );
        v8::V8::set_flags_from_string("--allow-natives-syntax");
        let _ = run_store_helper(&ProbeSandbox { root, address });
        panic!("probe returned");
    }
}
#[test]
fn platform_confinement_denies_files_network_and_spawn_and_preserves_jit() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "pm-runtime-denial-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("readable"), b"synthetic probe").unwrap();
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "confinement_child", "--nocapture"])
        .env("PM_STORE_CONFINEMENT_TEST", "1")
        .env("PM_STORE_PROBE_ROOT", &root)
        .env(
            "PM_STORE_PROBE_ADDRESS",
            listener.local_addr().unwrap().to_string(),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let init = json!({"v":1,"kind":"init","scope":"probe","cluster":"probe","limits":{"heap":64*1024*1024,"external":32*1024*1024,"source":8*1024*1024,"host_ops":64,"op_bytes":1024*1024,"ops":4096,"response":8*1024*1024,"timeout_ms":30000,"queue":64}});
    let bytes = serde_json::to_vec(&init).unwrap();
    let mut input = child.stdin.take().unwrap();
    input
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .unwrap();
    input.write_all(&bytes).unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    let denied_file_exists = root.join("forbidden").exists();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let evidence = String::from_utf8_lossy(&output.stderr);
    assert!(evidence.contains("TURBOFAN_STATUS"), "{evidence}");
    #[cfg(target_os = "macos")]
    assert!(evidence.contains("\"seatbelt\":true"), "{evidence}");
    #[cfg(target_os = "linux")]
    assert!(evidence.contains("\"seccomp\":true"), "{evidence}");
    assert!(!denied_file_exists, "sandbox allowed file creation");
    eprintln!("{evidence}");
}

#[test]
fn helper_admission_bounds_pending_and_active_commands() {
    let (mut config, _, _) = config();
    config.limits.queue_capacity = 2;
    let stores = StorePool::new(executable(), "admission-window", config.clone()).unwrap();
    assert!(stores.helper("").is_err());
    let helper = stores.helper("bounded").unwrap();
    wait(helper.eval("1").unwrap()).unwrap();
    let hung = helper.eval("for(;;){}").unwrap();
    let queued = helper.eval("42").unwrap();
    assert!(matches!(helper.eval("43"), Err(Error::Busy)));
    helper.close();
    assert!(wait(hung).is_err());
    assert!(wait(queued).is_err());
    config.limits.max_source_bytes = usize::MAX;
    assert!(StorePool::new(executable(), "invalid-limits", config).is_err());
}
