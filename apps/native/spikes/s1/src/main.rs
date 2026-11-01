//! Synthetic architecture probes, not a production extension runtime.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    ffi::{CString, c_void},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{net::UnixStream, process::ExitStatusExt},
    },
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
unsafe extern "C" {
    fn v8__V8__IsSandboxEnabled() -> bool;
}
const MIB: usize = 1024 * 1024;
const FLAGS: &str = "--random-seed=42 --allow-natives-syntax";
fn emit(v: Value) {
    println!("{v}");
}
fn elapsed(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
fn quantile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() as f64 * p).ceil() as usize).saturating_sub(1)]
}
fn stats(v: &[f64]) -> Value {
    json!({"n":v.len(),"p50_ms":quantile(&mut v.to_vec(),0.5),"p95_ms":quantile(&mut v.to_vec(),0.95),"max_ms":quantile(&mut v.to_vec(),1.0)})
}
fn footprint(pid: u32) -> u64 {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let rc =
        unsafe { libc::proc_pid_rusage(pid as _, libc::RUSAGE_INFO_V4, &mut info as *mut _ as _) };
    assert_eq!(rc, 0, "proc_pid_rusage");
    info.ri_phys_footprint
}
fn heap(i: &mut v8::Isolate) -> Value {
    let h = i.get_heap_statistics();
    json!({"used":h.used_heap_size(),"physical":h.total_physical_size(),"limit":h.heap_size_limit(),"external":h.external_memory()})
}
fn eval<'s>(s: &mut v8::PinScope<'s, '_>, code: &str) -> Option<v8::Local<'s, v8::Value>> {
    let text = v8::String::new(s, code).unwrap();
    v8::Script::compile(s, text, None)?.run(s)
}
fn eval_text(i: &mut v8::Isolate, c: &v8::Global<v8::Context>, code: &str) -> Option<String> {
    v8::scope!(let hs,i);
    let ctx = v8::Local::new(hs, c);
    let s = &mut v8::ContextScope::new(hs, ctx);
    let tc = std::pin::pin!(v8::TryCatch::new(s));
    let s = &mut tc.init();
    let out = eval(s, code).map(|r| r.to_string(s).unwrap().to_rust_string_lossy(s));
    if out.is_none()
        && !s.has_terminated()
        && let Some(e) = s.exception()
    {
        eprintln!("JS: {}", e.to_rust_string_lossy(s));
    }
    s.perform_microtask_checkpoint();
    out
}
fn now_callback(_s: &mut v8::PinScope, _a: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    rv.set_double(START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0);
}
#[derive(Default)]
struct ExternalQuota {
    used: usize,
    limit: usize,
}
impl ExternalQuota {
    fn allocate(
        &mut self,
        i: &mut v8::Isolate,
        bytes: usize,
    ) -> Result<v8::UniqueRef<v8::BackingStore>, &'static str> {
        let next = self
            .used
            .checked_add(bytes)
            .ok_or("external quota overflow")?;
        if next > self.limit {
            return Err("external quota exceeded");
        }
        self.used = next;
        Ok(v8::ArrayBuffer::new_backing_store(i, bytes))
    }
}
fn external_quota(blob: &[u8]) {
    let mut i = isolate(blob, 32 * MIB);
    let mut quota = ExternalQuota {
        used: 0,
        limit: 2 * MIB,
    };
    let backing = quota.allocate(&mut i, MIB).unwrap();
    assert!(quota.allocate(&mut i, 2 * MIB).is_err());
    let measured = i.get_heap_statistics().external_memory();
    drop(backing); // The reservation is conservatively held until generation retirement.
    emit(
        json!({"probe":"external-quota","limit":2*MIB,"reserved":quota.used,"v8_external_bytes":measured,"over_limit_rejected_before_allocation":true,"scope":"host buffers; JS external usage sampled after tasks"}),
    );
}
static CAGE_USED: AtomicUsize = AtomicUsize::new(0);
struct CageReservation(usize);
impl CageReservation {
    fn new(bytes: usize) -> Result<Self, &'static str> {
        CAGE_USED
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                n.checked_add(bytes).filter(|sum| *sum <= 3 * 1024 * MIB)
            })
            .map_err(|_| "shared cage budget exceeded")?;
        Ok(Self(bytes))
    }
}
impl Drop for CageReservation {
    fn drop(&mut self) {
        CAGE_USED.fetch_sub(self.0, Ordering::SeqCst);
    }
}
fn context(i: &mut v8::Isolate) -> v8::Global<v8::Context> {
    v8::scope!(let s,i);
    let c = v8::Context::new(s, Default::default());
    {
        let cs = &mut v8::ContextScope::new(s, c);
        let f = v8::Function::new(cs, now_callback).unwrap();
        let k = v8::String::new(cs, "hostNow").unwrap();
        c.global(cs).set(cs, k.into(), f.into()).unwrap();
    }
    v8::Global::new(s, c)
}
fn snapshot(kind: &str) -> Vec<u8> {
    let mut i = v8::Isolate::snapshot_creator(None, None);
    {
        v8::scope!(let s,&mut i);
        let c = v8::Context::new(s, Default::default());
        {
            let cs = &mut v8::ContextScope::new(s, c);
            eval(cs,&format!("globalThis.kind={kind:?}; globalThis.storage={{counter:7}}; globalThis.hot={{data:{{counter:0}}}}; globalThis.microtasks=0;")).unwrap();
        }
        s.set_default_context(c);
    }
    i.create_blob(v8::FunctionCodeHandling::Keep)
        .unwrap()
        .to_vec()
}
fn isolate(blob: &[u8], cap: usize) -> v8::OwnedIsolate {
    let mut i = v8::Isolate::new(
        v8::CreateParams::default()
            .snapshot_blob(blob.to_vec().into())
            .heap_limits(0, cap),
    );
    i.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    i
}
fn snapshot_cache_buffers() -> Vec<u8> {
    let mut chosen = Vec::new();
    for kind in [
        "editor-cluster",
        "extension",
        "compiler",
        "store",
        "compat-fallback",
    ] {
        let t = Instant::now();
        let blob = snapshot(kind);
        let build_ms = elapsed(t);
        let mut times = Vec::new();
        let mut last = Value::Null;
        for _ in 0..30 {
            let t = Instant::now();
            let mut i = isolate(&blob, 64 * MIB);
            let c = context(&mut i);
            assert_eq!(eval_text(&mut i, &c, "kind").unwrap(), kind);
            times.push(elapsed(t));
            last = heap(&mut i);
            drop(c);
            drop(i);
        }
        emit(
            json!({"probe":"snapshot","kind":kind,"bytes":blob.len(),"build_ms":build_ms,"restore":stats(&times),"heap":last}),
        );
        if kind == "extension" {
            chosen = blob;
        }
    }
    let mut i = isolate(&chosen, 64 * MIB);
    let c = context(&mut i);
    {
        let t = Instant::now();
        // Unique backing store has no JS owner or aliases while Rust writes.
        let backing = v8::ArrayBuffer::new_backing_store(&mut i, 1920 * 1080 * 4);
        assert!(!backing.is_shared());
        for (n, b) in backing.iter().enumerate() {
            b.set((n % 251) as u8);
        }
        let write_ms = elapsed(t);
        let ptr = backing.data().unwrap();
        v8::scope!(let hs,&mut i);
        let ctx = v8::Local::new(hs, &c);
        let s = &mut v8::ContextScope::new(hs, ctx);
        // The exclusive fill is over. No Rust write is allowed after publication.
        let buffer = v8::ArrayBuffer::with_backing_store(s, &backing.make_shared());
        assert_eq!(buffer.get_backing_store().data().unwrap(), ptr);
        let key = v8::String::new(s, "pixels").unwrap();
        ctx.global(s).set(s, key.into(), buffer.into()).unwrap();
        let t = Instant::now();
        assert!(
            eval(s, "new Uint8Array(pixels).every((b,i)=>b===i%251)")
                .unwrap()
                .boolean_value(s)
        );
        emit(
            json!({"probe":"sandbox-buffer","bytes":1920*1080*4,"rust_fill_ms":write_ms,"js_verify_ms":elapsed(t),"same_backing_pointer":true,"copy_after_fill":0}),
        );
    }
    let code = "function score(x){return x*7+1}; score(6)";
    let cache;
    {
        v8::scope!(let hs,&mut i);
        let ctx = v8::Local::new(hs, &c);
        let s = &mut v8::ContextScope::new(hs, ctx);
        let text = v8::String::new(s, code).unwrap();
        let mut src = v8::script_compiler::Source::new(text, None);
        let script = v8::script_compiler::compile_unbound_script(
            s,
            &mut src,
            v8::script_compiler::CompileOptions::EagerCompile,
            v8::script_compiler::NoCacheReason::NoReason,
        )
        .unwrap();
        assert_eq!(
            script
                .bind_to_current_context(s)
                .run(s)
                .unwrap()
                .int32_value(s),
            Some(43)
        );
        cache = script.create_code_cache().unwrap().to_vec();
    }
    drop(c);
    drop(i);
    let mut i = isolate(&chosen, 64 * MIB);
    let c = context(&mut i);
    {
        v8::scope!(let hs,&mut i);
        let ctx = v8::Local::new(hs, &c);
        let s = &mut v8::ContextScope::new(hs, ctx);
        let text = v8::String::new(s, code).unwrap();
        let mut src = v8::script_compiler::Source::new_with_cached_data(
            text,
            None,
            v8::script_compiler::CachedData::new(&cache),
        );
        let script = v8::script_compiler::compile_unbound_script(
            s,
            &mut src,
            v8::script_compiler::CompileOptions::ConsumeCodeCache,
            v8::script_compiler::NoCacheReason::NoReason,
        )
        .unwrap();
        assert!(!src.get_cached_data().unwrap().rejected());
        assert_eq!(
            script
                .bind_to_current_context(s)
                .run(s)
                .unwrap()
                .int32_value(s),
            Some(43)
        );
    }
    let mut key = Sha256::new();
    for bytes in [
        v8::V8::get_version().as_bytes(),
        &chosen,
        code.as_bytes(),
        FLAGS.as_bytes(),
    ] {
        key.update((bytes.len() as u64).to_le_bytes());
        key.update(bytes);
    }
    emit(
        json!({"probe":"code-cache","bytes":cache.len(),"accepted":true,"key":format!("{:x}",key.finalize()),"version_tag":v8::script_compiler::cached_data_version_tag()}),
    );
    drop(c);
    drop(i);
    chosen
}
fn watchdog(blob: Vec<u8>) {
    let (tx, rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let owner = thread::spawn(move || {
        let owner_id = thread::current().id();
        let mut i = isolate(&blob, 64 * MIB);
        let c = context(&mut i);
        let persisted =
            eval_text(&mut i, &c, "storage.counter=19; JSON.stringify(storage)").unwrap();
        tx.send(i.thread_safe_handle()).unwrap();
        // Explicit test authorization models a trusted user's Stop, not an automatic timeout kill.
        assert!(eval_text(&mut i, &c, "for(;;){}").is_none());
        drop(c);
        drop(i); // scopes -> Globals -> exit/dispose, on the owning thread.
        assert_eq!(thread::current().id(), owner_id);
        let mut i = isolate(&blob, 64 * MIB);
        let c = context(&mut i);
        assert_eq!(
            eval_text(
                &mut i,
                &c,
                &format!(
                    "storage={persisted}; Promise.resolve().then(()=>microtasks++); storage.counter"
                )
            )
            .unwrap(),
            "19"
        );
        assert_eq!(eval_text(&mut i, &c, "microtasks").unwrap(), "1");
        drop(c);
        drop(i);
        stop_tx.send(()).unwrap();
    });
    let handle = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    thread::sleep(Duration::from_millis(40));
    let t = Instant::now();
    assert!(handle.terminate_execution());
    stop_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    owner.join().unwrap();
    assert!(!handle.terminate_execution());
    emit(
        json!({"probe":"watchdog","stop_dispose_recreate_ms":elapsed(t),"owner_thread_disposal":true,"storage_rehydrated":true,"microtasks_drained":true,"trusted_policy":"explicit-test-Stop"}),
    );
}
struct QuotaState {
    handle: v8::IsolateHandle,
    calls: AtomicUsize,
}
unsafe extern "C" fn near_limit(data: *mut c_void, current: usize, _initial: usize) -> usize {
    let q = unsafe { &*(data as *const QuotaState) };
    q.calls.fetch_add(1, Ordering::SeqCst);
    q.handle.terminate_execution();
    current + 8 * MIB
}
fn quota() {
    let mut i = v8::Isolate::new(v8::CreateParams::default().heap_limits(0, 16 * MIB));
    let state = Box::new(QuotaState {
        handle: i.thread_safe_handle(),
        calls: AtomicUsize::new(0),
    });
    i.add_near_heap_limit_callback(near_limit, &*state as *const _ as *mut c_void);
    let c = context(&mut i);
    assert!(
        eval_text(
            &mut i,
            &c,
            "globalThis.kept=[]; for(;;)kept.push(new Array(65536).fill(123))"
        )
        .is_none()
    );
    let calls = state.calls.load(Ordering::SeqCst);
    assert_eq!(calls, 1);
    drop(c);
    i.remove_near_heap_limit_callback(near_limit, 0);
    drop(i);
    drop(state);
    emit(
        json!({"probe":"heap-quota","callback_calls":calls,"configured_bytes":16*MIB,"emergency_bytes":8*MIB,"terminated":true}),
    );
}
fn parking(blob: &[u8]) {
    let mut i = isolate(blob, 64 * MIB);
    let c = context(&mut i);
    assert_eq!(
        eval_text(&mut i, &c, "globalThis.turn=0; turn").unwrap(),
        "0"
    );
    // Safety: only V8 globals, owned snapshot bytes and default allocator; no
    // non-Send embedder data, Weak handles, finalizers or attached cppgc heap.
    let shared = Arc::new(unsafe { i.try_into_shared() }.unwrap());
    let c = Arc::new(c);
    let mut times = Vec::new();
    for n in 1..=8 {
        let s = shared.clone();
        let c = c.clone();
        let t = Instant::now();
        thread::spawn(move || {
            let mut guard = s.lock();
            assert_eq!(eval_text(&mut guard, &c, "++turn").unwrap(), n.to_string());
        })
        .join()
        .unwrap();
        times.push(elapsed(t));
    }
    {
        let mut guard = shared.lock();
        assert_eq!(eval_text(&mut guard, &c, "turn").unwrap(), "8");
    }
    drop(c);
    drop(shared);
    emit(
        json!({"probe":"parking","handoffs":8,"spawn_lock_execute_join":stats(&times),"decision":"release idle worker thread using SharedIsolate/Locker"}),
    );
}
struct Generation {
    id: usize,
    context: v8::Global<v8::Context>,
    isolate: v8::SharedIsolate,
    _cage: CageReservation,
}
impl Generation {
    fn new(blob: &[u8], id: usize, hot: u32) -> Self {
        let cage = CageReservation::new(64 * MIB).unwrap();
        let mut i = isolate(blob, 64 * MIB);
        let c = context(&mut i);
        assert_eq!(eval_text(&mut i,&c,&format!("hot.data.counter={hot}; globalThis.historyPayload=new Uint8Array({}); historyPayload.fill({id}%251); globalThis.undo=()=>historyPayload[0]; hot.data.counter",MIB)).unwrap(),hot.to_string());
        let shared = unsafe { i.try_into_shared() }.unwrap();
        Self {
            id,
            isolate: shared,
            context: c,
            _cage: cage,
        }
    }
    fn run(&self, src: &str) -> String {
        let mut lock = self.isolate.lock();
        eval_text(&mut lock, &self.context, src).unwrap()
    }
}
// Field order drops the Global first; SharedIsolate then drains deferred resets
// under its final lock before disposing. No V8 references cross a lock boundary.
fn reload(blob: &[u8]) {
    let base = footprint(std::process::id());
    let mut generations = Vec::new();
    let mut timings = Vec::new();
    let mut hot = 0;
    for id in 0..20 {
        let t = Instant::now();
        let g = Arc::new(Generation::new(blob, id, hot));
        hot = g.run("++hot.data.counter").parse().unwrap();
        assert_eq!(hot, id as u32 + 1);
        timings.push(elapsed(t));
        generations.push(g);
    }
    let retained = footprint(std::process::id());
    let cage_retained = CAGE_USED.load(Ordering::SeqCst);
    assert!(CageReservation::new(3 * 1024 * MIB).is_err());
    let export = generations[0].clone();
    for g in &generations {
        assert_eq!(g.run("undo()"), (g.id % 251).to_string());
    }
    let held_external: usize = generations
        .iter()
        .map(|g| {
            let mut i = g.isolate.lock();
            i.get_heap_statistics().external_memory()
        })
        .sum();
    let weak = Arc::downgrade(&generations[1]);
    generations.clear();
    assert!(weak.upgrade().is_none());
    assert_eq!(export.run("undo()"), "0");
    let pinned = footprint(std::process::id());
    let weak_export = Arc::downgrade(&export);
    drop(export);
    assert!(weak_export.upgrade().is_none());
    let retired = footprint(std::process::id());
    emit(
        json!({"probe":"reload-history","generations":20,"reload":stats(&timings),"retained_external_bytes":held_external,"physical_footprint":{"before":base,"retained":retained,"export_only":pinned,"all_retired":retired},"all_closures_callable":true,"export_pin_survived_history_retirement":true,"generations_disposed":20,"cage_retained_reserved":cage_retained,"cage_reserved_after_retirement":CAGE_USED.load(Ordering::SeqCst)}),
    );
    // Live services need identity, held old implementations, stacked disposal,
    // failed-activation recovery. A fresh whole-cluster isolate loses ===.
    let g = Generation::new(blob, 0, 0);
    let t = Instant::now();
    let result = g.run(include_str!("../cluster.js"));
    assert_eq!(result, "true");
    emit(
        json!({"probe":"cluster-reload","ms":elapsed(t),"staged_service_identity":true,"failed_activation_recovered":true,"decision":"staged module replacement within a sync cluster"}),
    );
}
fn run_js_file(blob: &[u8], path: &str) {
    let mut i = isolate(blob, 192 * MIB);
    let c = context(&mut i);
    let code = std::fs::read_to_string(path).unwrap();
    let result = eval_text(&mut i, &c, &code).expect("JS probe failed");
    let mut value: Value = serde_json::from_str(&result).expect("JS probe must return JSON");
    value["bundle_bytes"] = json!(code.len());
    value["heap"] = heap(&mut i);
    emit(value);
    drop(c);
    drop(i);
}

#[link(name = "sandbox")]
unsafe extern "C" {
    fn sandbox_init(profile: *const i8, flags: u64, error: *mut *mut i8) -> i32;
    fn sandbox_free_error(error: *mut i8);
}
fn seatbelt() {
    let profile = CString::new(include_str!("../store.sb")).unwrap();
    let mut error = std::ptr::null_mut();
    let rc = unsafe { sandbox_init(profile.as_ptr(), 0, &mut error) };
    if rc != 0 {
        let reason = if error.is_null() {
            "unknown".into()
        } else {
            unsafe { std::ffi::CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned()
        };
        if !error.is_null() {
            unsafe { sandbox_free_error(error) }
        }
        panic!("seatbelt refused: {reason}");
    }
}
fn helper(mode: &str, fd: i32) {
    // All runtime startup code is trusted; no extension runs before confinement.
    let blob = snapshot("store");
    let mut i = isolate(&blob, 16 * MIB);
    let c = context(&mut i);
    seatbelt();
    let mut socket = unsafe { UnixStream::from_raw_fd(fd) };
    let denied =
        |e: std::io::Error| matches!(e.raw_os_error(), Some(libc::EPERM) | Some(libc::EACCES));
    let read_denied = std::fs::read("/etc/hosts").err().is_some_and(denied);
    let write_denied = std::fs::write("/tmp/pm-s1-denied", "x")
        .err()
        .is_some_and(denied);
    let network_denied = std::net::TcpStream::connect("127.0.0.1:9")
        .err()
        .is_some_and(denied);
    let ready = json!({"probe":"helper-ready","seatbelt":true,"read_denied":read_denied,"write_denied":write_denied,"network_denied":network_denied,"footprint":footprint(std::process::id()),"heap":heap(&mut i)});
    writeln!(socket, "{ready}").unwrap();
    assert!(read_denied && write_denied && network_denied);
    let mut command = [0u8; 1];
    socket.read_exact(&mut command).unwrap();
    if mode == "--fatal-oom" {
        eval_text(
            &mut i,
            &c,
            "globalThis.kept=[]; for(;;)kept.push(new Array(65536).fill(123))",
        );
        panic!("expected a V8 fatal OOM");
    }
    // One counted IPC copy into an allocator-owned buffer before publishing.
    let backing = v8::ArrayBuffer::new_backing_store(&mut i, 4096);
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(backing.data().unwrap().as_ptr() as *mut u8, 4096)
    };
    socket.read_exact(bytes).unwrap();
    {
        v8::scope!(let hs,&mut i);
        let ctx = v8::Local::new(hs, &c);
        let s = &mut v8::ContextScope::new(hs, ctx);
        let ab = v8::ArrayBuffer::with_backing_store(s, &backing.make_shared());
        let key = v8::String::new(s, "ipcPixels").unwrap();
        ctx.global(s).set(s, key.into(), ab.into()).unwrap();
        assert!(
            eval(s, "new Uint8Array(ipcPixels).every((v,i)=>v===i%251)")
                .unwrap()
                .boolean_value(s)
        );
    }
    writeln!(socket, "{{\"ipc_copy_bytes\":4096,\"verified\":true}}").unwrap();
    drop(c);
    drop(i);
}
fn child_test(mode: &str) -> Value {
    let (mut parent, child) = UnixStream::pair().unwrap();
    parent
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let fd = child.as_raw_fd();
    unsafe {
        assert_eq!(libc::fcntl(fd, libc::F_SETFD, 0), 0);
    }
    let mut process = Command::new(std::env::current_exe().unwrap())
        .args([mode, &fd.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child);
    let mut line = Vec::new();
    loop {
        let mut b = [0];
        parent.read_exact(&mut b).unwrap();
        if b[0] == b'\n' {
            break;
        }
        line.push(b[0]);
    }
    let ready: Value = serde_json::from_slice(&line).unwrap();
    assert_eq!(ready["seatbelt"], true);
    parent.write_all(b"x").unwrap();
    if mode == "--helper" {
        parent
            .write_all(&(0..4096).map(|n| (n % 251) as u8).collect::<Vec<_>>())
            .unwrap();
        let mut reply = String::new();
        parent.read_to_string(&mut reply).unwrap();
        assert!(reply.contains("\"verified\":true"));
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if process.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() > deadline {
            process.kill().unwrap();
            panic!("helper deadline exceeded");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = process.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if mode == "--fatal-oom" {
        assert_eq!(output.status.signal(), Some(libc::SIGABRT));
        assert!(
            stderr.contains("heap out of memory") || stderr.contains("JavaScript heap"),
            "not a V8 OOM: {stderr}"
        );
    } else {
        assert!(output.status.success(), "{stderr}");
    }
    json!({"probe":mode,"ready":ready,"exit_signal":output.status.signal(),"parent_survived":true,"v8_fatal_oom":mode=="--fatal-oom"})
}
fn main() {
    // No more than 3 GiB of the shared 4-GiB compression cage is admitted.
    // Include compiler + active and retained generations in this same ledger.
    let reserved = [3 * 128 * MIB, 39 * 32 * MIB, 192 * MIB, 16 * 32 * MIB];
    assert!(reserved.iter().sum::<usize>() < 3 * 1024 * MIB);
    v8::V8::set_flags_from_string(FLAGS);
    let platform = v8::new_default_platform(2, false).make_shared();
    v8::V8::initialize_platform(platform);
    v8::V8::initialize();
    assert!(unsafe { v8__V8__IsSandboxEnabled() });
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args.get(1).map(String::as_str).unwrap_or("--all");
    match mode {
        "--smoke" => emit(
            json!({"probe":"smoke","sandbox":true,"v8":v8::V8::get_version(),"pointer_compression_cargo_feature":true}),
        ),
        "--quota" => quota(),
        "--helper" | "--fatal-oom" => helper(mode, args[2].parse().unwrap()),
        "--effects" => {
            let blob = snapshot("extension");
            let mut i = isolate(&blob, 64 * MIB);
            let c = context(&mut i);
            eval_text(&mut i, &c, include_str!("../effects.js")).unwrap();
            let result = eval_text(&mut i, &c, "effectResult").unwrap();
            emit(serde_json::from_str(&result).unwrap());
            drop(c);
            drop(i);
        }
        "--js" => {
            let blob = snapshot("extension");
            run_js_file(&blob, &args[2]);
        }
        "--all" => {
            emit(
                json!({"probe":"build","v8":v8::V8::get_version(),"sandbox":true,"cage_reserved_example":reserved.iter().sum::<usize>(),"footprint":footprint(std::process::id())}),
            );
            let blob = snapshot_cache_buffers();
            watchdog(blob.clone());
            parking(&blob);
            external_quota(&blob);
            reload(&blob);
            let output = Command::new(std::env::current_exe().unwrap())
                .arg("--quota")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            print!("{}", String::from_utf8_lossy(&output.stdout));
            emit(child_test("--helper"));
            let mut survivor = isolate(&blob, 32 * MIB);
            let survivor_context = context(&mut survivor);
            assert_eq!(
                eval_text(
                    &mut survivor,
                    &survivor_context,
                    "storage.counter=73; storage.counter"
                )
                .unwrap(),
                "73"
            );
            emit(child_test("--fatal-oom"));
            assert_eq!(
                eval_text(&mut survivor, &survivor_context, "storage.counter").unwrap(),
                "73"
            );
            emit(json!({"probe":"oom-blast-radius","live_parent_isolate_state_preserved":true}));
            drop(survivor_context);
            drop(survivor);
        }
        _ => panic!("unknown mode"),
    }
    unsafe {
        v8::V8::dispose();
    }
    v8::V8::dispose_platform();
}
