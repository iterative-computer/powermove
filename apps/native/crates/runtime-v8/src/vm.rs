use crate::{
    CacheKey, Cancellation, CodeCache, EngineProxy, Error, FetchRequest, Generation, HostBudget,
    Limits, Result, SnapshotKind,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    ffi::c_void,
    sync::{
        Arc, Mutex, Once, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

pub(crate) const SVELTE: &str = include_str!("../compiler/svelte-5.56.10.js");
const WEB: &str = include_str!("web.js");
const MIB: usize = 1024 * 1024;
pub(crate) const FLAGS: &str = if cfg!(feature = "macos-sandbox") {
    "no-extensible-ro-snapshot;sandbox=true"
} else {
    "no-extensible-ro-snapshot;sandbox=false"
};
pub(crate) fn initialize() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Every kind has its own custom snapshot, but isolates share V8's
        // read-only heap. Extending it independently per snapshot gives those
        // snapshots incompatible root offsets when another kind is restored.
        v8::V8::set_flags_from_string("--no-extensible-ro-snapshot");
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}
pub(crate) fn snapshot(kind: SnapshotKind) -> Arc<Vec<u8>> {
    initialize();
    type Snapshots = Mutex<HashMap<SnapshotKind, Arc<Vec<u8>>>>;
    static SNAPSHOTS: OnceLock<Snapshots> = OnceLock::new();
    let mut snapshots = SNAPSHOTS.get_or_init(Default::default).lock().unwrap();
    snapshots
        .entry(kind)
        .or_insert_with(|| {
            let mut isolate = v8::Isolate::snapshot_creator(None, None);
            {
                v8::scope!(let s, &mut isolate);
                let context = v8::Context::new(s, Default::default());
                {
                    let cs = &mut v8::ContextScope::new(s, context);
                    let source = v8::String::new(cs, WEB).unwrap();
                    v8::Script::compile(cs, source, None)
                        .unwrap()
                        .run(cs)
                        .unwrap();
                    if kind == SnapshotKind::Compiler {
                        let source = v8::String::new(cs, SVELTE).unwrap();
                        v8::Script::compile(cs, source, None)
                            .unwrap()
                            .run(cs)
                            .unwrap();
                    }
                    let source =
                        v8::String::new(cs, &format!("globalThis.__kind={:?}", kind.name()))
                            .unwrap();
                    v8::Script::compile(cs, source, None)
                        .unwrap()
                        .run(cs)
                        .unwrap();
                }
                s.set_default_context(context);
            }
            Arc::new(
                isolate
                    .create_blob(v8::FunctionCodeHandling::Keep)
                    .unwrap()
                    .to_vec(),
            )
        })
        .clone()
}

pub(crate) struct NativeState {
    pub limits: Limits,
    pub queued: Vec<(u64, FetchRequest)>,
    /// Accepted fetches waiting for reservation room (`Limits::queue_fetches`).
    pub deferred: std::collections::VecDeque<(u64, FetchRequest)>,
    pub aborted: Vec<u64>,
    pub logs: Vec<(String, String)>,
    pub in_flight: usize,
    pub reserved: usize,
    engine_reserved: usize,
    sampled_external: usize,
    next_id: u64,
    start: Instant,
    generation: Generation,
    engine: Option<Arc<dyn EngineProxy>>,
    cancellation: Cancellation,
    sync_calls: usize,
}
impl NativeState {
    /// Room a fetch leaves for a synchronous engine reply.
    pub fn engine_headroom(&self) -> usize {
        if self.limits.queue_fetches {
            self.limits
                .max_engine_response_bytes
                .min(self.limits.external_bytes)
        } else {
            0
        }
    }
    /// Starts waiting fetches while they fit; the rest keep waiting in order.
    pub fn admit_deferred(&mut self) {
        let reserve = self.limits.max_response_bytes;
        while !self.deferred.is_empty() && self.admits_response(reserve + self.engine_headroom()) {
            let (id, mut request) = self.deferred.pop_front().unwrap();
            // Its time starts when it starts, not while it waited.
            request.deadline = Instant::now() + self.limits.host_timeout;
            self.reserved += reserve;
            self.queued.push((id, request));
        }
    }
    fn admits_response(&self, bytes: usize) -> bool {
        self.sampled_external
            .checked_add(self.reserved)
            .and_then(|n| n.checked_add(self.engine_reserved))
            .and_then(|n| n.checked_add(bytes))
            .is_some_and(|n| n <= self.limits.external_bytes)
    }
}
/// Release on native adapter errors too; no V8 entry occurs under this lock.
struct EngineReservation {
    state: Arc<Mutex<NativeState>>,
    bytes: usize,
}
impl Drop for EngineReservation {
    fn drop(&mut self) {
        self.state.lock().unwrap().engine_reserved -= self.bytes;
    }
}
#[derive(Clone)]
struct Native(Arc<Mutex<NativeState>>);
struct Quota {
    handle: v8::IsolateHandle,
    exceeded: AtomicBool,
}
unsafe extern "C" fn near_limit(data: *mut c_void, current: usize, _initial: usize) -> usize {
    // Box lives until callback removal, on the same owner thread as this callback.
    let quota = unsafe { &*(data as *const Quota) };
    if !quota.exceeded.swap(true, Ordering::AcqRel) {
        quota.handle.terminate_execution();
        current.saturating_add(8 * MIB)
    } else {
        current
    }
}
/// Interrupts never reenter JS. Only read heap statistics and terminate.
pub(crate) unsafe extern "C" fn check_external(mut raw: v8::UnsafeRawIsolatePtr, _: *mut c_void) {
    // V8 invokes this with the isolate entered, on its currently locked owner.
    let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut raw) };
    if let Some(native) = isolate.get_slot::<Native>().cloned() {
        let native = native.0.lock().unwrap();
        if isolate
            .get_heap_statistics()
            .external_memory()
            .saturating_add(native.reserved)
            .saturating_add(native.engine_reserved)
            > native.limits.external_bytes
        {
            isolate.terminate_execution();
        }
    }
}
pub(crate) fn throw(s: &mut v8::PinScope, msg: &str) {
    let msg = v8::String::new(s, msg).unwrap();
    let error = v8::Exception::type_error(s, msg);
    s.throw_exception(error);
}
fn now(s: &mut v8::PinScope, _: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let elapsed = s
        .get_slot::<Native>()
        .unwrap()
        .0
        .lock()
        .unwrap()
        .start
        .elapsed()
        .as_secs_f64()
        * 1000.;
    rv.set_double(elapsed);
}
fn native(s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let op = a.get(0).to_rust_string_lossy(s);
    let Some(text) = a.get(1).to_string(s) else {
        return;
    };
    let state = s.get_slot::<Native>().unwrap().0.clone();
    // Refresh at the allocation boundary, not only at the next microtask or
    // watchdog interrupt. JS may have retained buffers earlier in this stack.
    state.lock().unwrap().sampled_external = s.get_heap_statistics().external_memory();
    if text.length() > state.lock().unwrap().limits.max_op_bytes {
        throw(s, "Host argument quota");
        return;
    }
    let arg = text.to_rust_string_lossy(s);
    if arg.len() > state.lock().unwrap().limits.max_op_bytes {
        throw(s, "Host argument quota");
        return;
    }
    let result = serde_json::from_str(&arg)
        .map_err(|e| e.to_string())
        .and_then(|v| {
            if op == "engine" {
                engine_op(&state, v)
            } else {
                host_op(&mut state.lock().unwrap(), &op, v).map(|v| v.to_string())
            }
        });
    if state.lock().unwrap().cancellation.is_cancelled() {
        s.terminate_execution();
        return;
    }
    match result {
        Ok(text) => match v8::String::new(s, &text) {
            Some(result) => rv.set(result.into()),
            None => throw(s, "Engine response quota"),
        },
        Err(error) => throw(s, &error),
    }
}
/// The reply as JSON text, serialized once.
fn engine_op(state: &Arc<Mutex<NativeState>>, input: Value) -> std::result::Result<String, String> {
    let (engine, generation, budget, _reservation) = {
        let mut n = state.lock().unwrap();
        if n.sync_calls >= n.limits.max_host_ops {
            return Err("Engine operation quota".into());
        }
        n.sync_calls += 1;
        let engine = n.engine.clone().ok_or("Engine service is not attached")?;
        let reserve = n
            .limits
            .max_engine_response_bytes
            .min(n.limits.external_bytes);
        if !n.admits_response(reserve) {
            return Err("External memory reservation quota".into());
        }
        n.engine_reserved += reserve;
        (
            engine,
            n.generation.clone(),
            HostBudget {
                cancellation: n.cancellation.clone(),
                deadline: Instant::now() + n.limits.host_timeout,
                max_response_bytes: reserve,
            },
            EngineReservation {
                state: state.clone(),
                bytes: reserve,
            },
        )
    };
    budget.check().map_err(|e| e.to_string())?;
    let text = engine
        .call_text(
            &generation,
            input["operation"]
                .as_str()
                .ok_or("Invalid engine operation")?,
            &input["input"],
            budget.clone(),
        )
        .map_err(|e| e.to_string())?;
    budget.check().map_err(|e| e.to_string())?;
    if text.len() > budget.max_response_bytes {
        return Err("Engine response quota".into());
    }
    Ok(text)
}
fn host_op(n: &mut NativeState, op: &str, v: Value) -> std::result::Result<Value, String> {
    match op {
        "console" => {
            if n.logs.len() >= n.limits.max_host_ops {
                return Err("Console queue quota".into());
            }
            n.logs.push((
                v["level"].as_str().unwrap_or("log").into(),
                v["message"].as_str().unwrap_or("").into(),
            ));
            Ok(Value::Null)
        }
        "fetch" => {
            if n.in_flight >= n.limits.max_host_ops {
                return Err("Host operation quota".into());
            }
            let reserve = n.limits.max_response_bytes;
            let headroom = n.engine_headroom();
            let start = n.admits_response(reserve + headroom);
            // Waiting helps only when other fetches hold the room it needs.
            let fits_alone = n
                .sampled_external
                .checked_add(n.engine_reserved)
                .and_then(|v| v.checked_add(reserve + headroom))
                .is_some_and(|v| v <= n.limits.external_bytes);
            if !start && !(n.limits.queue_fetches && fits_alone) {
                return Err("External memory reservation quota".into());
            }
            let url = url::Url::parse(v["url"].as_str().ok_or("Missing URL")?)
                .map_err(|e| e.to_string())?;
            if !["http", "https"].contains(&url.scheme()) {
                return Err("Unsupported fetch protocol".into());
            }
            let body = v["body"]
                .as_array()
                .ok_or("Invalid body")?
                .iter()
                .map(|x| {
                    x.as_u64()
                        .filter(|x| *x < 256)
                        .map(|x| x as u8)
                        .ok_or("Invalid byte")
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let headers = v["headers"]
                .as_array()
                .ok_or("Invalid headers")?
                .iter()
                .map(|x| {
                    Ok((
                        x[0].as_str().ok_or("Invalid header")?.into(),
                        x[1].as_str().ok_or("Invalid header")?.into(),
                    ))
                })
                .collect::<std::result::Result<Vec<_>, String>>()?;
            n.next_id += 1;
            n.in_flight += 1;
            let request = (
                n.next_id,
                FetchRequest {
                    url: url.into(),
                    method: v["method"].as_str().unwrap_or("GET").into(),
                    headers,
                    body,
                    max_response_bytes: reserve,
                    deadline: Instant::now() + n.limits.host_timeout,
                },
            );
            if start {
                n.reserved += reserve;
                n.queued.push(request);
            } else {
                n.deferred.push_back(request);
            }
            Ok(json!(n.next_id))
        }
        "abort" => {
            if n.aborted.len() < n.limits.max_host_ops {
                n.aborted.push(v.as_u64().ok_or("Invalid request id")?);
            }
            Ok(Value::Null)
        }
        "params.parse" => Ok(json!(
            url::form_urlencoded::parse(v.as_str().ok_or("Invalid params")?.as_bytes())
                .into_owned()
                .collect::<Vec<_>>()
        )),
        "params.serialize" => {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            for p in v.as_array().ok_or("Invalid params")? {
                serializer.append_pair(
                    p[0].as_str().ok_or("Invalid key")?,
                    p[1].as_str().ok_or("Invalid value")?,
                );
            }
            Ok(json!(serializer.finish()))
        }
        "url" => {
            let input = v["input"].as_str().ok_or("Missing URL")?;
            let mut url = if let Some(base) = v["base"].as_str() {
                url::Url::parse(base).and_then(|b| b.join(input))
            } else {
                url::Url::parse(input)
            }
            .map_err(|e| e.to_string())?;
            if let Some(field) = v["field"].as_str() {
                let value = v["value"].as_str().ok_or("Missing URL field value")?;
                match field {
                    "href" => url = url::Url::parse(value).map_err(|e| e.to_string())?,
                    "protocol" => {
                        let _ = url.set_scheme(value.trim_end_matches(':'));
                    }
                    "username" => {
                        let _ = url.set_username(value);
                    }
                    "password" => {
                        let _ = url.set_password(Some(value));
                    }
                    "hostname" => {
                        let _ = url.set_host(Some(value));
                    }
                    "host" => {
                        let candidate = format!("{}://{value}/", url.scheme());
                        if let Ok(host) = url::Url::parse(&candidate) {
                            let _ = url.set_host(host.host_str());
                            let _ = url.set_port(host.port());
                        }
                    }
                    "port" => {
                        if value.is_empty() {
                            let _ = url.set_port(None);
                        } else if let Ok(port) = value.parse() {
                            let _ = url.set_port(Some(port));
                        }
                    }
                    "pathname" => url.set_path(value),
                    "search" => url.set_query(if value.is_empty() {
                        None
                    } else {
                        Some(value.strip_prefix('?').unwrap_or(value))
                    }),
                    "hash" => url.set_fragment(if value.is_empty() {
                        None
                    } else {
                        Some(value.strip_prefix('#').unwrap_or(value))
                    }),
                    _ => return Err("Unknown URL field".into()),
                }
            }
            let host = url.host_str().unwrap_or("");
            let port = url.port().map(|p| p.to_string()).unwrap_or_default();
            Ok(
                json!({"href":url.as_str(),"protocol":format!("{}:",url.scheme()),"username":url.username(),"password":url.password().unwrap_or(""),"host":if port.is_empty(){host.to_owned()}else{format!("{host}:{port}")},"hostname":host,"port":port,"pathname":url.path(),"search":url.query().filter(|s|!s.is_empty()).map(|s|format!("?{s}")).unwrap_or_default(),"hash":url.fragment().filter(|s|!s.is_empty()).map(|s|format!("#{s}")).unwrap_or_default(),"origin":url.origin().ascii_serialization()}),
            )
        }
        _ => Err("Unknown host operation".into()),
    }
}
struct CloneDelegate;
impl v8::ValueSerializerImpl for CloneDelegate {
    fn throw_data_clone_error<'s>(
        &self,
        s: &mut v8::PinScope<'s, '_>,
        message: v8::Local<'s, v8::String>,
    ) {
        let error = v8::Exception::error(s, message);
        let object = error.to_object(s).unwrap();
        let key = v8::String::new(s, "name").unwrap();
        let name = v8::String::new(s, "DataCloneError").unwrap();
        object.set(s, key.into(), name.into());
        s.throw_exception(error);
    }
}
impl v8::ValueDeserializerImpl for CloneDelegate {}
fn throw_clone(s: &mut v8::PinScope, message: &str) {
    let message = v8::String::new(s, message).unwrap();
    v8::ValueSerializerImpl::throw_data_clone_error(&CloneDelegate, s, message);
}
fn clone_value(s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let value = a.get(0);
    let Ok(list) = v8::Local::<v8::Array>::try_from(a.get(1)) else {
        throw(s, "Invalid transfer list");
        return;
    };
    let mut transfers = Vec::new();
    for i in 0..list.length() {
        let Some(item) = list.get_index(s, i) else {
            return;
        };
        let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(item) else {
            throw_clone(s, "Only ArrayBuffers can be transferred");
            return;
        };
        if buffer.was_detached() || transfers.contains(&buffer) {
            throw_clone(s, "Detached or duplicate transfer");
            return;
        }
        transfers.push(buffer);
    }
    let context = s.get_current_context();
    let serializer = v8::ValueSerializer::new(s, Box::new(CloneDelegate));
    serializer.write_header();
    for (i, buffer) in transfers.iter().enumerate() {
        serializer.transfer_array_buffer(i as u32, *buffer);
    }
    if serializer.write_value(context, value) != Some(true) {
        return;
    }
    let bytes = serializer.release();
    let limit = s
        .get_slot::<Native>()
        .unwrap()
        .0
        .lock()
        .unwrap()
        .limits
        .external_bytes;
    if bytes.len() > limit {
        throw(s, "Structured clone quota");
        return;
    }
    let deserializer = v8::ValueDeserializer::new(s, Box::new(CloneDelegate), &bytes);
    if deserializer.read_header(context) != Some(true) {
        return;
    }
    for (i, buffer) in transfers.iter().enumerate() {
        let new = v8::ArrayBuffer::with_backing_store(s, &buffer.get_backing_store());
        deserializer.transfer_array_buffer(i as u32, new);
    }
    if let Some(result) = deserializer.read_value(context) {
        for buffer in transfers {
            buffer.detach(None);
        }
        rv.set(result);
    }
}

pub(crate) struct Vm {
    context: Option<v8::Global<v8::Context>>,
    retire_function: Option<v8::Global<v8::Function>>,
    isolate: Option<v8::SharedIsolate>,
    quota: Box<Quota>,
    pub native: Arc<Mutex<NativeState>>,
    blob: Arc<Vec<u8>>,
    cache: CodeCache,
    pub cache_hits: usize,
    owner: std::thread::ThreadId,
}
impl Vm {
    pub fn new(
        kind: SnapshotKind,
        limits: Limits,
        storage: &Value,
        hot: &Value,
        cache: CodeCache,
        generation: Generation,
        engine: Option<Arc<dyn EngineProxy>>,
    ) -> Result<Self> {
        let blob = snapshot(kind);
        let mut isolate = v8::Isolate::new(
            v8::CreateParams::default()
                .snapshot_blob(blob.as_ref().clone().into())
                .heap_limits(0, limits.heap_bytes),
        );
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        let quota = Box::new(Quota {
            handle: isolate.thread_safe_handle(),
            exceeded: AtomicBool::new(false),
        });
        isolate.add_near_heap_limit_callback(near_limit, &*quota as *const Quota as *mut c_void);
        let native_state = Arc::new(Mutex::new(NativeState {
            limits: limits.clone(),
            queued: vec![],
            deferred: Default::default(),
            aborted: vec![],
            logs: vec![],
            in_flight: 0,
            reserved: 0,
            engine_reserved: 0,
            sampled_external: 0,
            next_id: 0,
            start: Instant::now(),
            generation,
            engine,
            cancellation: Cancellation::default(),
            sync_calls: 0,
        }));
        isolate.set_slot(Native(native_state.clone()));
        crate::modules::install(&mut isolate);
        let (context, retire_function) = {
            v8::scope!(let s,&mut isolate);
            let context = v8::Context::new(s, Default::default());
            let cs = &mut v8::ContextScope::new(s, context);
            for (name, function) in [
                ("__host", v8::Function::new(cs, native).unwrap()),
                ("__now", v8::Function::new(cs, now).unwrap()),
                ("__clone", v8::Function::new(cs, clone_value).unwrap()),
                (
                    "__linkModules",
                    v8::Function::new(cs, crate::modules::link).unwrap(),
                ),
            ] {
                // The compiler handles untrusted source as data and never links it.
                if name == "__linkModules" && kind == SnapshotKind::Compiler {
                    continue;
                }
                let key = v8::String::new(cs, name).unwrap();
                context.global(cs).set(cs, key.into(), function.into());
            }
            let key = v8::String::new(cs, "__pm").unwrap();
            let object = context
                .global(cs)
                .get(cs, key.into())
                .unwrap()
                .to_object(cs)
                .unwrap();
            let key = v8::String::new(cs, "retire").unwrap();
            let function =
                v8::Local::<v8::Function>::try_from(object.get(cs, key.into()).unwrap()).unwrap();
            (v8::Global::new(cs, context), v8::Global::new(cs, function))
        };
        // SAFETY: only Send-owned native state, strong Globals, default allocator;
        // no Weak/finalizer/cppgc callbacks. V8 is accessed solely under Locker.
        let isolate = unsafe { isolate.try_into_shared() }
            .map_err(|_| Error::Invalid("Cannot park isolate"))?;
        let mut vm = Self {
            context: Some(context),
            retire_function: Some(retire_function),
            isolate: Some(isolate),
            quota,
            native: native_state,
            blob,
            cache,
            cache_hits: 0,
            owner: std::thread::current().id(),
        };
        let origin = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.;
        vm.text(&format!("globalThis.__timeOrigin={origin}; globalThis.__limits={};",json!({"max_host_ops":limits.max_host_ops,"max_ops":limits.max_ops,"max_op_bytes":limits.max_op_bytes})))?;
        vm.restore(storage, hot)?;
        Ok(vm)
    }
    pub fn memory(&self) -> crate::IsolateMemory {
        let mut isolate = self.isolate.as_ref().unwrap().lock();
        let stats = isolate.get_heap_statistics();
        let native = self.native.lock().unwrap();
        crate::IsolateMemory {
            used_heap_bytes: stats.used_heap_size(),
            physical_heap_bytes: stats.total_physical_size(),
            external_bytes: stats.external_memory(),
            fetch_reserved_bytes: native.reserved,
            engine_reserved_bytes: native.engine_reserved,
            snapshot_bytes: self.blob.len(),
        }
    }
    pub fn task_cancelled(&self) -> bool {
        self.native.lock().unwrap().cancellation.is_cancelled()
    }
    pub fn begin_task(&self, cancellation: Cancellation) {
        let mut n = self.native.lock().unwrap();
        n.cancellation = cancellation;
        n.sync_calls = 0;
    }
    pub fn handle(&self) -> v8::IsolateHandle {
        self.quota.handle.clone()
    }
    pub fn now(&self) -> f64 {
        self.native.lock().unwrap().start.elapsed().as_secs_f64() * 1000.
    }
    pub fn start(&mut self, code: &str) -> Result<v8::Global<v8::Value>> {
        self.compile_run(code, true)
    }
    fn compile_run(&mut self, code: &str, cached: bool) -> Result<v8::Global<v8::Value>> {
        if code.len() > self.native.lock().unwrap().limits.max_source_bytes {
            return Err(Error::Quota("source bytes"));
        }
        let key = cached.then(|| {
            CacheKey::new(
                v8::V8::get_version(),
                &self.blob,
                code.as_bytes(),
                &format!(
                    "{FLAGS};tag={}",
                    v8::script_compiler::cached_data_version_tag()
                ),
            )
        });
        let cache = key.and_then(|key| self.cache.get(key));
        let mut isolate = self.isolate.as_ref().unwrap().lock();
        v8::scope!(let hs,&mut *isolate);
        let context = v8::Local::new(hs, self.context.as_ref().unwrap());
        let cs = &mut v8::ContextScope::new(hs, context);
        let tc = std::pin::pin!(v8::TryCatch::new(cs));
        let s = &mut tc.init();
        let source = v8::String::new(s, code).ok_or(Error::Quota("source allocation"))?;
        let mut source = if let Some(cache) = &cache {
            v8::script_compiler::Source::new_with_cached_data(
                source,
                None,
                v8::script_compiler::CachedData::new(cache),
            )
        } else {
            v8::script_compiler::Source::new(source, None)
        };
        let compiled = v8::script_compiler::compile_unbound_script(
            s,
            &mut source,
            if cache.is_some() {
                v8::script_compiler::CompileOptions::ConsumeCodeCache
            } else {
                v8::script_compiler::CompileOptions::EagerCompile
            },
            v8::script_compiler::NoCacheReason::NoReason,
        );
        if let Some(script) = compiled {
            if source.get_cached_data().is_some_and(|c| !c.rejected()) {
                self.cache_hits += 1;
            }
            if cached && let Some(cache) = script.create_code_cache() {
                self.cache.insert(key.unwrap(), cache.to_vec());
            }
            if let Some(value) = script.bind_to_current_context(s).run(s) {
                return Ok(v8::Global::new(s, value));
            }
        }
        Err(if self.quota.exceeded.load(Ordering::Acquire) {
            Error::Quota("heap")
        } else {
            exception(s)
        })
    }
    pub fn settle(&self, value: &v8::Global<v8::Value>) -> Result<Option<String>> {
        let mut isolate = self.isolate.as_ref().unwrap().lock();
        v8::scope!(let hs,&mut *isolate);
        let context = v8::Local::new(hs, self.context.as_ref().unwrap());
        let cs = &mut v8::ContextScope::new(hs, context);
        let tc = std::pin::pin!(v8::TryCatch::new(cs));
        let s = &mut tc.init();
        s.perform_microtask_checkpoint();
        if s.is_execution_terminating() {
            return Err(if self.quota.exceeded.load(Ordering::Acquire) {
                Error::Quota("heap")
            } else {
                Error::Terminated
            });
        }
        let n = self.native.lock().unwrap();
        if s.get_heap_statistics()
            .external_memory()
            .saturating_add(n.reserved)
            .saturating_add(n.engine_reserved)
            > n.limits.external_bytes
        {
            return Err(Error::Quota("external memory"));
        }
        drop(n);
        let mut value = v8::Local::new(s, value);
        if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
            match promise.state() {
                v8::PromiseState::Pending => return Ok(None),
                v8::PromiseState::Rejected => {
                    return Err(Error::JavaScript(promise.result(s).to_rust_string_lossy(s)));
                }
                v8::PromiseState::Fulfilled => value = promise.result(s),
            }
        }
        value
            .to_string(s)
            .map(|text| Some(text.to_rust_string_lossy(s)))
            .ok_or_else(|| exception(s))
    }
    pub fn text(&mut self, code: &str) -> Result<String> {
        let value = self.compile_run(code, false)?;
        self.settle(&value)?
            .ok_or(Error::Invalid("Unexpected pending promise"))
    }
    fn control(&mut self, method: &str, args: Value) -> Result<String> {
        let mut isolate = self.isolate.as_ref().unwrap().lock();
        let value = {
            v8::scope!(let hs,&mut *isolate);
            let context = v8::Local::new(hs, self.context.as_ref().unwrap());
            let cs = &mut v8::ContextScope::new(hs, context);
            let tc = std::pin::pin!(v8::TryCatch::new(cs));
            let s = &mut tc.init();
            let result = (|| {
                let key = v8::String::new(s, "__pm")?;
                let object = context.global(s).get(s, key.into())?.to_object(s)?;
                let key = v8::String::new(s, method)?;
                let function =
                    v8::Local::<v8::Function>::try_from(object.get(s, key.into())?).ok()?;
                let text = v8::String::new(s, &args.to_string())?;
                let array = v8::Local::<v8::Array>::try_from(v8::json::parse(s, text)?).ok()?;
                let mut values = Vec::with_capacity(array.length() as usize);
                for i in 0..array.length() {
                    values.push(array.get_index(s, i)?);
                }
                function
                    .call(s, object.into(), &values)
                    .map(|value| v8::Global::new(s, value))
            })();
            result.ok_or_else(|| exception(s))?
        };
        drop(isolate);
        self.settle(&value)?
            .ok_or(Error::Invalid("Pending control operation"))
    }
    /// Keep cleanup callable even when extension code replaces global __pm.
    pub fn retire(&mut self) -> Result<()> {
        let mut isolate = self.isolate.as_ref().unwrap().lock();
        v8::scope!(let hs,&mut *isolate);
        let context = v8::Local::new(hs, self.context.as_ref().unwrap());
        let cs = &mut v8::ContextScope::new(hs, context);
        let tc = std::pin::pin!(v8::TryCatch::new(cs));
        let s = &mut tc.init();
        let function = v8::Local::new(s, self.retire_function.as_ref().unwrap());
        let receiver = v8::undefined(s).into();
        function
            .call(s, receiver, &[])
            .ok_or_else(|| exception(s))?;
        Ok(())
    }
    pub fn restore(&mut self, storage: &Value, hot: &Value) -> Result<()> {
        self.control("restore", json!([storage, hot]))?;
        Ok(())
    }
    pub fn effects(&mut self) -> Result<Value> {
        let data = self.control("take", json!([]))?;
        serde_json::from_str(&data).map_err(|e| Error::JavaScript(e.to_string()))
    }
    pub fn pump(&mut self, completed: Value) -> Result<()> {
        self.control("pump", json!([self.now(), completed]))?;
        Ok(())
    }
}
fn exception(s: &mut v8::PinnedRef<v8::TryCatch<v8::HandleScope>>) -> Error {
    if s.has_terminated() {
        Error::Terminated
    } else {
        Error::JavaScript(
            s.exception()
                .map(|e| e.to_rust_string_lossy(s))
                .unwrap_or_else(|| "Execution failed".into()),
        )
    }
}
impl Drop for Vm {
    fn drop(&mut self) {
        assert_eq!(
            self.owner,
            std::thread::current().id(),
            "V8 disposal must run on its owner"
        );
        if let Some(isolate) = &self.isolate {
            let mut guard = isolate.lock();
            drop(self.retire_function.take());
            drop(self.context.take());
            crate::modules::clear(&mut guard);
            guard.remove_near_heap_limit_callback(near_limit, 0);
        }
        drop(self.isolate.take());
    }
}
