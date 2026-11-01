//! Native view recovery state machine. Actions are queued to a main-thread
//! WebKit adapter and the kernel/V8 worker, never synchronously entered here.
use crate::{Error, Result, identity::DataStoreIdentity};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewToken {
    pub data_store: DataStoreIdentity,
    pub view: String,
    pub generation: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Failure {
    Crash,
    Hang,
    UserStop,
    ReadyTimeout,
    PlatformFailure,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewStatus {
    Starting,
    Ready,
    Sealing,
    Stopping,
    WaitingToRestart,
    Disabled,
    Closed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Create {
        token: ViewToken,
    },
    /// Seal acknowledged gestures and cancel callbacks/registrations. View
    /// crashes do NOT destroy the V8 realm. The bridge invalidates this view.
    Seal {
        token: ViewToken,
    },
    Stop {
        token: ViewToken,
    },
    ScheduleRestart {
        token: ViewToken,
        not_before_ms: u64,
    },
    RestoreSnapshot {
        token: ViewToken,
    },
    Disabled {
        token: ViewToken,
        reason: Failure,
    },
    Close {
        token: ViewToken,
    },
}
/// The WebKit implementation must retain one separate data-store object per
/// key, including while no views exist. No global/default store is permitted.
/// Opaque renderer origin; deny navigation/popups/uploads/downloads/browser
/// permissions/dialogs; run only bundled nonce applier. Queue, don't wait on JS.
pub trait ViewPlatform {
    fn enqueue(&mut self, action: &Action) -> Result<()>;
}
#[derive(Debug, Clone, Copy)]
pub struct RecoveryPolicy {
    pub max_restarts: usize,
    pub window_ms: u64,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub ready_timeout_ms: u64,
}
impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 3,
            window_ms: 60_000,
            initial_delay_ms: 250,
            max_delay_ms: 8_000,
            ready_timeout_ms: 20_000,
        }
    }
}
#[derive(Debug)]
pub struct View {
    token: ViewToken,
    status: ViewStatus,
    policy: RecoveryPolicy,
    failures: VecDeque<u64>,
    last_ms: u64,
    deadline_ms: u64,
    reason: Option<Failure>,
    restart_at: Option<u64>,
}
impl View {
    /// Generation is allocated by a process/session-unique kernel allocator.
    /// A restarted application must not reuse an old view capability or token.
    pub fn start(token: ViewToken, now_ms: u64, policy: RecoveryPolicy) -> Result<(Self, Action)> {
        if token.view.is_empty()
            || token.view.len() > 128
            || token.generation == 0
            || policy.window_ms == 0
            || policy.ready_timeout_ms == 0
            || policy.initial_delay_ms > policy.max_delay_ms
        {
            return Err(Error::Invalid("view config"));
        }
        let create = Action::Create {
            token: token.clone(),
        };
        Ok((
            Self {
                token,
                status: ViewStatus::Starting,
                policy,
                failures: VecDeque::new(),
                last_ms: now_ms,
                deadline_ms: now_ms.saturating_add(policy.ready_timeout_ms),
                reason: None,
                restart_at: None,
            },
            create,
        ))
    }
    pub fn token(&self) -> &ViewToken {
        &self.token
    }
    pub fn status(&self) -> ViewStatus {
        self.status
    }
    fn clock(&mut self, now: u64) -> Result<()> {
        if now < self.last_ms {
            return Err(Error::Invalid("non-monotonic clock"));
        }
        self.last_ms = now;
        Ok(())
    }
    fn current(&self, generation: u64) -> Result<()> {
        if generation != self.token.generation {
            return Err(Error::Stale);
        }
        Ok(())
    }
    pub fn ready(&mut self, generation: u64, now: u64) -> Result<Action> {
        self.current(generation)?;
        if self.status != ViewStatus::Starting {
            return Err(Error::Stale);
        }
        self.clock(now)?;
        if now >= self.deadline_ms {
            return Err(Error::Invalid("ready deadline"));
        }
        self.status = ViewStatus::Ready;
        Ok(Action::RestoreSnapshot {
            token: self.token.clone(),
        })
    }
    /// Hang evidence must be attributed JS execution time, not a background
    /// wall-clock heartbeat. Store hangs stop automatically (compat-host policy).
    pub fn fail(&mut self, generation: u64, reason: Failure, now: u64) -> Result<Action> {
        self.current(generation)?;
        if !matches!(self.status, ViewStatus::Starting | ViewStatus::Ready) {
            return Err(Error::Stale);
        }
        self.clock(now)?;
        while self
            .failures
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= self.policy.window_ms)
        {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
        self.reason = Some(reason);
        self.status = ViewStatus::Sealing;
        Ok(Action::Seal {
            token: self.token.clone(),
        })
    }
    /// Call only after kernel confirms sealing/revocation. A failed seal leaves
    /// Sealing and can retry; never kill before acknowledged work is safe.
    pub fn sealed(&mut self, generation: u64) -> Result<Action> {
        self.current(generation)?;
        if self.status != ViewStatus::Sealing {
            return Err(Error::Stale);
        }
        self.status = ViewStatus::Stopping;
        Ok(Action::Stop {
            token: self.token.clone(),
        })
    }
    /// Platform confirms the old content process exited, or detachment is fully
    /// complete for an already crashed process. An API invocation isn't proof.
    pub fn stopped(&mut self, generation: u64, now: u64) -> Result<Action> {
        self.current(generation)?;
        if self.status != ViewStatus::Stopping {
            return Err(Error::Stale);
        }
        self.clock(now)?;
        let reason = self.reason.ok_or(Error::Invalid("failure reason"))?;
        if reason == Failure::UserStop || self.failures.len() > self.policy.max_restarts {
            self.status = ViewStatus::Disabled;
            return Ok(Action::Disabled {
                token: self.token.clone(),
                reason,
            });
        }
        let power = u32::try_from(self.failures.len().saturating_sub(1))
            .unwrap_or(u32::MAX)
            .min(63);
        let delay = self
            .policy
            .initial_delay_ms
            .saturating_mul(1u64 << power)
            .min(self.policy.max_delay_ms);
        let at = now.saturating_add(delay);
        self.restart_at = Some(at);
        self.status = ViewStatus::WaitingToRestart;
        Ok(Action::ScheduleRestart {
            token: self.token.clone(),
            not_before_ms: at,
        })
    }
    /// Explicit restart after disabled/user Stop is allowed; generation must be
    /// a fresh kernel allocation, not a page-supplied counter.
    pub fn restart(&mut self, generation: u64, now: u64) -> Result<Action> {
        if !matches!(
            self.status,
            ViewStatus::WaitingToRestart | ViewStatus::Disabled
        ) {
            return Err(Error::Stale);
        }
        if generation <= self.token.generation {
            return Err(Error::Stale);
        }
        if self.status == ViewStatus::WaitingToRestart && self.restart_at.is_some_and(|at| now < at)
        {
            return Err(Error::Backpressure);
        }
        self.clock(now)?;
        if self.status == ViewStatus::Disabled {
            self.failures.clear();
        }
        self.token.generation = generation;
        self.status = ViewStatus::Starting;
        self.deadline_ms = now.saturating_add(self.policy.ready_timeout_ms);
        self.restart_at = None;
        Ok(Action::Create {
            token: self.token.clone(),
        })
    }
    pub fn tick(&mut self, now: u64) -> Result<Option<Action>> {
        self.clock(now)?;
        if self.status == ViewStatus::Starting && now >= self.deadline_ms {
            return self
                .fail(self.token.generation, Failure::ReadyTimeout, now)
                .map(Some);
        }
        Ok(None)
    }
    pub fn close(&mut self) -> Result<Action> {
        if self.status == ViewStatus::Closed {
            return Err(Error::Stale);
        }
        self.status = ViewStatus::Closed;
        Ok(Action::Close {
            token: self.token.clone(),
        })
    }
}
/// Linux double: stable store handles, explicit platform completion events,
/// injectable enqueue failure. It does not simulate WebKit process behavior.
#[derive(Debug, Default)]
pub struct MemoryPlatform {
    stores: BTreeMap<String, u64>,
    pub actions: Vec<Action>,
    pub fail_next: bool,
}
impl MemoryPlatform {
    pub fn store_handle(&self, key: &str) -> Option<u64> {
        self.stores.get(key).copied()
    }
}
impl ViewPlatform for MemoryPlatform {
    fn enqueue(&mut self, action: &Action) -> Result<()> {
        if self.fail_next {
            self.fail_next = false;
            return Err(Error::Platform("injected enqueue failure".into()));
        }
        if let Action::Create { token } = action {
            let next = self.stores.len() as u64 + 1;
            self.stores
                .entry(token.data_store.key.clone())
                .or_insert(next);
        }
        self.actions.push(action.clone());
        Ok(())
    }
}
