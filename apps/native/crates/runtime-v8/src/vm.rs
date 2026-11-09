use crate::{
    CacheKey, Cancellation, CodeCache, EngineProxy, Error, FetchRequest, Generation, HostBudget,
    Limits, Result, SnapshotKind, runtime::Reservation,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    ffi::c_void,
    sync::{
        Arc, Mutex, Once, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

pub(crate) const SVELTE: &str = include_str!("../compiler/svelte-5.56.10.js");
const WEB: &str = include_str!("web.js");
const MIB: usize = 1024 * 1024;
/// Every V8 flag the runtime sets, in one place; the code cache keys on it.
///
/// - Every kind has its own custom snapshot, but isolates share V8's read-only
///   heap. Extending it independently per snapshot gives those snapshots
///   incompatible root offsets when another kind is restored.
/// - Heaps grow from a slice through the near-heap-limit callback, which only
///   the isolate's owner thread can run. A background thread that allocates at
///   the limit while the owner waits for work can't reach it, and V8 then
///   aborts the whole process. So nothing allocates on the JS heap off the
///   owner: Sparkplug compiles there, and Maglev builds its code and deopt data
///   there. (Turbofan allocates its code when it finalizes on the owner;
///   marking and sweeping never allocate; lazy-compile dispatch and turbolev
///   are off by default.)
macro_rules! v8_flags {
    () => {
        "--no-extensible-ro-snapshot --no-concurrent-sparkplug \
         --no-maglev-build-code-on-background --no-maglev-deopt-data-on-background"
    };
}
pub(crate) const FLAGS: &str = if cfg!(feature = "macos-sandbox") {
    concat!(v8_flags!(), ";sandbox=true")
} else {
    concat!(v8_flags!(), ";sandbox=false")
};
pub(crate) fn initialize() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        v8::V8::set_flags_from_string(v8_flags!());
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
    /// Fetch response bodies the next `pump` hands to JS by request id; JS
    /// takes each once with `__body(id)` as one ArrayBuffer.
    pub bodies: HashMap<u64, Vec<u8>>,
    blocked: Arc<Blocked>,
}
/// Time the running task spent blocked in synchronous engine calls (the
/// kernel channel, the engine, a base raster): the watchdog counts JS time
/// only (plan 3.5.2, D11).
#[derive(Default)]
pub(crate) struct Blocked(Mutex<(Duration, Option<Instant>, usize)>);
impl Blocked {
    fn enter(&self) {
        let mut b = self.0.lock().unwrap();
        if b.2 == 0 {
            b.1 = Some(Instant::now());
        }
        b.2 += 1;
    }
    fn exit(&self) {
        let mut b = self.0.lock().unwrap();
        b.2 = b.2.saturating_sub(1);
        if b.2 == 0
            && let Some(at) = b.1.take()
        {
            b.0 += at.elapsed();
        }
    }
    fn reset(&self) {
        *self.0.lock().unwrap() = Default::default();
    }
    /// Blocked so far in this task, including a call still waiting.
    pub(crate) fn total(&self) -> Duration {
        let b = self.0.lock().unwrap();
        b.0 + b.1.map_or(Duration::ZERO, |at| at.elapsed())
    }
}
/// The extension a stack capture found running one execution.
#[derive(Clone, Default)]
pub(crate) struct Culprit(Arc<Mutex<Option<Found>>>);
/// One execution and the extension found running it.
type Found = (u64, Option<String>);
impl Culprit {
    pub(crate) fn get(&self, execution: u64) -> Option<String> {
        match &*self.0.lock().unwrap() {
            Some((id, owner)) if *id == execution => owner.clone(),
            _ => None,
        }
    }
}
/// The realm's context, for code that runs inside it from an interrupt.
#[derive(Clone)]
struct MainContext(v8::Global<v8::Context>);
struct Capture {
    slot: Culprit,
    execution: u64,
}
/// Reads the running task's stack from inside it (an interrupt runs on the
/// isolate's thread at its next stack check, also in a tight loop) and waits
/// briefly for the answer.
pub(crate) fn capture_culprit(handle: &v8::IsolateHandle, slot: &Culprit, execution: u64) {
    let capture = Box::into_raw(Box::new(Capture {
        slot: slot.clone(),
        execution,
    }));
    if !handle.request_interrupt(capture_stack, capture as *mut c_void) {
        // SAFETY: V8 refused the request, so the callback never takes it.
        drop(unsafe { Box::from_raw(capture) });
        return;
    }
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        if slot
            .0
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|(id, _)| *id == execution)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
unsafe extern "C" fn capture_stack(mut raw: v8::UnsafeRawIsolatePtr, data: *mut c_void) {
    // SAFETY: `data` is the box `capture_culprit` leaked for exactly this callback.
    let capture = unsafe { Box::from_raw(data as *mut Capture) };
    // V8 invokes this with the isolate entered, on its currently locked owner.
    let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut raw) };
    let Some(MainContext(context)) = isolate.get_slot::<MainContext>().cloned() else {
        *capture.slot.0.lock().unwrap() = Some((capture.execution, None));
        return;
    };
    v8::scope!(let hs, isolate);
    let context = v8::Local::new(hs, &context);
    let s = &mut v8::ContextScope::new(hs, context);
    let mut owner = None;
    if let Some(trace) = v8::StackTrace::current_stack_trace(s, 64) {
        for index in 0..trace.get_frame_count() {
            let name = trace
                .get_frame(s, index)
                .and_then(|frame| frame.get_script_name(s))
                .map(|name| name.to_rust_string_lossy(s));
            if let Some(id) = name.as_deref().and_then(extension_of_module) {
                owner = Some(id.to_owned());
                break;
            }
        }
    }
    *capture.slot.0.lock().unwrap() = Some((capture.execution, owner));
}
/// `pmext://<id>/<generation>/<file>`: the extension a module belongs to.
pub(crate) fn extension_of_module(url: &str) -> Option<&str> {
    url.strip_prefix("pmext://")?
        .split('/')
        .next()
        .filter(|id| !id.is_empty())
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
    /// For heap statistics inside the near-limit callback, on the owner.
    isolate: IsolatePtr,
    guard: Arc<HeapGuard>,
    plan: HeapPlan,
}
/// An isolate's admission state, shared by the near-limit callback and the
/// isolate's slot (interrupt and post-task checks).
struct HeapGuard {
    exceeded: AtomicBool,
    /// The heap couldn't grow because the shared account was full, not
    /// because the isolate reached its own ceiling.
    account_full: AtomicBool,
    reservation: Reservation,
}
impl HeapGuard {
    /// Its own ceiling is the extension's limit; a full account is the app's
    /// memory for extensions running out, and names itself so.
    fn error(&self) -> Error {
        if self.account_full.load(Ordering::Acquire) {
            Error::Quota(HEAP_FULL)
        } else {
            Error::Quota("heap")
        }
    }
    /// V8 admits one large young object whatever the limit and only meets
    /// the limit when a collection promotes it. A live heap past the
    /// reservation gets that collection now, which grows the reservation
    /// through the near-limit callback or fails the isolate; whatever still
    /// lies past it after that fails the isolate, charged to the account
    /// until disposal.
    fn check(&self, isolate: &mut v8::Isolate) {
        if self.exceeded.load(Ordering::Acquire)
            || isolate.get_heap_statistics().used_heap_size() <= self.reservation.bytes()
        {
            return;
        }
        isolate.memory_pressure_notification(v8::MemoryPressureLevel::Critical);
        let used = isolate.get_heap_statistics().used_heap_size();
        let held = self.reservation.bytes();
        if used > held && !self.exceeded.swap(true, Ordering::AcqRel) {
            self.reservation.grace(used - held);
            isolate.terminate_execution();
        }
    }
}
struct Guarded(Arc<HeapGuard>);
/// The quota an isolate fails with when the shared account had no room left.
pub const HEAP_FULL: &str = "extension memory is full";
/// What a terminating isolate may still allocate while it unwinds, above
/// what its heap already holds.
const GRACE: usize = 8 * MIB;
struct IsolatePtr(v8::UnsafeRawIsolatePtr);
// SAFETY: only dereferenced in the near-limit callback, which V8 runs on the
// isolate's locked owner thread while the isolate is alive.
unsafe impl Send for IsolatePtr {}
unsafe impl Sync for IsolatePtr {}
/// An isolate's heap in V8's terms. Admission counts the whole heap limit:
/// the young generation, fixed from the ceiling, plus the old generation's
/// current limit, which starts at the initial slice and grows on demand.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeapPlan {
    young: usize,
    initial_old: usize,
    ceiling_old: usize,
}
impl HeapPlan {
    pub fn new(limits: &Limits) -> Self {
        let params = v8::CreateParams::default().heap_limits(0, limits.heap_bytes);
        let young = params.max_young_generation_size_in_bytes();
        let ceiling_old = params.max_old_generation_size_in_bytes();
        let initial_old = limits
            .initial_heap_bytes
            .saturating_sub(young)
            .clamp(ceiling_old.min(4 * MIB), ceiling_old);
        Self {
            young,
            initial_old,
            ceiling_old,
        }
    }
    pub fn initial(&self) -> usize {
        self.young + self.initial_old
    }
    /// The next old-generation limits to ask the account for, largest
    /// first: doubling (8 to 256 MiB at a time), then 8 MiB, never past the
    /// ceiling.
    fn steps(&self, current: usize) -> [usize; 2] {
        let step = current.clamp(8 * MIB, 256 * MIB);
        [
            current.saturating_add(step).min(self.ceiling_old),
            current.saturating_add(8 * MIB).min(self.ceiling_old),
        ]
    }
}
unsafe extern "C" fn near_limit(data: *mut c_void, current: usize, _initial: usize) -> usize {
    // Box lives until callback removal, on the same owner thread as this callback.
    let quota = unsafe { &*(data as *const Quota) };
    let guard = &quota.guard;
    // One allocation can land far past the limit (a large young object is
    // admitted whatever the limit, then promoted), so a new limit must cover
    // what the heap holds now: V8 aborts the process if the limit it gets
    // back is still below the heap.
    let mut raw = quota.isolate.0;
    let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut raw) };
    let held = isolate.get_heap_statistics().total_heap_size();
    if !guard.exceeded.load(Ordering::Acquire) {
        let floor = held.saturating_add(GRACE);
        let mut refused = false;
        for next in quota.plan.steps(current) {
            let next = next.max(floor);
            if next <= quota.plan.ceiling_old && next > current {
                if guard.reservation.grow(next - current) {
                    return next;
                }
                refused = true;
            }
        }
        guard.account_full.store(refused, Ordering::Release);
    }
    // Past its ceiling or the account: the isolate fails. The grace covers
    // the heap as it stands while it unwinds, charged until disposal.
    guard.exceeded.store(true, Ordering::Release);
    quota.handle.terminate_execution();
    let next = current.max(held).saturating_add(GRACE);
    guard.reservation.grace(next - current);
    next
}
/// Interrupts never reenter JS. Only read heap statistics and terminate.
pub(crate) unsafe extern "C" fn check_external(mut raw: v8::UnsafeRawIsolatePtr, _: *mut c_void) {
    // V8 invokes this with the isolate entered, on its currently locked owner.
    let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut raw) };
    if let Some(guard) = isolate.get_slot::<Guarded>().map(|g| g.0.clone()) {
        guard.check(isolate);
    }
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
    // A Rust panic can't unwind through V8's frames (the process would abort
    // with "failed to initiate panic"): a panicking host operation ends this
    // generation instead, and the panic is reported by the panic hook.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        serde_json::from_str(&arg)
            .map_err(|e| e.to_string())
            .and_then(|v| {
                if op == "engine" {
                    engine_op(&state, v)
                } else {
                    host_op(&mut state.lock().unwrap(), &op, v).map(|v| v.to_string())
                }
            })
    }));
    let Ok(result) = result else {
        s.terminate_execution();
        return;
    };
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
/// `__bytes(operation, view, input)`: an engine call carrying one byte payload
/// (an encoded image, a file), copied out of V8 once instead of as JSON.
fn bytes_op(s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let op = a.get(0).to_rust_string_lossy(s);
    let bytes = if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(a.get(1)) {
        let mut out = vec![0; view.byte_length()];
        view.copy_contents(&mut out);
        out
    } else if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(a.get(1)) {
        let store = buffer.get_backing_store();
        store
            .data()
            .map(|data| {
                // SAFETY: the backing store is alive for this call and `byte_length` long.
                unsafe {
                    std::slice::from_raw_parts(data.as_ptr() as *const u8, store.byte_length())
                }
                .to_vec()
            })
            .unwrap_or_default()
    } else {
        throw(s, "Host byte payload must be an ArrayBuffer or view");
        return;
    };
    let input = a.get(2).to_rust_string_lossy(s);
    let state = s.get_slot::<Native>().unwrap().0.clone();
    let limits = state.lock().unwrap().limits.clone();
    if bytes.len() > limits.max_response_bytes || input.len() > limits.max_op_bytes {
        throw(s, "Host argument quota");
        return;
    }
    let result = serde_json::from_str::<Value>(&input)
        .map_err(|e| e.to_string())
        .and_then(|input| {
            engine_call(&state, &op, |engine, generation, budget| {
                engine
                    .call_bytes(generation, &op, &bytes, &input, budget)
                    .map(|value| value.to_string())
            })
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
/// `__read(operation, input)`: bytes the host holds, as a new `ArrayBuffer`.
fn read_op(s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let op = a.get(0).to_rust_string_lossy(s);
    let input = a.get(1).to_rust_string_lossy(s);
    let state = s.get_slot::<Native>().unwrap().0.clone();
    let max = state.lock().unwrap().limits.max_response_bytes;
    let mut bytes = None;
    let result = serde_json::from_str::<Value>(&input)
        .map_err(|e| e.to_string())
        .and_then(|input| {
            engine_call(&state, &op, |engine, generation, mut budget| {
                budget.max_response_bytes = max;
                let read = engine.read_bytes(generation, &op, &input, budget)?;
                if read.len() > max {
                    return Err(crate::Error::Quota("Engine response quota"));
                }
                bytes = Some(read);
                Ok(String::new())
            })
        });
    if state.lock().unwrap().cancellation.is_cancelled() {
        s.terminate_execution();
        return;
    }
    match (result, bytes) {
        (Ok(_), Some(bytes)) => {
            // Inside the V8 sandbox: allocated by the isolate, then filled.
            let buffer = v8::ArrayBuffer::new(s, bytes.len());
            if let Some(data) = buffer.get_backing_store().data() {
                // SAFETY: a fresh buffer of exactly `bytes.len()` bytes no one else sees yet.
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.as_ptr().cast(), bytes.len())
                };
            }
            rv.set(buffer.into());
        }
        (Err(error), _) => throw(s, &error),
        _ => throw(s, "Host read failed"),
    }
}
/// A fetch response body, once, as an ArrayBuffer the isolate allocated and
/// Rust filled (as `__read`): a JSON number array costs dozens of bytes per byte.
fn body_op(s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let id = a.get(0).number_value(s).unwrap_or(-1.);
    let state = s.get_slot::<Native>().unwrap().0.clone();
    let Some(bytes) = (id >= 0.)
        .then(|| state.lock().unwrap().bodies.remove(&(id as u64)))
        .flatten()
    else {
        throw(s, "No response body");
        return;
    };
    let buffer = v8::ArrayBuffer::new(s, bytes.len());
    if let Some(data) = buffer.get_backing_store().data() {
        // SAFETY: a fresh buffer of exactly `bytes.len()` bytes no one else sees yet.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.as_ptr().cast(), bytes.len()) };
    }
    rv.set(buffer.into());
}
/// The reply as JSON text, serialized once.
fn engine_op(state: &Arc<Mutex<NativeState>>, input: Value) -> std::result::Result<String, String> {
    let operation = input["operation"]
        .as_str()
        .ok_or("Invalid engine operation")?;
    engine_call(state, operation, |engine, generation, budget| {
        engine.call_text(generation, operation, &input["input"], budget)
    })
}
/// Meters, reserves and bounds one synchronous engine call.
fn engine_call(
    state: &Arc<Mutex<NativeState>>,
    operation: &str,
    call: impl FnOnce(&dyn EngineProxy, &Generation, HostBudget) -> crate::Result<String>,
) -> std::result::Result<String, String> {
    let (engine, generation, budget, _reservation) = {
        let mut n = state.lock().unwrap();
        let engine = n.engine.clone().ok_or("Engine service is not attached")?;
        if engine.metered(&n.generation, operation) {
            if n.sync_calls >= n.limits.max_host_ops {
                return Err("Engine operation quota".into());
            }
            n.sync_calls += 1;
        }
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
    let blocked = state.lock().unwrap().blocked.clone();
    blocked.enter();
    let text = call(&*engine, &generation, budget.clone());
    blocked.exit();
    let text = text.map_err(|e| e.to_string())?;
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
            if !start && (!n.limits.queue_fetches || !fits_alone) {
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
    cache_prefix: Option<crate::cache::CachePrefix>,
    owner: std::thread::ThreadId,
}
impl Vm {
    pub fn new(
        kind: SnapshotKind,
        config: &crate::Config,
        storage: &Value,
        hot: &Value,
        generation: Generation,
        reservation: Reservation,
    ) -> Result<Self> {
        let (limits, cache, engine) = (
            config.limits.clone(),
            config.cache.clone(),
            config.engine.clone(),
        );
        let blob = snapshot(kind);
        let plan = HeapPlan::new(&limits);
        debug_assert_eq!(reservation.bytes(), plan.initial());
        let mut isolate = v8::Isolate::new(
            v8::CreateParams::default()
                .snapshot_blob(blob.as_ref().clone().into())
                .heap_limits(0, limits.heap_bytes)
                .set_max_old_generation_size_in_bytes(plan.initial_old),
        );
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        let quota = Box::new(Quota {
            handle: isolate.thread_safe_handle(),
            // SAFETY: kept only while the isolate lives (see `IsolatePtr`).
            isolate: IsolatePtr(unsafe { isolate.as_raw_isolate_ptr() }),
            guard: Arc::new(HeapGuard {
                exceeded: AtomicBool::new(false),
                account_full: AtomicBool::new(false),
                reservation,
            }),
            plan,
        });
        isolate.set_slot(Guarded(quota.guard.clone()));
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
            bodies: HashMap::new(),
            blocked: Arc::default(),
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
                ("__bytes", v8::Function::new(cs, bytes_op).unwrap()),
                ("__read", v8::Function::new(cs, read_op).unwrap()),
                ("__body", v8::Function::new(cs, body_op).unwrap()),
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
        isolate.set_slot(MainContext(context.clone()));
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
            cache_prefix: None,
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
            reserved_heap_bytes: self.quota.guard.reservation.bytes(),
        }
    }
    /// True once the isolate holds more than its initial slice.
    pub fn grown(&self) -> bool {
        self.quota.guard.reservation.bytes() > self.quota.plan.initial()
    }
    /// An idle isolate gives back what it grew past its live size: a full
    /// collection, then the old-generation limit comes down to half again
    /// above what lives (never under the initial slice). V8 keeps any limit
    /// it can't honour, so the account follows V8's own limit, not the ask.
    pub fn trim(&mut self) {
        if !self.grown() || self.quota.guard.exceeded.load(Ordering::Acquire) {
            return;
        }
        let quota = &*self.quota as *const Quota as *mut c_void;
        let mut isolate = self.isolate.as_ref().unwrap().lock();
        isolate.low_memory_notification();
        let before = isolate.get_heap_statistics();
        let used = before.used_heap_size();
        let target = (used + used / 2 + 4 * MIB)
            .next_multiple_of(MIB)
            .max(self.quota.plan.initial_old);
        isolate.remove_near_heap_limit_callback(near_limit, target);
        isolate.add_near_heap_limit_callback(near_limit, quota);
        let after = isolate.get_heap_statistics().heap_size_limit();
        self.quota
            .guard
            .reservation
            .shrink(before.heap_size_limit().saturating_sub(after));
    }
    pub fn task_cancelled(&self) -> bool {
        self.native.lock().unwrap().cancellation.is_cancelled()
    }
    pub fn begin_task(&self, cancellation: Cancellation) {
        let mut n = self.native.lock().unwrap();
        n.cancellation = cancellation;
        n.sync_calls = 0;
        n.blocked.reset();
    }
    /// The running task's time blocked in synchronous engine calls.
    pub(crate) fn blocked(&self) -> Arc<Blocked> {
        self.native.lock().unwrap().blocked.clone()
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
            self.cache_prefix
                .get_or_insert_with(|| {
                    CacheKey::prefix(
                        v8::V8::get_version(),
                        &self.blob,
                        &format!(
                            "{FLAGS};tag={}",
                            v8::script_compiler::cached_data_version_tag()
                        ),
                    )
                })
                .key(code.as_bytes())
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
        Err(if self.quota.guard.exceeded.load(Ordering::Acquire) {
            self.quota.guard.error()
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
        self.quota.guard.check(s);
        if s.is_execution_terminating() || self.quota.guard.exceeded.load(Ordering::Acquire) {
            return Err(if self.quota.guard.exceeded.load(Ordering::Acquire) {
                self.quota.guard.error()
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
