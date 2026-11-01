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
    prompts: AtomicUsize,
    stop: AtomicBool,
}
impl WatchdogHooks for Watch {
    fn notice(&self, _: &Generation, _: u64) {
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
        admission: admission.clone(),
    };
    change(&mut config);
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
        (RealmKind::Store("synthetic".into()), "store"),
    ] {
        let r = f.pool.realm(kind).unwrap();
        assert_eq!(eval(&r, "__kind"), name);
        close(&r);
    }
    close(&a);
    close(&b);
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
    assert!(f.admission.used() >= 128 * 1024 * 1024);
    drop(pin);
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.admission.used() > 64 * 1024 * 1024 {
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
fn admission_refuses_then_releases_after_owner_disposal() {
    let f = fixture_with(|c| c.admission = Admission::new(64 * 1024 * 1024));
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
    let f = fixture_with(|c| c.limits.heap_bytes = 16 * 1024 * 1024);
    let r = f.pool.realm(RealmKind::Compat).unwrap();
    eval(&r, "storage.n=19");
    assert!(matches!(
        wait(
            r.eval("globalThis.kept=[];for(;;)kept.push(new Array(65536).fill(123))")
                .unwrap()
        ),
        Err(Error::Quota("heap")) | Err(Error::Terminated)
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
