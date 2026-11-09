use super::*;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Kernel {
    stores: Mutex<HashMap<String, Value>>,
    commits: Mutex<Vec<(Generation, Vec<Value>)>>,
    revoked: Mutex<Vec<Generation>>,
    sealed: Mutex<Vec<Generation>>,
    refuse: AtomicBool,
    calls: AtomicUsize,
}
impl KernelHooks for Kernel {
    fn load_storage(&self, realm: &str) -> Result<Value> {
        Ok(self
            .stores
            .lock()
            .unwrap()
            .get(realm)
            .cloned()
            .unwrap_or(json!({})))
    }
    fn commit_activation(
        &self,
        _previous: Option<&Generation>,
        next: &Generation,
        ops: &[Value],
        storage: &Value,
    ) -> Result<()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.refuse.load(Ordering::Acquire) {
            return Err(Error::Host("Synthetic commit refusal".into()));
        }
        self.stores
            .lock()
            .unwrap()
            .insert(next.realm.clone(), storage.clone());
        if !ops.is_empty() {
            self.commits
                .lock()
                .unwrap()
                .push((next.clone(), ops.into()));
        }
        Ok(())
    }
    fn commit_retained(&self, generation: &Generation, ops: &[Value]) -> Result<()> {
        self.commits
            .lock()
            .unwrap()
            .push((generation.clone(), ops.into()));
        Ok(())
    }
    fn revoke_generation(&self, g: &Generation) {
        self.revoked.lock().unwrap().push(g.clone());
    }
    fn seal_open_gestures(&self, g: &Generation) {
        self.sealed.lock().unwrap().push(g.clone());
    }
}
#[derive(Default)]
struct Watch {
    notices: AtomicUsize,
    noticed_executions: Mutex<Vec<u64>>,
    prompts: AtomicUsize,
    stop: AtomicBool,
}
impl WatchdogHooks for Watch {
    fn notice(&self, _: &Generation, execution: u64) {
        self.noticed_executions.lock().unwrap().push(execution);
        self.notices.fetch_add(1, Ordering::Relaxed);
    }
    fn prompt(&self, _: &Generation, _: u64) -> WatchdogDecision {
        self.prompts.fetch_add(1, Ordering::Relaxed);
        if self.stop.load(Ordering::Relaxed) {
            WatchdogDecision::Stop
        } else {
            WatchdogDecision::Wait
        }
    }
}
#[derive(Default)]
struct Host {
    starts: AtomicUsize,
    cancels: Arc<AtomicUsize>,
    logs: Mutex<Vec<String>>,
    mode: AtomicUsize,
}
struct Request {
    deadline: Instant,
    mode: usize,
    cancels: Arc<AtomicUsize>,
    cancelled: bool,
}
impl HostRequest for Request {
    fn poll(&mut self) -> Option<Result<FetchResponse>> {
        if self.mode == 1 || Instant::now() < self.deadline {
            return None;
        }
        Some(Ok(FetchResponse {
            status: 200,
            url: "https://example.test/".into(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: if self.mode == 2 {
                vec![0; 10000]
            } else if self.mode == 3 {
                (0..6 * 1024 * 1024).map(|i| (i % 251) as u8).collect()
            } else {
                b"{\"ok\":true}".to_vec()
            },
        }))
    }
    fn cancel(&mut self) {
        if !self.cancelled {
            self.cancelled = true;
            self.cancels.fetch_add(1, Ordering::Relaxed);
        }
    }
}
impl HostProxy for Host {
    fn fetch(
        &self,
        _: &Generation,
        request: FetchRequest,
        _cancel: Cancellation,
    ) -> Result<Box<dyn HostRequest>> {
        assert!(request.deadline > Instant::now());
        self.starts.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(Request {
            deadline: Instant::now() + Duration::from_millis(10),
            mode: self.mode.load(Ordering::Relaxed),
            cancels: self.cancels.clone(),
            cancelled: false,
        }))
    }
    fn console(&self, _: &Generation, level: &str, message: &str) {
        self.logs.lock().unwrap().push(format!("{level}:{message}"));
    }
}
struct Fixture {
    pool: RuntimePool,
    kernel: Arc<Kernel>,
    host: Arc<Host>,
    watch: Arc<Watch>,
    admission: Admission,
}
fn fixture_with(change: impl FnOnce(&mut Config)) -> Fixture {
    let kernel = Arc::new(Kernel::default());
    let host = Arc::new(Host::default());
    let watch = Arc::new(Watch::default());
    let admission = Admission::new(1024 * 1024 * 1024);
    let mut config = Config {
        limits: Limits::default(),
        watchdog: WatchdogPolicy {
            notice: Duration::from_millis(30),
            prompt: Duration::from_millis(80),
            repeat: Duration::from_millis(40),
        },
        kernel: kernel.clone(),
        host: host.clone(),
        watchdog_hooks: watch.clone(),
        cache: CodeCache::new(4 * 1024 * 1024),
        engine: None,
        admission: admission.clone(),
    };
    change(&mut config);
    let admission = config.admission.clone();
    Fixture {
        pool: RuntimePool::new("synthetic-window", config).unwrap(),
        kernel,
        host,
        watch,
        admission,
    }
}
fn fixture() -> Fixture {
    fixture_with(|_| {})
}
/// What an isolate with default limits reserves when it starts.
fn slice() -> usize {
    crate::vm::HeapPlan::new(&Limits::default()).initial()
}
fn wait<T>(ticket: Ticket<T>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(result) = ticket.try_take() {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "Ticket {} timed out",
            ticket.execution
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn eval(realm: &Realm, source: &str) -> String {
    wait(realm.eval(source).unwrap()).unwrap().value
}
fn close(realm: &Realm) {
    realm.close();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !realm.is_disposed() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn restores_and_executes_real_v8() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(eval(&r, "__kind + ':' + (6*7)"), "compat:42");
    assert_eq!(
        eval(&r, "typeof process+':'+typeof document"),
        "undefined:undefined"
    );
    close(&r);
    assert_eq!(f.admission.used(), 0);
}
#[test]
fn promises_timers_and_microtasks() {
    let f = fixture();
    let r = f
        .pool
        .realm(RealmKind::V4Extension("clock".into()))
        .unwrap();
    assert_eq!(
        eval(
            &r,
            "(async()=>{let a=[];queueMicrotask(()=>a.push('micro'));await new Promise(resolve=>setTimeout(()=>{a.push('timer');resolve()},5));return a.join(',')})()"
        ),
        "micro,timer"
    );
    close(&r);
}

#[test]
fn cluster_routing_preserves_sync_closure_and_compat_identity() {
    let f = fixture();
    let a = f.pool.builtin("timeline", false).unwrap();
    let b = f.pool.builtin("inspector", false).unwrap();
    assert_eq!(
        eval(&a, "globalThis.liveProvider={value:19}; liveProvider.value"),
        "19"
    );
    assert_eq!(eval(&b, "liveProvider.value"), "19");
    for id in [
        "viewer",
        "toolbar",
        "captions",
        "layers-3d",
        "assets",
        "settings",
        "mods",
        "unknown-callback",
    ] {
        assert_eq!(builtin_cluster(id), RealmKind::BuiltinEditor);
    }
    let c = f.pool.builtin("timeline", true).unwrap();
    let d = f.pool.builtin("viewer", true).unwrap();
    eval(&c, "globalThis.compatOnly=23");
    assert_eq!(eval(&d, "compatOnly"), "23");
    assert_eq!(eval(&a, "typeof compatOnly"), "undefined");
    let rest = f.pool.builtin("effects", false).unwrap();
    assert_eq!(eval(&rest, "__kind"), "rest");
    close(&a);
    close(&c);
    close(&rest);
}
#[test]
fn one_isolate_per_extension_and_all_snapshot_kinds() {
    let f = fixture();
    let a = f.pool.realm(RealmKind::V4Extension("one".into())).unwrap();
    let b = f.pool.realm(RealmKind::V4Extension("two".into())).unwrap();
    eval(&a, "globalThis.privateValue=13");
    assert_eq!(eval(&b, "typeof privateValue"), "undefined");
    for (kind, name) in [
        (RealmKind::BuiltinAgent, "agent"),
        (RealmKind::Compiler, "compiler"),
    ] {
        let r = f.pool.realm(kind).unwrap();
        assert_eq!(eval(&r, "__kind"), name);
        close(&r);
    }
    assert!(f.pool.realm(RealmKind::Store("synthetic".into())).is_err());
    close(&a);
    close(&b);
}

#[test]
fn snapshot_reuse_child() {
    if std::env::var_os("PM_RUNTIME_SNAPSHOT_REUSE_CHILD").is_none() {
        return;
    }
    let f = fixture();
    // Finalize one custom compiler snapshot, retire its last isolate, then
    // restore it while another kind owns the shared read-only heap.
    let compiler = f.pool.realm(RealmKind::Compiler).unwrap();
    eval(&compiler, "__kind");
    close(&compiler);
    let extension = f
        .pool
        .realm(RealmKind::V4Extension("snapshot".into()))
        .unwrap();
    eval(&extension, "globalThis.answer=42");
    for _ in 0..3 {
        let compiler = f.pool.realm(RealmKind::Compiler).unwrap();
        assert_eq!(eval(&compiler, "typeof svelte.compile"), "function");
        close(&compiler);
        assert_eq!(eval(&extension, "answer"), "42");
    }
    close(&extension);
}

#[test]
fn compiler_snapshot_can_be_restored_after_a_different_kind_replaces_the_shared_heap() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::snapshot_reuse_child", "--nocapture"])
        .env("PM_RUNTIME_SNAPSHOT_REUSE_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{:?}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn locked_parking_survives_idle_and_cross_thread_submission() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    eval(&r, "globalThis.turn=0");
    for n in 1..=8 {
        let r = r.clone();
        thread::spawn(move || {
            assert_eq!(eval(&r, "++turn"), n.to_string());
        })
        .join()
        .unwrap();
        thread::sleep(Duration::from_millis(6));
    }
    assert_eq!(eval(&r, "turn"), "8");
    close(&r);
}
#[test]
fn code_cache_key_and_budget() {
    let base = CacheKey::new("v8", b"snap", b"bundle", "flags");
    for different in [
        CacheKey::new("v9", b"snap", b"bundle", "flags"),
        CacheKey::new("v8", b"snap2", b"bundle", "flags"),
        CacheKey::new("v8", b"snap", b"bundle2", "flags"),
        CacheKey::new("v8", b"snap", b"bundle", "flags2"),
        CacheKey::new("v", b"8snap", b"bundle", "flags"),
    ] {
        assert_ne!(base, different);
    }
    let cache = CodeCache::new(4);
    cache.insert(base, vec![1, 2]);
    assert_eq!(cache.get(base), Some(vec![1, 2]));
    cache.insert(base, vec![3]);
    assert_eq!(cache.get(base), Some(vec![3]));
    let other = CacheKey::new("other", b"", b"", "");
    cache.insert(other, vec![0; 4]);
    assert_eq!(cache.get(base), None);
    cache.insert(base, vec![0; 5]);
    assert_eq!(cache.get(base), None);
}
#[test]
fn real_code_cache_accepted_after_restore() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    let code = "function cacheScore(x){return x*7+1}; cacheScore(6)";
    assert_eq!(eval(&r, code), "43");
    let result = wait(r.eval(code).unwrap()).unwrap();
    assert!(result.cache_hits > 0);
    close(&r);
    let next = f.pool.realm(RealmKind::Compat).unwrap();
    let result = wait(next.eval(code).unwrap()).unwrap();
    assert_eq!(result.value, "43");
    assert!(result.cache_hits > 0);
    close(&next);
}
#[test]
fn utf8_encoding_streaming_bom_fatal_and_encode_into() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            r#"(()=>{let e=new TextEncoder(),d=new TextDecoder();let s='Aé😀\ud800';return d.decode(e.encode(s))==='Aé😀�'})()"#
        ),
        "true"
    );
    assert_eq!(
        eval(
            &r,
            "(()=>{let d=new TextDecoder();return d.decode(new Uint8Array([239,187]),{stream:true})+d.decode(new Uint8Array([191,240,159]),{stream:true})+d.decode(new Uint8Array([152,128]))})()"
        ),
        "😀"
    );
    assert_eq!(
        eval(
            &r,
            "new TextDecoder().decode(new Uint8Array([224,128,128,65,226,130]))"
        ),
        "���A�"
    );
    assert_eq!(
        eval(
            &r,
            "(()=>{try{new TextDecoder('utf8',{fatal:true}).decode(new Uint8Array([255]));return false}catch(e){return e instanceof TypeError}})()"
        ),
        "true"
    );
    assert_eq!(
        eval(
            &r,
            "JSON.stringify(new TextEncoder().encodeInto('😀x',new Uint8Array(4)))"
        ),
        "{\"read\":2,\"written\":4}"
    );
    close(&r);
}
#[test]
fn url_and_live_search_params() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "(()=>{let u=new URL('../a b?x=1&x=2','https://例え.test/root/page');let p=u.searchParams;p.append('é','a b');u.pathname='/other';return u.href+'|'+p.getAll('x').join(',')})()"
        ),
        "https://xn--r8jz45g.test/other?x=1&x=2&%C3%A9=a+b|1,2"
    );
    assert_eq!(
        eval(
            &r,
            "(()=>{let u=new URL('https://example.test:443/a');let p=u.searchParams;u.search='?q=a+b';p.set('q','x&y');return u.origin+'|'+u.href})()"
        ),
        "https://example.test|https://example.test/a?q=x%26y"
    );
    assert_eq!(eval(&r, "URL.canParse('bad relative')"), "false");
    close(&r);
}
#[test]
fn structured_clone_preserves_cycles_maps_typed_arrays_and_transfers() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "(()=>{let a={date:new Date(123),big:42n,map:new Map(),set:new Set([2]),bytes:new Uint8Array([7])};a.self=a;a.map.set(a,a.bytes);let b=structuredClone(a);return b!==a&&b.self===b&&b.date.getTime()===123&&b.big===42n&&b.map.get(b)===b.bytes&&b.bytes[0]===7&&b.set.has(2)})()"
        ),
        "true"
    );
    assert_eq!(
        eval(
            &r,
            "(()=>{let b=new Uint8Array([1,2,3]).buffer;let c=structuredClone({b},{transfer:[b]});return b.byteLength+':'+new Uint8Array(c.b)[2]})()"
        ),
        "0:3"
    );
    assert_eq!(
        eval(
            &r,
            "(()=>{try{structuredClone(()=>0);return false}catch(e){return e.name==='DataCloneError'}})()"
        ),
        "true"
    );
    close(&r);
}
/// A binary body reaches JS as its bytes: 6 MiB fits a 64 MiB isolate, where
/// a JSON number array of it would not.
#[test]
fn large_binary_bodies_arrive_as_bytes() {
    let f = fixture();
    f.host.mode.store(3, Ordering::Relaxed);
    let r = f
        .pool
        .realm(RealmKind::V4Extension("bytes".into()))
        .unwrap();
    assert_eq!(
        eval(
            &r,
            "(async()=>{const b=new Uint8Array(await (await fetch('https://example.test/big')).arrayBuffer());let ok=b.length===6*1024*1024;for(let i=0;i<b.length;i+=4099)ok=ok&&b[i]===i%251;return ok})()"
        ),
        "true"
    );
    close(&r);
}
#[test]
fn fetch_proxy_response_and_body_consumption() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "(async()=>{const r=await fetch('https://example.test/data',{method:'POST',headers:{'X-Test':'yes'},body:'é'});const c=r.clone();const v=await r.json();let used=false;try{await r.text()}catch{used=true}return r.ok&&r.headers.get('content-type')==='application/json'&&v.ok&&used&&(await c.text()).includes('true')})()"
        ),
        "true"
    );
    assert_eq!(f.host.starts.load(Ordering::Relaxed), 1);
    close(&r);
}
#[test]
fn abort_controller_cancels_proxy_and_dispatches_once() {
    let f = fixture();
    f.host.mode.store(1, Ordering::Relaxed);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "(async()=>{const c=new AbortController();let calls=0;c.signal.addEventListener('abort',()=>calls++,{once:true});const p=fetch('https://example.test/slow',{signal:c.signal});setTimeout(()=>{c.abort('reason');c.abort('later')},20);try{await p;return false}catch(e){return e==='reason'&&calls===1&&c.signal.aborted}})()"
        ),
        "true"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.host.cancels.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(3));
    }
    close(&r);
}
#[test]
fn preaborted_fetch_never_starts() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "fetch('https://example.test/',{signal:AbortSignal.abort('no')}).catch(e=>e)"
        ),
        "no"
    );
    assert_eq!(f.host.starts.load(Ordering::Relaxed), 0);
    close(&r);
}
#[test]
fn host_timeout_and_response_bounds() {
    let f = fixture_with(|c| {
        c.limits.host_timeout = Duration::from_millis(20);
        c.limits.max_response_bytes = 100;
    });
    f.host.mode.store(1, Ordering::Relaxed);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "fetch('https://example.test/').then(()=>false,e=>String(e).includes('timed out'))"
        ),
        "true"
    );
    assert!(f.host.cancels.load(Ordering::Relaxed) > 0);
    f.host.mode.store(2, Ordering::Relaxed);
    assert_eq!(
        eval(
            &r,
            "fetch('https://example.test/').then(()=>false,e=>String(e).includes('quota'))"
        ),
        "true"
    );
    close(&r);
}
#[test]
fn host_ops_timers_source_and_staging_are_bounded() {
    let f = fixture_with(|c| {
        c.limits.max_host_ops = 2;
        c.limits.max_ops = 2;
        c.limits.max_source_bytes = 1000;
    });
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert!(matches!(
        r.eval(" ".repeat(1001)),
        Err(Error::Quota("source bytes"))
    ));
    assert!(
        wait(
            r.eval("setTimeout(()=>0,10000);setTimeout(()=>0,10000);setTimeout(()=>0,10000)")
                .unwrap()
        )
        .is_err()
    );
    assert!(
        wait(
            r.eval("runtime.stage({id:1});runtime.stage({id:2});runtime.stage({id:3})")
                .unwrap()
        )
        .is_err()
    );
    assert!(f.kernel.commits.lock().unwrap().is_empty());
    close(&r);
}
#[test]
fn console_and_monotonic_performance() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(&r, "console.warn('hello',42); performance.now()>=0"),
        "true"
    );
    eval(
        &r,
        "new Promise(resolve=>setTimeout(()=>resolve(performance.now()>0),4))",
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.host.logs.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(3));
    }
    assert!(
        f.host
            .logs
            .lock()
            .unwrap()
            .contains(&"warn:hello 42".into())
    );
    close(&r);
}
#[test]
fn failed_reload_retains_old_state_ops_and_generation() {
    let f = fixture();
    let r = f
        .pool
        .realm(RealmKind::V4Extension("reload".into()))
        .unwrap();
    let first=wait(r.eval("storage.deep={n:7};hot.data.deep={n:11};globalThis.provider={n:19};runtime.stage({id:'old'})").unwrap()).unwrap();
    assert!(wait(r.reload("storage.deep.n=99;hot.data.deep.n=100;runtime.stage({id:'bad'});throw new Error('activation failed')").unwrap()).is_err());
    assert_eq!(
        eval(&r, "storage.deep.n+':'+hot.data.deep.n+':'+provider.n"),
        "7:11:19"
    );
    let next = wait(r.eval("42").unwrap()).unwrap();
    assert_eq!(first.generation, next.generation);
    assert!(
        !f.kernel
            .commits
            .lock()
            .unwrap()
            .iter()
            .any(|(_, ops)| ops.iter().any(|op| op["id"] == "bad"))
    );
    close(&r);
}
#[test]
fn async_reload_stages_until_success_and_retains_callable_history() {
    let f = fixture();
    let r = f
        .pool
        .realm(RealmKind::V4Extension("reload".into()))
        .unwrap();
    let first = wait(
        r.eval("storage.n=7;hot.data.n=11;globalThis.undo=()=>19")
            .unwrap(),
    )
    .unwrap();
    let pin = wait(r.pin().unwrap()).unwrap();
    let ticket=r.reload("(async()=>{storage.n++;hot.data.n++;runtime.stage({id:'new'});await new Promise(r=>setTimeout(r,25));globalThis.undo=()=>23;return storage.n+hot.data.n})()").unwrap();
    thread::sleep(Duration::from_millis(8));
    assert!(
        !f.kernel
            .commits
            .lock()
            .unwrap()
            .iter()
            .any(|(_, ops)| ops.iter().any(|op| op["id"] == "new"))
    );
    let next = wait(ticket).unwrap();
    assert_eq!(next.value, "20");
    assert_ne!(first.generation, next.generation);
    assert_eq!(eval(&r, "undo()"), "23");
    assert_eq!(
        wait(r.invoke_retained(&pin, "undo()").unwrap())
            .unwrap()
            .value,
        "19"
    );
    assert_eq!(eval(&r, "storage.n"), "8");
    assert!(f.kernel.revoked.lock().unwrap().contains(&first.generation));
    assert!(f.admission.used() >= 2 * slice());
    drop(pin);
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.admission.used() > slice() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(3));
    }
    close(&r);
}
#[test]
fn kernel_commit_refusal_keeps_previous_generation() {
    let f = fixture();
    let r = f
        .pool
        .realm(RealmKind::V4Extension("reload".into()))
        .unwrap();
    let old = wait(r.eval("storage.n=7").unwrap()).unwrap();
    f.kernel.refuse.store(true, Ordering::Release);
    assert!(wait(r.reload("storage.n=99;runtime.stage({id:'no'})").unwrap()).is_err());
    f.kernel.refuse.store(false, Ordering::Release);
    let after = wait(r.eval("storage.n").unwrap()).unwrap();
    assert_eq!(after.value, "7");
    assert_eq!(after.generation, old.generation);
    close(&r);
}
#[test]
fn compat_rejects_isolate_replacement_and_preserves_live_identity() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    eval(&r, "globalThis.provider={};globalThis.held=provider");
    assert!(wait(r.reload("42").unwrap()).is_err());
    assert_eq!(eval(&r, "held===provider"), "true");
    close(&r);
}
#[test]
fn watchdog_wait_then_explicit_stop_recreates_and_seals() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    let first = wait(
        r.eval("storage.persisted={n:19};globalThis.ephemeral=1")
            .unwrap(),
    )
    .unwrap();
    let infinite = r.eval("for(;;){}").unwrap();
    let id = infinite.execution;
    let deadline = Instant::now() + Duration::from_secs(3);
    while f.watch.prompts.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(3));
    }
    assert!(infinite.try_take().is_none());
    let active = r.active_execution().unwrap();
    assert_eq!(active.execution, id);
    assert_eq!(active.generation, first.generation);
    assert!(active.elapsed >= Duration::from_millis(30));
    assert!(r.stop(id));
    assert!(matches!(wait(infinite), Err(Error::Terminated)));
    let recovered = wait(r.eval("storage.persisted.n+':'+typeof ephemeral").unwrap()).unwrap();
    assert_eq!(recovered.value, "19:undefined");
    assert_ne!(first.generation, recovered.generation);
    assert!(!r.stop(id));
    assert!(f.watch.notices.load(Ordering::Relaxed) > 0);
    assert!(f.kernel.revoked.lock().unwrap().contains(&first.generation));
    assert!(f.kernel.sealed.lock().unwrap().contains(&first.generation));
    close(&r);
}
#[test]
fn watchdog_prompt_stop_and_microtask_runaway() {
    let f = fixture();
    f.watch.stop.store(true, Ordering::Relaxed);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert!(matches!(
        wait(r.eval("Promise.resolve().then(()=>{for(;;){}})").unwrap()),
        Err(Error::Terminated)
    ));
    assert_eq!(eval(&r, "6*7"), "42");
    close(&r);
}
#[test]
fn runaway_failed_activation_does_not_replace_active() {
    let f = fixture();
    f.watch.stop.store(true, Ordering::Relaxed);
    let r = f.pool.realm(RealmKind::V4Extension("loop".into())).unwrap();
    let old = wait(r.eval("storage.n=7").unwrap()).unwrap();
    assert!(matches!(
        wait(
            r.reload("storage.n=99;runtime.stage({id:'no'});for(;;){}")
                .unwrap()
        ),
        Err(Error::Terminated)
    ));
    let after = wait(r.eval("storage.n").unwrap()).unwrap();
    assert_eq!(after.value, "7");
    assert_eq!(after.generation, old.generation);
    close(&r);
}
#[test]
fn stop_cancels_host_requests_and_fences_generation() {
    let f = fixture();
    f.host.mode.store(1, Ordering::Relaxed);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    let old = wait(r.eval("storage.n=9").unwrap()).unwrap();
    let pending = r.eval("fetch('https://example.test/slow')").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while f.host.starts.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(r.stop(pending.execution));
    assert!(wait(pending).is_err());
    assert!(f.host.cancels.load(Ordering::Relaxed) > 0);
    assert!(matches!(
        wait(r.eval_generation("42", Some(old.generation)).unwrap()),
        Err(Error::StaleGeneration)
    ));
    close(&r);
}
#[test]
fn external_memory_quota_recreates_from_storage() {
    let f = fixture_with(|c| c.limits.external_bytes = 2 * 1024 * 1024);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    eval(&r, "storage.n=19");
    assert!(matches!(
        wait(
            r.eval("globalThis.bytes=new Uint8Array(3*1024*1024);42")
                .unwrap()
        ),
        Err(Error::Quota("external memory")) | Err(Error::Terminated)
    ));
    assert_eq!(eval(&r, "storage.n+':'+typeof bytes"), "19:undefined");
    close(&r);
}

#[test]
fn fetch_reservation_includes_buffers_allocated_in_the_same_js_stack() {
    let f = fixture_with(|c| {
        c.limits.external_bytes = 2 * 1024 * 1024;
        c.limits.max_response_bytes = 1024 * 1024;
    });
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "globalThis.bytes=new Uint8Array(1536*1024);fetch('https://example.test/').catch(e=>String(e).includes('reservation quota'))"
        ),
        "true"
    );
    assert_eq!(f.host.starts.load(Ordering::Relaxed), 0);
    let measured = wait(r.measure().unwrap()).unwrap();
    assert!(measured[0].memory.external_bytes >= 1536 * 1024);
    assert_eq!(measured[0].memory.fetch_reserved_bytes, 0);
    assert_eq!(eval(&r, "bytes.length"), "1572864");
    close(&r);
}

#[test]
fn pending_fetch_capacity_is_attributed_and_released_on_abort() {
    let f = fixture_with(|c| c.limits.max_response_bytes = 1024 * 1024);
    f.host.mode.store(1, Ordering::Relaxed);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    eval(
        &r,
        "globalThis.abort=new AbortController();globalThis.pending=fetch('https://example.test/',{signal:abort.signal}).catch(e=>e.name);'started'",
    );
    let measured = wait(r.measure().unwrap()).unwrap();
    assert_eq!(measured[0].memory.fetch_reserved_bytes, 1024 * 1024);
    assert_eq!(measured[0].memory.engine_reserved_bytes, 0);
    assert_eq!(eval(&r, "abort.abort();pending"), "AbortError");
    assert_eq!(
        wait(r.measure().unwrap()).unwrap()[0]
            .memory
            .fetch_reserved_bytes,
        0
    );
    assert_eq!(f.host.cancels.load(Ordering::Relaxed), 1);
    close(&r);
}

#[test]
fn abort_before_host_start_releases_capacity_even_when_js_throws() {
    let f = fixture_with(|c| c.limits.max_response_bytes = 1024 * 1024);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert!(matches!(wait(r.eval("const c=new AbortController();fetch('https://example.test/',{signal:c.signal}).catch(()=>{});c.abort();throw new Error('after abort')").unwrap()), Err(Error::JavaScript(_))));
    assert_eq!(f.host.starts.load(Ordering::Relaxed), 0);
    assert_eq!(
        wait(r.measure().unwrap()).unwrap()[0]
            .memory
            .fetch_reserved_bytes,
        0
    );
    assert_eq!(
        eval(&r, "fetch('https://example.test/').then(r=>r.text())"),
        "{\"ok\":true}"
    );
    close(&r);
}

#[test]
fn engine_reservation_counts_v8_and_pending_fetch_memory_before_calling_host() {
    let engine = Arc::new(Engine::default());
    let f = fixture_with(|c| {
        c.engine = Some(engine.clone());
        c.limits.external_bytes = 2 * 1024 * 1024;
        c.limits.max_response_bytes = 1024 * 1024;
        c.limits.max_engine_response_bytes = 1024 * 1024;
    });
    f.host.mode.store(1, Ordering::Relaxed);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    eval(
        &r,
        "globalThis.bytes=new Uint8Array(768*1024);globalThis.abort=new AbortController();globalThis.pending=fetch('https://example.test/',{signal:abort.signal}).catch(e=>e.name);'started'",
    );
    assert_eq!(
        eval(
            &r,
            "(()=>{try{runtime.engine('read')}catch(e){return String(e).includes('reservation quota')}})()"
        ),
        "true"
    );
    assert_eq!(engine.calls.load(Ordering::Relaxed), 0);
    eval(&r, "abort.abort();pending");
    assert_eq!(eval(&r, "runtime.engine('read').operation"), "read");
    assert_eq!(engine.calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        wait(r.measure().unwrap()).unwrap()[0]
            .memory
            .engine_reserved_bytes,
        0
    );
    close(&r);
}
#[test]
fn queued_fetches_wait_for_room_and_never_refuse_the_engine() {
    let engine = Arc::new(Engine::default());
    let f = fixture_with(|c| {
        c.engine = Some(engine.clone());
        c.limits.external_bytes = 4 * 1024 * 1024;
        c.limits.max_response_bytes = 1024 * 1024;
        c.limits.max_engine_response_bytes = 2 * 1024 * 1024;
        c.limits.queue_fetches = true;
    });
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    // Room for two fetches beside the engine's reply; twelve are asked for.
    eval(
        &r,
        "globalThis.all=Promise.all(Array.from({length:12},()=>fetch('https://example.test/').then(r=>r.json()).then(j=>j.ok)));'started'",
    );
    let measured = wait(r.measure().unwrap()).unwrap();
    assert!(measured[0].memory.fetch_reserved_bytes <= 2 * 1024 * 1024);
    // The synchronous engine channel is admitted while fetches wait.
    assert_eq!(eval(&r, "runtime.engine('read').operation"), "read");
    assert_eq!(eval(&r, "all.then(v=>v.every(Boolean)&&v.length)"), "12");
    assert_eq!(f.host.starts.load(Ordering::Relaxed), 12);
    assert_eq!(
        wait(r.measure().unwrap()).unwrap()[0]
            .memory
            .fetch_reserved_bytes,
        0
    );
    // An aborted waiting fetch leaves without ever starting.
    eval(
        &r,
        "globalThis.held=Array.from({length:2},()=>fetch('https://example.test/'));globalThis.c=new AbortController();globalThis.late=fetch('https://example.test/',{signal:c.signal}).catch(e=>e.name);c.abort();'ok'",
    );
    assert_eq!(eval(&r, "late"), "AbortError");
    eval(&r, "Promise.all(held).then(()=>'done')");
    assert_eq!(f.host.starts.load(Ordering::Relaxed), 14);
    close(&r);
}
#[test]
fn admission_refuses_then_releases_after_owner_disposal() {
    let f = fixture_with(|c| c.admission = Admission::new(slice()));
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert!(matches!(
        f.pool.realm(RealmKind::Compiler),
        Err(Error::Quota("isolate admission"))
    ));
    eval(&r, "42");
    close(&r);
    let next = f.pool.realm(RealmKind::Compiler).unwrap();
    assert_eq!(eval(&next, "42"), "42");
    close(&next);
}
#[test]
fn heap_quota_child() {
    if std::env::var_os("PM_RUNTIME_SYNTHETIC_HEAP_CHILD").is_none() {
        return;
    }
    let f = fixture_with(|c| {
        c.limits.heap_bytes = 16 * 1024 * 1024;
        c.watchdog.prompt = Duration::from_secs(2);
    });
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    eval(&r, "storage.n=19");
    assert!(matches!(
        wait(
            r.eval("globalThis.kept=[];for(;;)kept.push(new Array(65536).fill(123))")
                .unwrap()
        ),
        Err(Error::Quota("heap"))
    ));
    assert_eq!(eval(&r, "storage.n"), "19");
    close(&r);
}
#[test]
fn near_heap_callback_terminates_in_subprocess() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::heap_quota_child", "--nocapture"])
        .env("PM_RUNTIME_SYNTHETIC_HEAP_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn heaps_grow_from_their_slice_while_the_account_has_room_and_trim_when_idle() {
    let f = fixture();
    let r = f
        .pool
        .realm(RealmKind::V4Extension("grows".into()))
        .unwrap();
    eval(&r, "1");
    assert_eq!(f.admission.used(), slice());
    assert_eq!(f.admission.isolates(), 1);
    // 40 MiB that stays alive: past the 16 MiB slice, under the 64 MiB ceiling.
    eval(
        &r,
        "globalThis.kept=[];for(let i=0;i<40;i++)kept.push(new Array(131072).fill(i))",
    );
    let grown = f.admission.used();
    let measured = wait(r.measure().unwrap()).unwrap();
    assert_eq!(measured[0].memory.reserved_heap_bytes, grown);
    assert!(grown > slice() + 16 * 1024 * 1024, "{grown}");
    assert!(grown <= Limits::default().heap_bytes);
    // Freed and idle: the isolate gives back what it grew past what lives.
    eval(&r, "globalThis.kept=null");
    let deadline = Instant::now() + IDLE_WAIT;
    while f.admission.used() >= grown {
        assert!(
            Instant::now() < deadline,
            "{} never trimmed",
            f.admission.used()
        );
        thread::sleep(Duration::from_millis(20));
    }
    let trimmed = f.admission.used();
    assert_eq!(
        wait(r.measure().unwrap()).unwrap()[0]
            .memory
            .reserved_heap_bytes,
        trimmed
    );
    assert!(trimmed >= slice());
    // It grows again after a trim, and the account stays exact.
    eval(
        &r,
        "globalThis.kept=[];for(let i=0;i<40;i++)kept.push(new Array(131072).fill(i))",
    );
    assert!(f.admission.used() > trimmed);
    assert!(f.admission.peak() >= grown);
    close(&r);
    assert_eq!(f.admission.used(), 0);
    assert_eq!(f.admission.isolates(), 0);
}
const IDLE_WAIT: Duration = Duration::from_secs(20);
#[test]
fn full_account_child() {
    if std::env::var_os("PM_RUNTIME_SYNTHETIC_ACCOUNT_CHILD").is_none() {
        return;
    }
    // Room for two slices and 24 MiB of growth, shared by two isolates.
    let f = fixture_with(|c| c.admission = Admission::new(2 * slice() + 24 * 1024 * 1024));
    let quiet = f
        .pool
        .realm(RealmKind::V4Extension("quiet".into()))
        .unwrap();
    let hungry = f
        .pool
        .realm(RealmKind::V4Extension("hungry".into()))
        .unwrap();
    eval(&quiet, "storage.n=3");
    eval(&hungry, "storage.n=5");
    assert!(matches!(
        wait(
            hungry
                .eval("globalThis.kept=[];for(;;)kept.push(new Array(65536).fill(1))")
                .unwrap()
        ),
        Err(Error::Quota(HEAP_FULL))
    ));
    // Only that extension failed: it recovered from its storage, the other
    // kept running, and the account is back to the two slices.
    assert_eq!(eval(&hungry, "storage.n"), "5");
    assert_eq!(eval(&quiet, "storage.n"), "3");
    assert_eq!(f.admission.used(), 2 * slice());
    close(&hungry);
    close(&quiet);
    assert_eq!(f.admission.used(), 0);
}
/// Large single allocations: V8 admits one large young object whatever the
/// limit, so each case must still fail only its realm, never the process.
#[test]
fn large_allocation_child() {
    let Some(case) = std::env::var_os("PM_RUNTIME_SYNTHETIC_LARGE_CHILD") else {
        return;
    };
    let case = case.to_string_lossy().into_owned();
    let budget = if case == "account" {
        2 * slice() + 24 * 1024 * 1024
    } else {
        1024 * 1024 * 1024
    };
    let f = fixture_with(|c| {
        if case == "one" || case == "two" {
            c.limits.heap_bytes = 16 * 1024 * 1024;
            c.limits.initial_heap_bytes = 16 * 1024 * 1024;
        }
        c.admission = Admission::new(budget);
        c.watchdog.prompt = Duration::from_secs(2);
    });
    let at_rest = |f: &Fixture| f.admission.used();
    let quiet = f
        .pool
        .realm(RealmKind::V4Extension("quiet".into()))
        .unwrap();
    let big = f.pool.realm(RealmKind::V4Extension("big".into())).unwrap();
    eval(&quiet, "storage.n=3");
    eval(&big, "storage.n=5");
    let rest = at_rest(&f);
    let source = match case.as_str() {
        // 128 MiB in one array, far past a 16 MiB heap.
        "one" => "globalThis.a=new Array(32*1024*1024).fill(123);a.length",
        // Two of them retained.
        "two" => {
            "globalThis.a=new Array(32*1024*1024).fill(123);globalThis.b=new Array(32*1024*1024).fill(7);a.length+b.length"
        }
        // 32 MiB under a 64 MiB ceiling: past what a full account holds,
        // or within one with room.
        _ => "globalThis.a=new Array(8*1024*1024).fill(123);a.length",
    };
    let result = wait(big.eval(source).unwrap());
    if case == "fits" {
        // Room in its ceiling and the account: it grows to hold the array.
        assert_eq!(result.unwrap().value, "8388608");
        let m = &wait(big.measure().unwrap()).unwrap()[0].memory;
        assert!(m.used_heap_bytes >= 32 * 1024 * 1024);
        assert!(m.reserved_heap_bytes >= m.used_heap_bytes, "{m:?}");
        assert_eq!(f.admission.used() - rest, m.reserved_heap_bytes - slice());
        close(&big);
        close(&quiet);
        assert_eq!(f.admission.used(), 0);
        return;
    }
    let want = if case == "account" { HEAP_FULL } else { "heap" };
    assert!(
        matches!(result, Err(Error::Quota(q)) if q == want),
        "{case}: {result:?}"
    );
    assert_eq!(eval(&big, "storage.n"), "5");
    assert_eq!(eval(&quiet, "storage.n"), "3");
    assert_eq!(
        f.admission.used(),
        rest,
        "{case}: the failed generation's charge returns"
    );
    close(&big);
    close(&quiet);
    assert_eq!(f.admission.used(), 0);
}
#[test]
fn large_allocations_fail_their_realm_not_the_process() {
    for case in ["one", "two", "account", "fits"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::large_allocation_child", "--nocapture"])
            .env("PM_RUNTIME_SYNTHETIC_LARGE_CHILD", case)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{case}: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
#[test]
fn a_full_account_fails_one_isolate_not_the_process() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::full_account_child", "--nocapture"])
        .env("PM_RUNTIME_SYNTHETIC_ACCOUNT_CHILD", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn shutdown_is_nonblocking_and_completes_queued_tickets() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    let infinite = r.eval("for(;;){}").unwrap();
    let queued = r.eval("42").unwrap();
    let start = Instant::now();
    r.close();
    assert!(start.elapsed() < Duration::from_millis(20));
    assert!(wait(infinite).is_err());
    assert!(matches!(wait(queued), Err(Error::Closed)));
    close(&r);
    assert!(matches!(r.eval("42"), Err(Error::Closed)));
}
#[test]
fn watchdog_defaults_are_two_and_eight_seconds() {
    let p = WatchdogPolicy::default();
    assert_eq!(p.notice, Duration::from_secs(2));
    assert_eq!(p.prompt, Duration::from_secs(8));
}

#[derive(Default)]
struct Engine {
    calls: AtomicUsize,
    mode: AtomicUsize,
}
impl EngineProxy for Engine {
    fn call(
        &self,
        _: &Generation,
        operation: &str,
        input: &Value,
        budget: HostBudget,
    ) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.mode.load(Ordering::Relaxed) {
            1 => loop {
                budget.check()?;
                thread::sleep(Duration::from_millis(1));
            },
            2 => Ok(json!("x".repeat(1000))),
            3 => Ok(Value::Null),
            _ => Ok(json!({"operation":operation,"input":input,"owner":thread::current().name()})),
        }
    }
    fn metered(&self, _generation: &Generation, operation: &str) -> bool {
        operation != "host.poll"
    }
}
#[test]
fn engine_hook_runs_on_owner_and_preserves_arguments() {
    let engine = Arc::new(Engine::default());
    let f = fixture_with(|c| c.engine = Some(engine.clone()));
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "(()=>{const r=runtime.engine('bounds',{id:'layer-1',time:1.25});return r.operation+':'+r.input.id+':'+r.input.time+':'+r.owner})()"
        ),
        "bounds:layer-1:1.25:pm-v8-owner"
    );
    assert_eq!(engine.calls.load(Ordering::Relaxed), 1);
    close(&r);
}
#[test]
fn engine_hook_obeys_cancellation_timeout_and_byte_limits() {
    let engine = Arc::new(Engine::default());
    engine.mode.store(1, Ordering::Relaxed);
    let f = fixture_with(|c| {
        c.engine = Some(engine.clone());
        c.limits.host_timeout = Duration::from_millis(15);
        c.limits.max_engine_response_bytes = 100;
    });
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "(()=>{try{runtime.engine('slow')}catch(e){return String(e).includes('timed out')}})()"
        ),
        "true"
    );
    engine.mode.store(2, Ordering::Relaxed);
    assert_eq!(
        eval(
            &r,
            "(()=>{try{runtime.engine('large')}catch(e){return String(e).includes('quota')}})()"
        ),
        "true"
    );
    close(&r);
    engine.mode.store(1, Ordering::Relaxed);
    let f = fixture_with(|c| c.engine = Some(engine.clone()));
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    let before = engine.calls.load(Ordering::Relaxed);
    let pending = r.eval("runtime.engine('cancel')").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while engine.calls.load(Ordering::Relaxed) == before {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(r.stop(pending.execution));
    assert!(matches!(wait(pending), Err(Error::Terminated)));
    assert_eq!(eval(&r, "42"), "42");
    close(&r);
}
/// Unmetered queue plumbing (a host's poll) never exhausts the task's quota;
/// every other operation still does.
#[test]
fn unmetered_operations_do_not_spend_the_engine_operation_quota() {
    let engine = Arc::new(Engine::default());
    engine.mode.store(3, Ordering::Relaxed);
    let f = fixture_with(|c| {
        c.engine = Some(engine.clone());
        c.limits.max_host_ops = 4;
    });
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "(()=>{for(let i=0;i<100;i++)runtime.engine('host.poll',i);return 'ok'})()"
        ),
        "ok"
    );
    assert_eq!(
        eval(
            &r,
            "(()=>{try{for(let i=0;i<5;i++)runtime.engine('read',i);return 'no'}catch(e){return String(e)}})()"
        ),
        "TypeError: Engine operation quota"
    );
    close(&r);
}
#[test]
fn idle_realms_do_not_poll_or_commit_and_timers_wake_them() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    let initial = r.eval("42").unwrap();
    let initial_execution = initial.execution;
    assert_eq!(wait(initial).unwrap().value, "42");
    assert!(r.active_execution().is_none());
    let before = f.kernel.calls.load(Ordering::Relaxed);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(f.kernel.calls.load(Ordering::Relaxed), before);
    assert!(r.active_execution().is_none());
    // CPU contention may legitimately trigger the watchdog during the initial
    // task. Attribute notices by execution instead of calling that idle work.
    assert!(
        f.watch
            .noticed_executions
            .lock()
            .unwrap()
            .iter()
            .all(|id| *id == initial_execution)
    );
    eval(
        &r,
        "setTimeout(()=>{storage.timer=19;runtime.stage({id:'timer'})},15)",
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while !f
        .kernel
        .commits
        .lock()
        .unwrap()
        .iter()
        .any(|(_, ops)| ops.iter().any(|op| op["id"] == "timer"))
    {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(3));
    }
    assert_eq!(eval(&r, "storage.timer"), "19");
    close(&r);
}
#[test]
fn clear_interval_and_timer_arguments() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "new Promise(resolve=>{let n=0;const gone=setTimeout(()=>n=1000,1);clearTimeout(gone);const interval=setInterval(x=>{n+=x;if(n===6){clearInterval(interval);setTimeout(()=>resolve(n),8)}},2,2)})"
        ),
        "6"
    );
    close(&r);
}
#[test]
fn throwing_retained_history_stays_callable_and_cannot_clobber_active_storage() {
    let f = fixture();
    let r = f
        .pool
        .realm(RealmKind::V4Extension("history".into()))
        .unwrap();
    eval(&r, "storage.n=7;globalThis.undo=()=>19");
    let pin = wait(r.pin().unwrap()).unwrap();
    wait(r.reload("storage.n=23").unwrap()).unwrap();
    assert!(
        wait(
            r.invoke_retained(
                &pin,
                "storage.n=99;runtime.stage({id:'failed'});throw new Error('undo failure')"
            )
            .unwrap()
        )
        .is_err()
    );
    assert_eq!(
        wait(r.invoke_retained(&pin, "undo()").unwrap())
            .unwrap()
            .value,
        "19"
    );
    wait(
        r.invoke_retained(&pin, "storage.n=999;runtime.stage({id:'undo'})")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(eval(&r, "storage.n"), "23");
    assert!(
        f.kernel
            .commits
            .lock()
            .unwrap()
            .iter()
            .any(|(_, ops)| ops.iter().any(|op| op["id"] == "undo"))
    );
    close(&r);
}
#[test]
fn active_history_pin_failure_recovers_and_seals() {
    let f = fixture();
    f.watch.stop.store(true, Ordering::Relaxed);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    eval(&r, "storage.n=19");
    let pin = wait(r.pin().unwrap()).unwrap();
    assert!(matches!(
        wait(r.invoke_retained(&pin, "for(;;){}").unwrap()),
        Err(Error::Terminated)
    ));
    assert_eq!(eval(&r, "storage.n"), "19");
    assert!(f.kernel.sealed.lock().unwrap().contains(&pin.generation));
    close(&r);
}
#[test]
fn storage_restore_preserves_proto_keys_and_does_not_use_source_budget() {
    let f = fixture_with(|c| c.limits.max_source_bytes = 1000);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    let g = wait(r.eval("42").unwrap()).unwrap().generation;
    close(&r);
    f.kernel.stores.lock().unwrap().insert(
        g.realm.clone(),
        json!({"__proto__":{"authored":true},"large":"x".repeat(5000)}),
    );
    let next = f.pool.realm(RealmKind::Compat).unwrap();
    let value=wait(next.eval("Object.hasOwn(storage,'__proto__')+':'+storage.__proto__.authored+':'+storage.large.length").unwrap()).unwrap();
    assert_eq!(value.value, "true:true:5000");
    assert_ne!(value.generation, g);
    close(&next);
}
#[test]
fn no_script_runs_after_activation_is_committed() {
    let f = fixture();
    let r = f
        .pool
        .realm(RealmKind::V4Extension("atomic".into()))
        .unwrap();
    eval(&r, "storage.n=7");
    let before = f.kernel.calls.load(Ordering::Relaxed);
    // Corrupting the scheduling surface must fail before publication, rather than
    // returning failure after already installing candidate storage/registrations.
    assert!(wait(r.reload("storage.n=99;runtime.stage({id:'no'});globalThis.__pm={take:()=>JSON.stringify({ops:[{id:'no'}],storage:{n:99},hot:{}}),nextDelay(){throw new Error('schedule')}}").unwrap()).is_err());
    assert_eq!(f.kernel.calls.load(Ordering::Relaxed), before);
    assert_eq!(eval(&r, "storage.n"), "7");
    close(&r);
}
#[test]
fn queue_is_bounded_and_future_wakes() {
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Wake, Waker},
    };
    struct WakeCount(AtomicUsize);
    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let f = fixture_with(|c| c.limits.queue_capacity = 2);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    let infinite = r.eval("for(;;){}").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while f.watch.notices.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let a = r.eval("1").unwrap();
    let b = r.eval("2").unwrap();
    assert!(matches!(r.eval("3"), Err(Error::Busy)));
    assert!(r.stop(infinite.execution));
    assert!(wait(infinite).is_err());
    assert_eq!(wait(a).unwrap().value, "1");
    assert_eq!(wait(b).unwrap().value, "2");
    let mut ticket = r
        .eval("new Promise(resolve=>setTimeout(()=>resolve(19),20))")
        .unwrap();
    let wake = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker = Waker::from(wake.clone());
    let mut cx = Context::from_waker(&waker);
    assert!(Pin::new(&mut ticket).poll(&mut cx).is_pending());
    let deadline = Instant::now() + Duration::from_secs(3);
    while wake.0.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(matches!(
        Pin::new(&mut ticket).poll(&mut cx),
        std::task::Poll::Ready(Ok(_))
    ));
    close(&r);
}
#[test]
fn dropping_last_handle_disposes_parked_realm() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    eval(&r, "42");
    drop(r);
    let deadline = Instant::now() + Duration::from_secs(3);
    while f.admission.used() != 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn search_params_iterators_do_not_expose_internal_pairs() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "(()=>{let p=new URLSearchParams('x=1');let pair=p.entries().next().value;pair[1]='bad';let it=p.entries();it.next();p.append('y','2');return p.toString()+':'+it.next().value[0]})()"
        ),
        "x=1&y=2:y"
    );
    close(&r);
}

#[test]
fn external_interrupt_child() {
    if std::env::var_os("PM_RUNTIME_SYNTHETIC_EXTERNAL_CHILD").is_none() {
        return;
    }
    let f = fixture_with(|c| c.limits.external_bytes = 2 * 1024 * 1024);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    eval(&r, "storage.n=19");
    assert!(matches!(
        wait(
            r.eval(
                "globalThis.kept=[];for(;;){let b=new Uint8Array(256*1024);b.fill(1);kept.push(b)}"
            )
            .unwrap()
        ),
        Err(Error::Terminated) | Err(Error::Quota("external memory"))
    ));
    assert_eq!(eval(&r, "storage.n"), "19");
    close(&r);
}
#[test]
fn external_quota_interrupts_a_running_task_in_subprocess() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::external_interrupt_child", "--nocapture"])
        .env("PM_RUNTIME_SYNTHETIC_EXTERNAL_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn operation_budget_counts_utf8_bytes_and_native_boundary_revalidates() {
    let f = fixture_with(|c| c.limits.max_op_bytes = 100);
    let r = f
        .pool
        .realm(RealmKind::V4Extension("bytes".into()))
        .unwrap();
    assert_eq!(
        eval(
            &r,
            "(()=>{try{runtime.stage({text:'😀'.repeat(30)})}catch(e){return String(e).includes('quota')}})()"
        ),
        "true"
    );
    assert!(f.kernel.commits.lock().unwrap().is_empty());
    // The host independently validates even if extension JS replaces the staging facade.
    assert!(matches!(wait(r.reload("globalThis.__pm={take:()=>JSON.stringify({ops:[{text:'😀'.repeat(30)}],storage:{},hot:{}}),nextDelay:()=>null}").unwrap()),Err(Error::Quota("staged operation bytes/count"))));
    assert!(f.kernel.commits.lock().unwrap().is_empty());
    close(&r);
}
#[test]
fn clone_transfer_errors_keep_buffers_attached_and_url_prefixes_are_exact() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    assert_eq!(
        eval(
            &r,
            "(()=>{let b=new ArrayBuffer(4);try{structuredClone({b},{transfer:[b,b]})}catch(e){return e.name+':'+b.byteLength}})()"
        ),
        "DataCloneError:4"
    );
    assert_eq!(
        eval(
            &r,
            "(()=>{let u=new URL('https://example.test');u.search='??x=1';u.hash='##a';return u.search+'|'+u.hash})()"
        ),
        "??x=1|##a"
    );
    close(&r);
}

fn compile_wait(mut ticket: SvelteTicket) -> Result<SvelteOutput> {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(result) = ticket.try_take() {
            return result;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn svelte_snapshot_compiles_real_components_css_modules_and_reports_errors() {
    let f = fixture();
    let compiler = SvelteCompiler::new(&f.pool, 1024 * 1024, 1024 * 1024).unwrap();
    assert_eq!(eval(compiler.realm(), "svelte.VERSION"), SVELTE_VERSION);
    assert_eq!(
        eval(compiler.realm(), "typeof process+':'+typeof require"),
        "undefined:undefined"
    );
    let source = "<script lang='ts'>let count: number = $state(0)</script><button onclick={() => count++}>{count}</button><style>button{color:red}</style>";
    let first = compile_wait(
        compiler
            .compile(source, "Panel.svelte", SvelteUnit::Component)
            .unwrap(),
    )
    .unwrap();
    assert!(!first.cache_hit);
    assert!(first.javascript.contains("svelte/internal/client"));
    assert!(first.javascript.contains("powermove:styles"));
    assert!(
        first.compiled["css"]["code"]
            .as_str()
            .unwrap()
            .contains("red")
    );
    let second = compile_wait(
        compiler
            .compile(source, "Panel.svelte", SvelteUnit::Component)
            .unwrap(),
    )
    .unwrap();
    assert!(second.cache_hit);
    assert_eq!(first.javascript, second.javascript);
    let other = compile_wait(
        compiler
            .compile(source, "Other.svelte", SvelteUnit::Component)
            .unwrap(),
    )
    .unwrap();
    assert!(!other.cache_hit);
    let module = compile_wait(
        compiler
            .compile(
                "export const state = $state({n:1});",
                "state.svelte.js",
                SvelteUnit::Module,
            )
            .unwrap(),
    )
    .unwrap();
    assert!(module.javascript.contains("svelte/internal/client"));
    assert!(
        compile_wait(
            compiler
                .compile("<div>", "Broken.svelte", SvelteUnit::Component)
                .unwrap()
        )
        .is_err()
    );
    assert!(
        compiler
            .compile(
                &"x".repeat(1024 * 1024 + 1),
                "Huge.svelte",
                SvelteUnit::Component
            )
            .is_err()
    );
    close(compiler.realm());
}
#[test]
fn svelte_cache_is_version_source_options_sensitive_and_bounded() {
    let f = fixture();
    let compiler = SvelteCompiler::new(&f.pool, 1, 1024).unwrap();
    for _ in 0..2 {
        assert!(
            !compile_wait(
                compiler
                    .compile("<p>one</p>", "P.svelte", SvelteUnit::Component)
                    .unwrap()
            )
            .unwrap()
            .cache_hit
        );
    }
    assert_ne!(
        CacheKey::new("5.56.10", b"compiler", b"source", "options"),
        CacheKey::new("5.56.11", b"compiler", b"source", "options")
    );
    close(compiler.realm());
}
#[test]
fn reload_releases_all_effect_kinds_but_preserves_history_and_export_pins() {
    let f = fixture();
    let r = f
        .pool
        .realm(RealmKind::V4Extension("effects".into()))
        .unwrap();
    eval(
        &r,
        "globalThis.events=new EventTarget();globalThis.fired=0;events.addEventListener('test',()=>fired++);globalThis.ticks=0;setInterval(()=>ticks++,2);runtime.stage({kind:'registration',id:'old'});runtime.stage({kind:'gesture',id:'open'});globalThis.undo=()=>{events.dispatchEvent(new Event('test'));runtime.stage({kind:'undo'});return fired};globalThis.payload=new Uint8Array(1024*1024);payload[0]=37;globalThis.exportFrame=()=>payload[0]",
    );
    let history = wait(r.pin().unwrap()).unwrap();
    let export = wait(r.pin().unwrap()).unwrap();
    let old = history.generation.clone();
    wait(
        r.reload("runtime.stage({kind:'registration',id:'new'});globalThis.fresh=true")
            .unwrap(),
    )
    .unwrap();
    assert!(f.kernel.revoked.lock().unwrap().contains(&old));
    assert!(f.kernel.sealed.lock().unwrap().contains(&old));
    assert_eq!(
        wait(r.invoke_retained(&history, "undo()").unwrap())
            .unwrap()
            .value,
        "0"
    );
    let ticks = wait(r.invoke_retained(&history, "ticks").unwrap())
        .unwrap()
        .value;
    thread::sleep(Duration::from_millis(20));
    assert_eq!(
        wait(r.invoke_retained(&history, "ticks").unwrap())
            .unwrap()
            .value,
        ticks
    );
    assert!(wait(r.invoke_retained(&history, "setTimeout(()=>{},0)").unwrap()).is_err());
    assert_eq!(
        wait(r.invoke_retained(&export, "exportFrame()").unwrap())
            .unwrap()
            .value,
        "37"
    );
    let stats = wait(r.measure().unwrap()).unwrap();
    assert_eq!(stats.len(), 2);
    assert!(
        stats
            .iter()
            .any(|s| s.retired && s.memory.external_bytes >= 1024 * 1024)
    );
    drop(history);
    eval(&r, "1");
    assert_eq!(wait(r.measure().unwrap()).unwrap().len(), 2);
    drop(export);
    eval(&r, "1");
    assert_eq!(wait(r.measure().unwrap()).unwrap().len(), 1);
    close(&r);
    assert_eq!(f.admission.used(), 0);
}
#[test]
fn repeated_reload_accounts_for_history_retirement_and_failed_candidate_effects() {
    let f = fixture_with(|c| c.admission = Admission::new(3 * 1024 * 1024 * 1024));
    let r = f
        .pool
        .realm(RealmKind::V4Extension("waves".into()))
        .unwrap();
    let heap = slice();
    let mut history = Vec::new();
    for i in 0..20 {
        eval(
            &r,
            &format!(
                "globalThis.buf=new Uint8Array(1024*1024);buf[0]={i};globalThis.undo=()=>buf[0]"
            ),
        );
        history.push(wait(r.pin().unwrap()).unwrap());
        wait(r.reload("42").unwrap()).unwrap();
    }
    eprintln!(
        "RELOAD_MEASUREMENT reserved_peak_bytes={} generations=21",
        f.admission.used()
    );
    assert_eq!(f.admission.used(), 21 * heap);
    let stats = wait(r.measure().unwrap()).unwrap();
    assert_eq!(stats.len(), 21);
    eprintln!(
        "RELOAD_HEAPS used={} physical={} external={}",
        stats
            .iter()
            .map(|s| s.memory.used_heap_bytes)
            .sum::<usize>(),
        stats
            .iter()
            .map(|s| s.memory.physical_heap_bytes)
            .sum::<usize>(),
        stats.iter().map(|s| s.memory.external_bytes).sum::<usize>()
    );
    assert!(
        stats
            .iter()
            .filter(|s| s.retired)
            .map(|s| s.memory.external_bytes)
            .sum::<usize>()
            >= 20 * 1024 * 1024
    );
    for (i, pin) in history.iter().enumerate() {
        assert_eq!(
            wait(r.invoke_retained(pin, "undo()").unwrap())
                .unwrap()
                .value,
            i.to_string()
        );
    }
    let export = history[0].clone();
    history.clear();
    eval(&r, "1");
    assert_eq!(f.admission.used(), 2 * heap);
    drop(export);
    eval(&r, "1");
    assert_eq!(f.admission.used(), heap);
    assert!(wait(r.reload("setInterval(()=>console.log('leak'),1);const t=new EventTarget();t.addEventListener('x',()=>{});runtime.stage({kind:'registration',id:'failed'});throw Error('reject')").unwrap()).is_err());
    assert_eq!(f.admission.used(), heap);
    assert_eq!(wait(r.measure().unwrap()).unwrap().len(), 1);
    close(&r);
    assert_eq!(f.admission.used(), 0);
}

#[test]
fn native_typescript_module_path_preserves_runes_and_never_executes_source() {
    let f = fixture();
    let compiler = SvelteCompiler::new(&f.pool, 1024 * 1024, 4096).unwrap();
    let source = "import type {Missing} from './types'; interface Counter { n: number }; export const counter = $state<Counter>({n:3}); export function value(): number { return counter.n; }";
    let compiled = compile_wait(
        compiler
            .compile(source, "counter.svelte.ts", SvelteUnit::Module)
            .unwrap(),
    )
    .unwrap();
    assert!(compiled.javascript.contains("svelte/internal/client"));
    assert!(!compiled.javascript.contains("interface Counter"));
    assert!(!compiled.javascript.contains("import type"));
    assert!(compiled.javascript.contains("counter"));
    let compiled = compile_wait(
        compiler
            .compile(
                "throw Error('must not run');export let value: number = 9;",
                "safe.svelte.ts",
                SvelteUnit::Module,
            )
            .unwrap(),
    )
    .unwrap();
    assert!(compiled.javascript.contains("must not run"));
    assert!(
        compile_wait(
            compiler
                .compile(
                    "export const value: = ;",
                    "bad.svelte.ts",
                    SvelteUnit::Module
                )
                .unwrap()
        )
        .is_err()
    );
    let error = compile_wait(
        compiler
            .compile("<div>", "bad.svelte", SvelteUnit::Component)
            .unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(error,Error::JavaScript(ref text) if text.contains("element_unclosed") && text.contains("start"))
    );
    let stats = wait(compiler.realm().measure().unwrap()).unwrap();
    assert_eq!(stats.len(), 1);
    assert!(stats[0].memory.snapshot_bytes > 1024 * 1024);
    eprintln!("COMPILER_MEASUREMENT {:?}", stats[0].memory);
    close(compiler.realm());
}
#[test]
fn last_history_pin_retires_idle_generation_without_another_submission() {
    let f = fixture();
    let r = f
        .pool
        .realm(RealmKind::V4Extension("idle-history".into()))
        .unwrap();
    eval(&r, "globalThis.undo=()=>42");
    let pin = wait(r.pin().unwrap()).unwrap();
    wait(r.reload("1").unwrap()).unwrap();
    assert_eq!(f.admission.used(), 2 * slice());
    drop(pin);
    let deadline = Instant::now() + Duration::from_secs(5);
    while f.admission.used() != slice() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    close(&r);
}
#[test]
fn reload_cleanup_does_not_depend_on_mutable_control_globals() {
    let f = fixture();
    let r = f
        .pool
        .realm(RealmKind::V4Extension("private-cleanup".into()))
        .unwrap();
    eval(
        &r,
        "globalThis.events=new EventTarget();globalThis.fired=0;events.addEventListener('test',()=>fired++);globalThis.undo=()=>{events.dispatchEvent(new Event('test'));return fired}",
    );
    let pin = wait(r.pin().unwrap()).unwrap();
    eval(
        &r,
        "globalThis.__pm={take:()=>JSON.stringify({ops:[],storage:{},hot:{}}),nextDelay:()=>null,retire(){throw Error('replaced global')}}",
    );
    wait(r.reload("2").unwrap()).unwrap();
    assert_eq!(
        wait(r.invoke_retained(&pin, "undo()").unwrap())
            .unwrap()
            .value,
        "0"
    );
    drop(pin);
    close(&r);
}

#[test]
fn module_graphs_keep_esm_semantics_cycles_and_dynamic_import() {
    let f = fixture();
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    let graph = json!({
        "entry": "pmext://a/1/index",
        "sources": {
            "pmext://a/1/index": "import {count,bump} from './counter';import {late} from './cycle';export default function(){bump();return [count,late()]};export const lazy=()=>import('./later').then(m=>m.value);",
            "pmext://a/1/counter": "export let count=0;export function bump(){count++}",
            "pmext://a/1/cycle": "import {late as again} from './cycle-b';export const late=()=>again();",
            "pmext://a/1/cycle-b": "import {late as first} from './cycle';export const late=()=>typeof first;",
            "pmext://a/1/later": "export const value=await Promise.resolve(41+1);"
        },
        "edges": {
            "pmext://a/1/index": {"./counter":"pmext://a/1/counter","./cycle":"pmext://a/1/cycle","./later":"pmext://a/1/later"},
            "pmext://a/1/cycle": {"./cycle-b":"pmext://a/1/cycle-b"},
            "pmext://a/1/cycle-b": {"./cycle":"pmext://a/1/cycle"}
        }
    });
    let source = format!(
        "(async()=>{{const [done,ns]=__linkModules({});await done;return JSON.stringify([ns.default(),await ns.lazy()])}})()",
        json!(graph.to_string())
    );
    assert_eq!(eval(&r, &source), r#"[[1,"function"],42]"#);
    let thrown = json!({
        "entry": "pmext://b/1/index",
        "sources": {"pmext://b/1/index": "throw new Error('synthetic')"},
        "edges": {}
    });
    let source = format!(
        "(async()=>{{const [done]=__linkModules({});try{{await done}}catch(e){{return e.message}}}})()",
        json!(thrown.to_string())
    );
    assert_eq!(eval(&r, &source), "synthetic");
    close(&r);
}
