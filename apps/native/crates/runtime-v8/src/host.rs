use crate::{Result, SnapshotKind};
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Generation {
    pub realm: String,
    pub number: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogDecision {
    Wait,
    Stop,
}
#[derive(Debug, Clone)]
pub struct WatchdogPolicy {
    pub notice: Duration,
    pub prompt: Duration,
    pub repeat: Duration,
}
impl Default for WatchdogPolicy {
    fn default() -> Self {
        Self {
            notice: Duration::from_secs(2),
            prompt: Duration::from_secs(8),
            repeat: Duration::from_secs(8),
        }
    }
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub max_response_bytes: usize,
    pub deadline: Instant,
}
#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub status: u16,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
/// Nonblocking poll; implementations enforce redirects, credentials, permissions,
/// deadlines and byte limits, including streamed bytes before accumulation.
pub trait HostRequest: Send {
    fn poll(&mut self) -> Option<Result<FetchResponse>>;
    fn cancel(&mut self);
}
/// Start must enqueue work and return promptly. Network never runs in a V8 callback.
pub trait HostProxy: Send + Sync + 'static {
    fn fetch(
        &self,
        generation: &Generation,
        request: FetchRequest,
        cancel: Cancellation,
    ) -> Result<Box<dyn HostRequest>>;
    fn console(&self, generation: &Generation, level: &str, message: &str);
}
/// Implement with kernel actor messages. commit_activation must atomically publish
/// the op list + storage + registrations, or publish nothing. It must not call JS.
pub trait KernelHooks: Send + Sync + 'static {
    fn load_storage(&self, realm: &str) -> Result<Value>;
    fn commit_activation(
        &self,
        previous: Option<&Generation>,
        next: &Generation,
        ops: &[Value],
        storage: &Value,
    ) -> Result<()>;
    /// History/export pins may call retired closures; storage must not overwrite the active generation.
    fn commit_retained(&self, generation: &Generation, ops: &[Value]) -> Result<()>;
    fn revoke_generation(&self, generation: &Generation);
    fn seal_open_gestures(&self, generation: &Generation);
}
/// Hooks enqueue native UI notices/prompts; they never synchronously wait for UI.
/// Wait keeps execution alive. A later explicit Stop uses Realm::stop(execution).
pub trait WatchdogHooks: Send + Sync + 'static {
    fn notice(&self, generation: &Generation, execution: u64);
    fn prompt(&self, generation: &Generation, execution: u64) -> WatchdogDecision;
}
#[derive(Debug, Clone)]
pub struct Limits {
    pub heap_bytes: usize,
    pub external_bytes: usize,
    pub max_source_bytes: usize,
    pub max_host_ops: usize,
    pub max_op_bytes: usize,
    pub max_ops: usize,
    pub max_response_bytes: usize,
    pub host_timeout: Duration,
    pub queue_capacity: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            heap_bytes: 64 * 1024 * 1024,
            external_bytes: 32 * 1024 * 1024,
            max_source_bytes: 8 * 1024 * 1024,
            max_host_ops: 64,
            max_op_bytes: 1024 * 1024,
            max_ops: 4096,
            max_response_bytes: 8 * 1024 * 1024,
            host_timeout: Duration::from_secs(30),
            queue_capacity: 64,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RealmKind {
    BuiltinEditor,
    BuiltinRest,
    BuiltinAgent,
    V4Extension(String),
    Compat,
    Compiler,
    Store(String),
}
impl RealmKind {
    pub fn snapshot_kind(&self) -> SnapshotKind {
        match self {
            Self::BuiltinEditor => SnapshotKind::Editor,
            Self::BuiltinRest => SnapshotKind::Rest,
            Self::BuiltinAgent => SnapshotKind::Agent,
            Self::V4Extension(_) => SnapshotKind::Extension,
            Self::Compat => SnapshotKind::Compat,
            Self::Compiler => SnapshotKind::Compiler,
            Self::Store(_) => SnapshotKind::Store,
        }
    }
}
/// C-MATRIX's entire connected editor closure. Unproven remaining callbacks are
/// conservatively co-located. Agent is explicitly a native proposal, opt-in only.
pub fn builtin_cluster(id: &str) -> RealmKind {
    match id {
        "theme" | "effects" | "transitions" => RealmKind::BuiltinRest,
        _ => RealmKind::BuiltinEditor,
    }
}

/// A synchronous engine operation runs only on a V8 owner, never the UI thread.
/// Implementations must check cancellation/deadline while waiting, enforce the
/// output budget before allocating, and never synchronously enter JS or main.
#[derive(Clone)]
pub struct HostBudget {
    pub cancellation: Cancellation,
    pub deadline: Instant,
    pub max_response_bytes: usize,
}
impl HostBudget {
    pub fn check(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            return Err(crate::Error::Terminated);
        }
        if Instant::now() >= self.deadline {
            return Err(crate::Error::Host("Engine operation timed out".into()));
        }
        Ok(())
    }
}
pub trait EngineProxy: Send + Sync + 'static {
    fn call(
        &self,
        generation: &Generation,
        operation: &str,
        input: &Value,
        budget: HostBudget,
    ) -> Result<Value>;
}

/// V8's statistics, not RSS or an admission reservation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IsolateMemory {
    pub used_heap_bytes: usize,
    pub physical_heap_bytes: usize,
    pub external_bytes: usize,
    /// Response capacity reserved before native fetch jobs start. Not RSS and
    /// not part of V8's external_memory statistic.
    pub fetch_reserved_bytes: usize,
    /// Capacity held while a synchronous native engine call executes.
    pub engine_reserved_bytes: usize,
    pub snapshot_bytes: usize,
}
#[derive(Debug, Clone)]
pub struct GenerationMemory {
    pub generation: Generation,
    pub retired: bool,
    pub memory: IsolateMemory,
}
