use crate::{
    vm::{self, HEAP_FULL, HeapPlan, Vm},
    *,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

pub(crate) struct Completion<T> {
    value: Option<Result<T>>,
    waker: Option<Waker>,
}
pub(crate) type SharedCompletion<T> = Arc<Mutex<Completion<T>>>;
/// Nonblocking result: poll as a Future or inspect with try_take; worker
/// threads may block on it with `wait_until`.
pub struct Ticket<T> {
    pub execution: u64,
    shared: SharedCompletion<T>,
}
impl<T> Ticket<T> {
    pub fn try_take(&self) -> Option<Result<T>> {
        self.shared.lock().unwrap().value.take()
    }
}
impl<T> Future for Ticket<T> {
    type Output = Result<T>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut c = self.shared.lock().unwrap();
        if let Some(value) = c.value.take() {
            Poll::Ready(value)
        } else {
            c.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
/// Blocks a worker thread until `future` completes or `deadline` passes, and
/// leaves the future with the caller on timeout. Never call this on AppKit's
/// main thread: it must keep polling.
pub fn wait_until<F: Future + Unpin>(future: &mut F, deadline: Instant) -> Option<F::Output> {
    struct Unpark(thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(value) = Pin::new(&mut *future).poll(&mut cx) {
            return Some(value);
        }
        let left = deadline.checked_duration_since(Instant::now())?;
        thread::park_timeout(left);
    }
}
pub(crate) fn ticket<T>(id: u64) -> (Ticket<T>, SharedCompletion<T>) {
    let shared = Arc::new(Mutex::new(Completion {
        value: None,
        waker: None,
    }));
    (
        Ticket {
            execution: id,
            shared: shared.clone(),
        },
        shared,
    )
}
pub(crate) fn complete<T>(shared: SharedCompletion<T>, value: Result<T>) {
    let waker = {
        let mut c = shared.lock().unwrap();
        c.value = Some(value);
        c.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub generation: Generation,
    pub value: String,
    pub cache_hits: usize,
}
#[derive(Debug, Clone)]
pub struct ActiveExecution {
    pub execution: u64,
    pub generation: Generation,
    pub elapsed: Duration,
}
#[derive(Clone)]
pub struct GenerationPin {
    pub generation: Generation,
    live: Option<Arc<()>>,
    notifier: Weak<Mailbox>,
}
impl std::fmt::Debug for GenerationPin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.generation.fmt(f)
    }
}
impl Drop for GenerationPin {
    fn drop(&mut self) {
        drop(self.live.take());
        if let Some(tx) = self.notifier.upgrade() {
            let _ = tx.send(Command::Wake, Delivery::Immediate);
        }
    }
}
struct Execution {
    id: u64,
    handle: v8::IsolateHandle,
    cancellation: Cancellation,
    generation: Generation,
    epoch: u64,
    started: Instant,
    /// Time blocked in engine calls, which the watchdog doesn't count.
    blocked: Arc<crate::vm::Blocked>,
    /// The running thread's CPU clock, and its reading at the start.
    clock: Option<(ThreadClock, Duration)>,
    noticed: bool,
    next_prompt: Duration,
}
impl Execution {
    /// How long the task has run, as the watchdog counts it: time on a CPU,
    /// so a thread a busy machine starves of CPU (it waits to be scheduled)
    /// isn't taken for a hang; wall time less engine calls where the clock
    /// can't be read, and past `backstop` (a task stuck in a wait).
    fn ran(&self, backstop: Duration) -> Duration {
        let wall = self.started.elapsed().saturating_sub(self.blocked.total());
        match self
            .clock
            .and_then(|(clock, start)| Some(clock.cpu()?.saturating_sub(start)))
        {
            Some(cpu) if wall < backstop => cpu.min(wall),
            _ => wall,
        }
    }
}
/// A thread's CPU time (user and system), readable from another thread.
#[derive(Clone, Copy)]
struct ThreadClock(#[cfg(target_os = "macos")] libc::mach_port_t);
impl ThreadClock {
    /// The calling thread's clock.
    fn current() -> Option<Self> {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: the port of the calling thread, owned by its pthread.
            let port = unsafe { libc::pthread_mach_thread_np(libc::pthread_self()) };
            (port != 0).then_some(Self(port))
        }
        #[cfg(not(target_os = "macos"))]
        None
    }
    fn cpu(&self) -> Option<Duration> {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: a THREAD_BASIC_INFO sized out-buffer and its count.
            #[allow(deprecated)]
            let (status, info) = unsafe {
                let mut info: libc::thread_basic_info = std::mem::zeroed();
                let mut count = libc::THREAD_BASIC_INFO_COUNT;
                let status = libc::thread_info(
                    self.0,
                    libc::THREAD_BASIC_INFO as libc::thread_flavor_t,
                    (&raw mut info).cast(),
                    &mut count,
                );
                (status, info)
            };
            #[allow(deprecated)]
            let time = |t: libc::time_value_t| {
                Duration::from_secs(t.seconds.max(0) as u64)
                    + Duration::from_micros(t.microseconds.max(0) as u64)
            };
            (status == libc::KERN_SUCCESS).then(|| time(info.user_time) + time(info.system_time))
        }
        #[cfg(not(target_os = "macos"))]
        None
    }
}
struct Control {
    active: Mutex<Option<Execution>>,
    closing: AtomicBool,
    disposed: AtomicBool,
    wake: Condvar,
    ids: AtomicU64,
    /// The extension the watchdog found running a task it prompts about.
    culprit: crate::vm::Culprit,
    /// Whether the isolate was admitted to the heap account (`None`: still
    /// waiting its turn), and the signal when it is decided.
    admitted: Mutex<Option<Result<()>>>,
    decided: Condvar,
    panel_memory: Option<PanelMemory>,
}
struct PanelMemory {
    error: Mutex<Option<Error>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl Control {
    /// The owner tells the panel host even when the caller discarded the
    /// evaluation ticket, or the failure came from a timer.
    fn report_panel_memory(
        &self,
        error: &Error,
        live: Option<&Live>,
        name: &str,
        limits: &Limits,
    ) -> bool {
        let Some(panel) = &self.panel_memory else {
            return false;
        };
        if !error.is_memory() {
            return false;
        }
        if let Some(live) = live {
            let memory = live.vm.memory();
            eprintln!(
                "{name} ran out of memory ({error}): heap used {} bytes, reserved {} bytes, ceiling {} bytes; external {} bytes, ceiling {} bytes",
                memory.used_heap_bytes,
                memory.reserved_heap_bytes,
                limits.heap_bytes,
                memory.external_bytes,
                limits.external_bytes,
            );
        } else {
            eprintln!(
                "{name} ran out of memory during startup ({error}): heap ceiling {} bytes",
                limits.heap_bytes
            );
        }
        *panel.error.lock().unwrap() = Some(error.clone());
        (panel.wake)();
        true
    }
}
struct Inner {
    mailbox: Arc<Mailbox>,
    control: Arc<Control>,
    max_source: usize,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.control.closing.store(true, Ordering::Release);
        self.mailbox.close();
        self.control.wake.notify_all();
        if let Some(e) = &*self.control.active.lock().unwrap() {
            e.cancellation.cancel();
            e.handle.terminate_execution();
        }
    }
}
#[derive(Clone)]
pub struct Realm {
    inner: Arc<Inner>,
}
enum Command {
    MemoryPressure,
    Measure {
        reply: SharedCompletion<Vec<GenerationMemory>>,
    },
    Wake,
    Eval {
        id: u64,
        source: Source,
        expected: Option<Generation>,
        svelte: Option<(String, SvelteUnit)>,
        reply: SharedCompletion<Evaluation>,
    },
    Reload {
        id: u64,
        source: String,
        reply: SharedCompletion<Evaluation>,
    },
    Pin {
        reply: SharedCompletion<GenerationPin>,
    },
    Retained {
        id: u64,
        pin: GenerationPin,
        source: String,
        reply: SharedCompletion<Evaluation>,
    },
}
enum Source {
    Text(String),
    Prepare(Box<dyn FnOnce() -> Result<String> + Send>),
}

/// Use a stable key for each stream of replaceable observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateChannel {
    Publication,
    TimelineOutline,
    ViewerFrame,
    ExportProgress,
    Pointer { panel: String, pointer: u64 },
    Parameter(String),
    Range { panel: String, target: u64 },
    Control { panel: String, target: u64 },
}

enum Delivery {
    Immediate,
    Discrete(Option<StateChannel>),
    Latest(StateChannel),
    Notify(StateChannel),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct QueueStats {
    pub busy_rejections: u64,
    pub coalesced: u64,
    pub deferred_discrete: u64,
}

struct Scheduled {
    command: Command,
    sequence: u64,
}

#[derive(Default)]
struct Queued {
    closed: bool,
    sequence: u64,
    ready: VecDeque<Scheduled>,
    discrete: VecDeque<Scheduled>,
    latest: VecDeque<(StateChannel, Scheduled)>,
    stats: QueueStats,
}

impl Queued {
    fn schedule(&mut self, command: Command) -> Scheduled {
        self.sequence += 1;
        Scheduled {
            command,
            sequence: self.sequence,
        }
    }

    fn take_latest(&mut self, channel: &StateChannel) -> Option<(StateChannel, Scheduled)> {
        let index = self.latest.iter().position(|(key, _)| key == channel)?;
        self.latest.remove(index)
    }

    fn push_discrete(&mut self, command: Command, capacity: usize, count_event: bool) {
        let work = self.schedule(command);
        if self.ready.len() < capacity {
            self.ready.push_back(work);
        } else {
            self.stats.deferred_discrete += u64::from(count_event);
            self.discrete.push_back(work);
        }
    }
}

struct Mailbox {
    capacity: usize,
    queued: Mutex<Queued>,
    work: Condvar,
}

impl Mailbox {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queued: Mutex::default(),
            work: Condvar::new(),
        }
    }

    fn send(&self, command: Command, delivery: Delivery) -> Result<()> {
        let mut queued = self.queued.lock().unwrap();
        if queued.closed {
            drop(queued);
            return Err(Error::Closed);
        }
        // A barrier seals only its own stream. The sample and event enter
        // the FIFO together, so a later sample cannot replace the sealed one.
        if let Delivery::Discrete(Some(channel)) = &delivery
            && let Some((_, sample)) = queued.take_latest(channel)
        {
            queued.push_discrete(sample.command, self.capacity, false);
        }
        let replaced = match delivery {
            Delivery::Immediate if queued.ready.len() == self.capacity => {
                queued.stats.busy_rejections += 1;
                drop(queued);
                return Err(Error::Busy);
            }
            Delivery::Discrete(_) => {
                queued.push_discrete(command, self.capacity, true);
                None
            }
            Delivery::Notify(ref channel)
                if queued.latest.iter().any(|(key, _)| key == channel) =>
            {
                // An idempotent refresh can use the pending evaluation. Keep
                // its acknowledgement for any caller already waiting for it.
                queued.stats.coalesced += 1;
                drop(queued);
                return Ok(());
            }
            Delivery::Latest(channel) | Delivery::Notify(channel) => {
                let replaced = queued.take_latest(&channel);
                queued.stats.coalesced += u64::from(replaced.is_some());
                let work = queued.schedule(command);
                queued.latest.push_back((channel, work));
                replaced
            }
            Delivery::Immediate => {
                let work = queued.schedule(command);
                queued.ready.push_back(work);
                None
            }
        };
        drop(queued);
        self.work.notify_one();
        if let Some((
            _,
            Scheduled {
                command: Command::Eval { reply, .. },
                ..
            },
        )) = replaced
        {
            complete(reply, Err(Error::Superseded));
        }
        Ok(())
    }

    fn receive(&self, deadline: Option<Instant>) -> std::result::Result<Option<Command>, Error> {
        let idle = |q: &mut Queued| !q.closed && q.ready.is_empty() && q.latest.is_empty();
        let queued = self.queued.lock().unwrap();
        let mut queued = match deadline {
            Some(at) => {
                self.work
                    .wait_timeout_while(queued, at.saturating_duration_since(Instant::now()), idle)
                    .unwrap()
                    .0
            }
            None => self.work.wait_while(queued, idle).unwrap(),
        };
        if queued.closed {
            return Err(Error::Closed);
        }
        let latest_first = queued.latest.front().is_some_and(|(_, latest)| {
            queued
                .ready
                .front()
                .is_none_or(|ready| latest.sequence < ready.sequence)
        });
        if latest_first {
            return Ok(queued.latest.pop_front().map(|(_, work)| work.command));
        }
        let ready = queued.ready.pop_front();
        if ready.is_some()
            && let Some(next) = queued.discrete.pop_front()
        {
            // Claim the slot for the oldest deferred discrete event before
            // producers can take it.
            queued.ready.push_back(next);
        }
        Ok(ready.map(|work| work.command))
    }

    fn close(&self) {
        self.queued.lock().unwrap().closed = true;
        self.work.notify_one();
    }

    fn take_closed(&self) -> impl Iterator<Item = Command> {
        let mut queued = self.queued.lock().unwrap();
        std::mem::take(&mut queued.ready)
            .into_iter()
            .chain(std::mem::take(&mut queued.discrete))
            .chain(
                std::mem::take(&mut queued.latest)
                    .into_iter()
                    .map(|(_, work)| work),
            )
            .map(|work| work.command)
    }
}

#[cfg(test)]
mod mailbox_tests {
    use super::*;

    fn eval_command(id: u64) -> (Command, Ticket<Evaluation>) {
        let (ticket, reply) = ticket(id);
        let command = Command::Eval {
            id,
            source: Source::Text("0".to_owned()),
            expected: None,
            svelte: None,
            reply,
        };
        (command, ticket)
    }

    #[test]
    fn ten_thousand_moves_keep_one_pending_value_per_channel() {
        let mailbox = Mailbox::new(1);
        mailbox.send(Command::Wake, Delivery::Immediate).unwrap();
        assert!(matches!(
            mailbox.send(Command::Wake, Delivery::Immediate),
            Err(Error::Busy)
        ));
        let channel = StateChannel::Pointer {
            panel: "test".to_owned(),
            pointer: 1,
        };
        let mut previous: Option<Ticket<Evaluation>> = None;
        for id in 0..10_000 {
            let (command, ticket) = eval_command(id);
            mailbox
                .send(command, Delivery::Latest(channel.clone()))
                .unwrap();
            if let Some(old) = previous.replace(ticket) {
                assert!(matches!(old.try_take(), Some(Err(Error::Superseded))));
            }
        }
        {
            let queued = mailbox.queued.lock().unwrap();
            assert_eq!(queued.ready.len(), 1);
            assert!(queued.discrete.is_empty());
            assert_eq!(queued.latest.len(), 1);
            assert_eq!(queued.stats.coalesced, 9_999);
            assert_eq!(queued.stats.busy_rejections, 1);
        }
        assert!(matches!(mailbox.receive(None), Ok(Some(Command::Wake))));
        assert!(matches!(
            mailbox.receive(None),
            Ok(Some(Command::Eval { id: 9_999, .. }))
        ));
    }

    #[test]
    fn a_refresh_notification_preserves_the_waiting_publication_ticket() {
        let mailbox = Mailbox::new(1);
        mailbox.send(Command::Wake, Delivery::Immediate).unwrap();
        let (publication, ticket) = eval_command(1);
        mailbox
            .send(publication, Delivery::Latest(StateChannel::Publication))
            .unwrap();
        mailbox
            .send(
                eval_command(2).0,
                Delivery::Notify(StateChannel::Publication),
            )
            .unwrap();
        assert!(ticket.try_take().is_none());
        assert!(matches!(mailbox.receive(None), Ok(Some(Command::Wake))));
        assert!(matches!(
            mailbox.receive(None),
            Ok(Some(Command::Eval { id: 1, .. }))
        ));
    }

    #[test]
    fn a_barrier_seals_only_the_sample_on_its_own_channel() {
        let mailbox = Mailbox::new(1);
        let pointer = StateChannel::Pointer {
            panel: "test".to_owned(),
            pointer: 1,
        };
        let other = StateChannel::Publication;
        mailbox.send(Command::Wake, Delivery::Immediate).unwrap();
        for (id, delivery) in [
            (1, Delivery::Latest(pointer.clone())),
            (2, Delivery::Latest(other)),
            (3, Delivery::Discrete(Some(pointer.clone()))),
            (4, Delivery::Latest(pointer)),
        ] {
            mailbox.send(eval_command(id).0, delivery).unwrap();
        }
        {
            let queued = mailbox.queued.lock().unwrap();
            assert_eq!(queued.discrete.len(), 2);
            assert_eq!(queued.latest.len(), 2);
            assert_eq!(queued.stats.deferred_discrete, 1);
        }
        assert!(matches!(mailbox.receive(None), Ok(Some(Command::Wake))));
        for want in [2, 1, 3, 4] {
            assert!(
                matches!(mailbox.receive(None), Ok(Some(Command::Eval { id, .. })) if id == want)
            );
        }
    }
}

impl Realm {
    /// A terminal heap or external memory failure of a shipped panel.
    pub fn take_panel_memory_failure(&self) -> Option<Error> {
        self.inner
            .control
            .panel_memory
            .as_ref()?
            .error
            .lock()
            .unwrap()
            .take()
    }

    /// Waits up to `within` for the isolate's turn in the heap account:
    /// `None` while it still waits (others hold the room), else whether it
    /// was admitted. A realm starts at once and queues its tasks; a caller
    /// that times its first task waits for this first, so the time spent in
    /// line is not counted against the task.
    pub fn admitted(&self, within: Duration) -> Option<Result<()>> {
        let control = &self.inner.control;
        let decided = control.admitted.lock().unwrap();
        let (decided, _) = control
            .decided
            .wait_timeout_while(decided, within, |d| d.is_none())
            .unwrap();
        decided.clone()
    }
    /// Nonblocking; GC and V8 API access stay on the isolate owner.
    pub fn memory_pressure(&self) -> Result<()> {
        self.send(Command::MemoryPressure)
    }
    fn send(&self, command: Command) -> Result<()> {
        self.inner.mailbox.send(command, Delivery::Immediate)
    }

    pub fn queue_stats(&self) -> QueueStats {
        self.inner.mailbox.queued.lock().unwrap().stats
    }

    /// Retain a discrete evaluation until it can run, without waiting on the caller.
    pub fn eval_discrete(&self, source: impl Into<String>) -> Result<Ticket<Evaluation>> {
        let source = source.into();
        self.check_source(&source)?;
        self.queue_eval(Delivery::Discrete(None), Source::Text(source))
    }

    /// Seal a pending observation on this channel before applying the commit.
    pub fn eval_barrier(
        &self,
        channel: StateChannel,
        source: impl Into<String>,
    ) -> Result<Ticket<Evaluation>> {
        let source = source.into();
        self.check_source(&source)?;
        self.queue_eval(Delivery::Discrete(Some(channel)), Source::Text(source))
    }

    /// Replace any pending observation on this channel. The old ticket finishes
    /// as Superseded. The owner wakes directly on work or when it frees a slot.
    pub fn eval_latest(
        &self,
        channel: StateChannel,
        source: impl Into<String>,
    ) -> Result<Ticket<Evaluation>> {
        let source = source.into();
        self.check_source(&source)?;
        self.queue_eval(Delivery::Latest(channel), Source::Text(source))
    }

    /// Request the same idempotent evaluation already pending on a channel.
    /// The script reads the latest state when it runs.
    pub fn notify_latest(&self, channel: StateChannel, source: impl Into<String>) -> Result<()> {
        let source = source.into();
        self.check_source(&source)?;
        drop(self.queue_eval(Delivery::Notify(channel), Source::Text(source))?);
        Ok(())
    }

    /// Plan native input in dispatch order, on the owner just before evaluation.
    /// A planner must not wait on this realm or run JavaScript.
    pub fn eval_input(
        &self,
        channel: StateChannel,
        sample: bool,
        prepare: impl FnOnce() -> Result<String> + Send + 'static,
    ) -> Result<Ticket<Evaluation>> {
        let delivery = if sample {
            Delivery::Latest(channel)
        } else {
            Delivery::Discrete(Some(channel))
        };
        self.queue_eval(delivery, Source::Prepare(Box::new(prepare)))
    }

    fn queue_eval(&self, delivery: Delivery, source: Source) -> Result<Ticket<Evaluation>> {
        let id = self.inner.control.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.inner.mailbox.send(
            Command::Eval {
                id,
                source,
                expected: None,
                svelte: None,
                reply,
            },
            delivery,
        )?;
        Ok(ticket)
    }

    fn check_source(&self, source: &str) -> Result<()> {
        if source.len() > self.inner.max_source {
            Err(Error::Quota("source bytes"))
        } else {
            Ok(())
        }
    }
    pub fn eval(&self, source: impl Into<String>) -> Result<Ticket<Evaluation>> {
        self.eval_generation(source, None)
    }
    pub fn eval_generation(
        &self,
        source: impl Into<String>,
        expected: Option<Generation>,
    ) -> Result<Ticket<Evaluation>> {
        let source = source.into();
        self.check_source(&source)?;
        let id = self.inner.control.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Eval {
            id,
            source: Source::Text(source),
            expected,
            svelte: None,
            reply,
        })?;
        Ok(ticket)
    }
    pub(crate) fn eval_svelte(
        &self,
        source: String,
        filename: String,
        unit: SvelteUnit,
    ) -> Result<Ticket<Evaluation>> {
        self.check_source(&source)?;
        let id = self.inner.control.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Eval {
            id,
            source: Source::Text(source),
            expected: None,
            svelte: Some((filename, unit)),
            reply,
        })?;
        Ok(ticket)
    }
    /// Source is a script expression, optionally yielding a Promise. The caller's
    /// compiler/bundler resolves ESM imports before submission. Only v4 may reload.
    pub fn reload(&self, source: impl Into<String>) -> Result<Ticket<Evaluation>> {
        let source = source.into();
        self.check_source(&source)?;
        let id = self.inner.control.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Reload { id, source, reply })?;
        Ok(ticket)
    }
    /// Sampling runs on the owner and includes every pinned retired generation.
    pub fn measure(&self) -> Result<Ticket<Vec<GenerationMemory>>> {
        let (ticket, reply) = ticket(0);
        self.send(Command::Measure { reply })?;
        Ok(ticket)
    }
    pub fn pin(&self) -> Result<Ticket<GenerationPin>> {
        let (ticket, reply) = ticket(0);
        self.send(Command::Pin { reply })?;
        Ok(ticket)
    }
    pub fn invoke_retained(
        &self,
        pin: &GenerationPin,
        source: impl Into<String>,
    ) -> Result<Ticket<Evaluation>> {
        let source = source.into();
        self.check_source(&source)?;
        let id = self.inner.control.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.send(Command::Retained {
            id,
            pin: pin.clone(),
            source,
            reply,
        })?;
        Ok(ticket)
    }
    /// Inspect a hung or active realm without entering JavaScript.
    pub fn active_execution(&self) -> Option<ActiveExecution> {
        self.inner
            .control
            .active
            .lock()
            .unwrap()
            .as_ref()
            .map(|e| ActiveExecution {
                execution: e.id,
                generation: e.generation.clone(),
                elapsed: e.started.elapsed(),
            })
    }
    /// Fenced by execution id; an old Stop cannot terminate a later task.
    /// The extension (`pmext://<id>/…` module) the watchdog found running
    /// `execution` when it prompted, if any.
    pub fn culprit(&self, execution: u64) -> Option<String> {
        self.inner.control.culprit.get(execution)
    }
    /// Where `execution` was when the watchdog looked: its innermost frames
    /// (`function (script:line) < …`).
    pub fn culprit_place(&self, execution: u64) -> Option<String> {
        self.inner.control.culprit.place(execution)
    }
    pub fn stop(&self, execution: u64) -> bool {
        let active = self.inner.control.active.lock().unwrap();
        if let Some(active) = active.as_ref().filter(|a| a.id == execution) {
            active.cancellation.cancel();
            active.handle.terminate_execution();
            true
        } else {
            false
        }
    }
    /// Nonblocking shutdown; disposal and all revocations happen on the owner.
    pub fn close(&self) {
        self.inner.control.closing.store(true, Ordering::Release);
        self.inner.mailbox.close();
        self.inner.control.wake.notify_all();
        if let Some(e) = &*self.inner.control.active.lock().unwrap() {
            e.cancellation.cancel();
            e.handle.terminate_execution();
        }
    }
    pub fn is_disposed(&self) -> bool {
        self.inner.control.disposed.load(Ordering::Acquire)
    }
}

/// A pool is scoped to one project window. Its compat key is unique in that scope.
/// Callers must resolve synchronous dependencies before routing to this pool.
pub struct RuntimePool {
    scope: String,
    realms: Mutex<HashMap<RealmKind, Weak<Inner>>>,
    config: Config,
}
#[derive(Clone)]
pub struct Config {
    pub limits: Limits,
    pub watchdog: WatchdogPolicy,
    pub kernel: Arc<dyn KernelHooks>,
    pub host: Arc<dyn HostProxy>,
    pub watchdog_hooks: Arc<dyn WatchdogHooks>,
    pub cache: CodeCache,
    pub engine: Option<Arc<dyn EngineProxy>>,
    /// Shared admission account must include all pools and retained generations.
    pub admission: Admission,
}
/// Heap admission: what every isolate's current heap limit adds up to. An
/// isolate reserves its initial slice at start, takes more from V8's
/// near-heap-limit callback only while the account has room, gives back what
/// it grew past its live size when it idles, and returns the rest when it is
/// disposed on its owner, so `used` is exact at every step.
#[derive(Clone)]
pub struct Admission {
    used: Arc<AtomicUsize>,
    /// The most `used` has been, and how many isolates hold a reservation.
    peak: Arc<AtomicUsize>,
    isolates: Arc<AtomicUsize>,
    budget: usize,
    /// Isolates waiting for room, served first come first served, and the
    /// signal that room was given back.
    queue: Arc<(Mutex<Queue>, Condvar)>,
    /// How long a waiting isolate goes on while the account does not change
    /// at all (nothing given back, nothing taken): then it is stuck, not busy.
    patience: Duration,
}
#[derive(Default)]
struct Queue {
    /// The next ticket handed out and the one served now.
    next: u64,
    serving: u64,
    /// Tickets whose isolate stopped waiting before its turn.
    left: std::collections::BTreeSet<u64>,
    /// Bumped whenever the account changes, so a waiter can tell busy from stuck.
    changes: u64,
}
/// In-process isolates share one 4 GB pointer-compression cage (plan §3.5.1);
/// a quarter stays for V8's code and read-only spaces, the grace a failing
/// isolate unwinds in, and allocator overheads.
pub const CAGE_ADMISSION: usize = 3 * 1024 * 1024 * 1024;
/// How long an isolate waits for room while the account stays exactly as it
/// is (no isolate starts, grows, trims or ends): two minutes, as an export
/// waits for the extensions of other open projects.
pub const ADMISSION_PATIENCE: Duration = Duration::from_secs(120);
impl Admission {
    pub fn new(budget: usize) -> Self {
        Self::with_patience(budget, ADMISSION_PATIENCE)
    }
    pub fn with_patience(budget: usize, patience: Duration) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            peak: Arc::new(AtomicUsize::new(0)),
            isolates: Arc::new(AtomicUsize::new(0)),
            budget,
            queue: Arc::new((Mutex::new(Queue::default()), Condvar::new())),
            patience,
        }
    }
    /// Room held outside any isolate, given back when the holder is dropped
    /// (a test fills the account with it).
    #[doc(hidden)]
    pub fn hold(&self, bytes: usize) -> Result<impl Drop + Send + use<>> {
        self.reserve(bytes)
    }
    /// How many isolates wait for room now.
    pub fn waiting(&self) -> usize {
        let queue = self.queue.0.lock().unwrap();
        (queue.next - queue.serving) as usize - queue.left.len()
    }
    /// Room was given back (or the account changed): waiters look again.
    fn changed(&self) {
        let (queue, room) = &*self.queue;
        queue.lock().unwrap().changes += 1;
        room.notify_all();
    }
    /// The process's one account: every window's built-ins, v4 extensions,
    /// compat realm and compiler reserve from it.
    pub fn cage() -> Self {
        static CAGE: std::sync::OnceLock<Admission> = std::sync::OnceLock::new();
        CAGE.get_or_init(|| Admission::new(CAGE_ADMISSION)).clone()
    }
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
    pub fn budget(&self) -> usize {
        self.budget
    }
    pub fn peak(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }
    pub fn isolates(&self) -> usize {
        self.isolates.load(Ordering::Acquire)
    }
    fn take(&self, bytes: usize) -> bool {
        let taken = self
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|n| *n <= self.budget)
            });
        if let Ok(before) = taken {
            self.peak.fetch_max(before + bytes, Ordering::AcqRel);
        }
        taken.is_ok()
    }
    /// Room now, never ahead of an isolate already waiting for it.
    pub(crate) fn reserve(&self, bytes: usize) -> Result<Reservation> {
        let queue = self.queue.0.lock().unwrap();
        let waiters = queue.next - queue.serving > queue.left.len() as u64;
        if waiters || !self.take(bytes) {
            return Err(Error::Quota("isolate admission"));
        }
        drop(queue);
        Ok(self.reserved(bytes))
    }
    /// Room for an isolate someone asked for: when the account is full it
    /// waits its turn (first come, first served) for isolates to give heap
    /// back, while `wanted` (its realm is still open). It gives up only when
    /// it is no longer wanted, when it could never fit, or when the account
    /// stayed exactly as it is for the account's patience (stuck, not busy).
    pub(crate) fn reserve_waiting(&self, bytes: usize, wanted: impl Fn() -> bool) -> Result<Reservation> {
        if bytes > self.budget {
            return Err(Error::Quota("isolate admission"));
        }
        let (queue, room) = &*self.queue;
        let mut q = queue.lock().unwrap();
        let ticket = q.next;
        q.next += 1;
        let mut seen = q.changes;
        let mut since = Instant::now();
        let result = loop {
            loop {
                let serving = q.serving;
                if !q.left.remove(&serving) {
                    break;
                }
                q.serving += 1;
            }
            if q.serving == ticket && self.take(bytes) {
                break Ok(());
            }
            if !wanted() || since.elapsed() >= self.patience {
                break Err(Error::Quota("isolate admission"));
            }
            q = room.wait_timeout(q, Duration::from_millis(100)).unwrap().0;
            if q.changes != seen {
                seen = q.changes;
                since = Instant::now();
            }
        };
        if q.serving == ticket {
            q.serving += 1;
        } else {
            q.left.insert(ticket);
        }
        drop(q);
        // The next in line may fit now.
        room.notify_all();
        result.map(|()| self.reserved(bytes))
    }
    fn reserved(&self, bytes: usize) -> Reservation {
        self.isolates.fetch_add(1, Ordering::AcqRel);
        self.changed();
        Reservation {
            account: self.clone(),
            bytes: AtomicUsize::new(bytes),
        }
    }
}
/// One isolate's share of its account. Only its owner thread changes it.
pub(crate) struct Reservation {
    account: Admission,
    bytes: AtomicUsize,
}
impl Reservation {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes.load(Ordering::Acquire)
    }
    /// More heap, only while the account has room.
    pub(crate) fn grow(&self, bytes: usize) -> bool {
        let taken = self.account.take(bytes);
        if taken {
            self.bytes.fetch_add(bytes, Ordering::AcqRel);
            self.account.changed();
        }
        taken
    }
    /// The grace a terminating isolate unwinds in: taken even past the
    /// budget (the cage keeps room for it), and given back at disposal.
    pub(crate) fn grace(&self, bytes: usize) {
        let before = self.account.used.fetch_add(bytes, Ordering::AcqRel);
        self.account
            .peak
            .fetch_max(before + bytes, Ordering::AcqRel);
        self.bytes.fetch_add(bytes, Ordering::AcqRel);
    }
    pub(crate) fn shrink(&self, bytes: usize) {
        let bytes = bytes.min(self.bytes());
        if bytes == 0 {
            // Nothing given back: waiters must not take it for a change, or
            // trims that free nothing keep a stuck account from ever saying so.
            return;
        }
        self.bytes.fetch_sub(bytes, Ordering::AcqRel);
        self.account.used.fetch_sub(bytes, Ordering::AcqRel);
        self.account.changed();
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.account.isolates.fetch_sub(1, Ordering::AcqRel);
        self.account
            .used
            .fetch_sub(*self.bytes.get_mut(), Ordering::AcqRel);
        self.account.changed();
    }
}
impl RuntimePool {
    pub fn new(scope: impl Into<String>, config: Config) -> Result<Self> {
        if config.limits.heap_bytes < 16 * 1024 * 1024
            || config.limits.initial_heap_bytes > config.limits.heap_bytes
            || config.limits.queue_capacity == 0
            || config.limits.max_host_ops == 0
            || config.watchdog.prompt < config.watchdog.notice
            || config.watchdog.repeat.is_zero()
        {
            return Err(Error::Invalid("Invalid runtime limits"));
        }
        Ok(Self {
            scope: scope.into(),
            realms: Default::default(),
            config,
        })
    }
    /// Starting is asynchronous: the first task observes startup/load failures.
    pub fn realm(&self, kind: RealmKind) -> Result<Realm> {
        if matches!(kind, RealmKind::Store(_)) {
            return Err(Error::Invalid(
                "Store realms require StorePool helper confinement",
            ));
        }
        self.start_realm(kind)
    }
    /// Shipped panel code gets a larger ceiling on the same shared account,
    /// while the host remounts the panel after a terminal memory failure.
    pub fn builtin_panel_realm(
        &self,
        panel: &str,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Realm> {
        let mut limits = self.config.limits.clone();
        limits.heap_bytes = crate::host::builtin_heap_bytes(panel);
        self.start_realm_with(RealmKind::V4Extension(panel.into()), limits, Some(wake))
    }
    pub(crate) fn start_realm(&self, kind: RealmKind) -> Result<Realm> {
        self.start_realm_with(kind, self.config.limits.clone(), None)
    }
    pub(crate) fn start_realm_with(
        &self,
        kind: RealmKind,
        limits: Limits,
        panel_wake: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<Realm> {
        let mut realms = self.realms.lock().unwrap();
        if let Some(inner) = realms
            .get(&kind)
            .and_then(Weak::upgrade)
            .filter(|i| !i.control.closing.load(Ordering::Acquire))
        {
            return Ok(Realm { inner });
        }
        let mailbox = Arc::new(Mailbox::new(self.config.limits.queue_capacity));
        let control = Arc::new(Control {
            active: Mutex::new(None),
            closing: AtomicBool::new(false),
            disposed: AtomicBool::new(false),
            wake: Condvar::new(),
            ids: AtomicU64::new(1),
            culprit: crate::vm::Culprit::default(),
            admitted: Mutex::new(None),
            decided: Condvar::new(),
            panel_memory: panel_wake.map(|wake| PanelMemory {
                error: Mutex::new(None),
                wake,
            }),
        });
        let inner = Arc::new(Inner {
            mailbox: mailbox.clone(),
            control: control.clone(),
            max_source: self.config.limits.max_source_bytes,
        });
        let name = format!("{}:{kind:?}", self.scope);
        let mut config = self.config.clone();
        config.limits = limits;
        let worker_kind = kind.clone();
        thread::Builder::new()
            .name("pm-v8-owner".into())
            .spawn(move || worker(mailbox, control, name, worker_kind, config))
            .map_err(|e| Error::Host(e.to_string()))?;
        realms.insert(kind.clone(), Arc::downgrade(&inner));
        Ok(Realm { inner })
    }
    /// Whole editor closure switches to Compat when any provider has a v1–3 fork.
    pub fn builtin(&self, id: &str, compat_editor: bool) -> Result<Realm> {
        let kind = builtin_cluster(id);
        self.realm(if compat_editor && kind == RealmKind::BuiltinEditor {
            RealmKind::Compat
        } else {
            kind
        })
    }
}

/// A task that has waited this many times the prompt's limit (wall time,
/// less engine calls) is asked about even if it used little CPU: stuck in a
/// wait rather than starved.
const BACKSTOP: u32 = 4;

// One sleeping watchdog per owner, never a new thread per task or timer tick.
struct WatchdogThread {
    control: Arc<Control>,
    join: Option<thread::JoinHandle<()>>,
}
impl WatchdogThread {
    fn new(control: &Arc<Control>, config: &Config) -> Self {
        let control = control.clone();
        let watched = control.clone();
        let hooks = config.watchdog_hooks.clone();
        let repeat = config.watchdog.repeat;
        let notice_after = config.watchdog.notice;
        let backstop = config.watchdog.prompt * BACKSTOP;
        let join = thread::spawn(move || {
            loop {
                let mut active = watched.active.lock().unwrap();
                while active.is_none() && !watched.closing.load(Ordering::Acquire) {
                    active = watched.wake.wait(active).unwrap();
                }
                if watched.closing.load(Ordering::Acquire) {
                    break;
                }
                let (mut active, _) = watched
                    .wake
                    .wait_timeout(active, Duration::from_millis(5))
                    .unwrap();
                let Some(execution) = active.as_mut() else {
                    continue;
                };
                if execution.cancellation.is_cancelled() {
                    execution.handle.terminate_execution();
                }
                execution
                    .handle
                    .request_interrupt(vm::check_external, std::ptr::null_mut());
                // JS time only: waiting on the engine or kernel, or for a
                // CPU on a busy machine, is not a hang.
                let elapsed = execution.ran(backstop);
                let notice = !execution.noticed && elapsed >= notice_after;
                if notice {
                    execution.noticed = true;
                }
                let prompt = elapsed >= execution.next_prompt;
                if prompt {
                    execution.next_prompt = elapsed + repeat;
                }
                let (generation, id, epoch) =
                    (execution.generation.clone(), execution.id, execution.epoch);
                drop(active);
                if notice {
                    hooks.notice(&generation, id);
                }
                if prompt {
                    // Who is running: the innermost extension module on the
                    // stack, read inside the running JS (any hang: a raster
                    // override, an event, a timer, a command).
                    let handle = watched
                        .active
                        .lock()
                        .unwrap()
                        .as_ref()
                        .filter(|e| e.epoch == epoch)
                        .map(|e| e.handle.clone());
                    if let Some(handle) = handle {
                        crate::vm::capture_culprit(&handle, &watched.culprit, id);
                    }
                }
                if prompt && hooks.prompt(&generation, id) == WatchdogDecision::Stop {
                    let active = watched.active.lock().unwrap();
                    if let Some(execution) = active.as_ref().filter(|e| e.epoch == epoch) {
                        execution.cancellation.cancel();
                        execution.handle.terminate_execution();
                    }
                }
            }
        });
        Self {
            control,
            join: Some(join),
        }
    }
}
impl Drop for WatchdogThread {
    fn drop(&mut self) {
        self.control.closing.store(true, Ordering::Release);
        self.control.wake.notify_all();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
struct Guard {
    control: Arc<Control>,
    epoch: u64,
}
impl Guard {
    fn new(
        vm: &Vm,
        generation: &Generation,
        id: u64,
        config: &Config,
        control: &Arc<Control>,
    ) -> Self {
        static EPOCH: AtomicU64 = AtomicU64::new(1);
        let epoch = EPOCH.fetch_add(1, Ordering::Relaxed);
        let handle = vm.handle();
        let cancellation = Cancellation::default();
        vm.begin_task(cancellation.clone());
        *control.active.lock().unwrap() = Some(Execution {
            id,
            epoch,
            generation: generation.clone(),
            handle: handle.clone(),
            cancellation: cancellation.clone(),
            started: Instant::now(),
            blocked: vm.blocked(),
            clock: ThreadClock::current().and_then(|clock| Some((clock, clock.cpu()?))),
            noticed: false,
            next_prompt: config.watchdog.prompt,
        });
        control.wake.notify_all();
        if control.closing.load(Ordering::Acquire) {
            cancellation.cancel();
            handle.terminate_execution();
        }
        Self {
            control: control.clone(),
            epoch,
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let mut active = self.control.active.lock().unwrap();
        if active.as_ref().is_some_and(|a| a.epoch == self.epoch) {
            *active = None;
        }
        self.control.wake.notify_all();
    }
}
struct Request {
    id: u64,
    job: Box<dyn HostRequest>,
    cancel: Cancellation,
    deadline: Instant,
    /// External bytes it holds (response room and body), released on retire.
    reserved: usize,
}
struct Live {
    vm: Vm,
    generation: Generation,
    storage: Value,
    hot: Value,
    pins: Arc<()>,
    requests: Vec<Request>,
    next_wake: Option<Instant>,
}
impl Live {
    fn new(
        generation: Generation,
        kind: &RealmKind,
        config: &Config,
        storage: Value,
        hot: Value,
        reservation: Reservation,
    ) -> Result<Self> {
        let vm = Vm::new(
            kind.snapshot_kind(),
            config,
            &storage,
            &hot,
            generation.clone(),
            reservation,
        )?;
        Ok(Self {
            vm,
            generation,
            storage,
            hot,
            pins: Arc::new(()),
            requests: vec![],
            next_wake: None,
        })
    }
    fn cancel_io(&mut self) {
        for request in &mut self.requests {
            request.cancel.cancel();
            request.job.cancel();
        }
        self.requests.clear();
        let mut n = self.vm.native.lock().unwrap();
        n.queued.clear();
        n.deferred.clear();
        n.aborted.clear();
        n.in_flight = 0;
        n.reserved = 0;
    }
    /// AbortSignal rejects in JS immediately. Retire the corresponding native
    /// job before acknowledging that task, even if no future idle tick occurs.
    fn cancel_aborted(&mut self) {
        let (aborted, mut retired) = {
            let mut n = self.vm.native.lock().unwrap();
            let aborted = std::mem::take(&mut n.aborted);
            let (mut count, mut released) = (0, 0);
            n.queued.retain(|(id, r)| {
                let keep = !aborted.contains(id);
                if !keep {
                    count += 1;
                    released += crate::vm::held(r, true);
                }
                keep
            });
            // A waiting fetch holds its body only.
            n.deferred.retain(|(id, r)| {
                let keep = !aborted.contains(id);
                if !keep {
                    count += 1;
                    released += crate::vm::held(r, false);
                }
                keep
            });
            n.in_flight -= count;
            n.reserved -= released;
            (aborted, 0)
        };
        let mut released = 0;
        self.requests.retain_mut(|request| {
            if aborted.contains(&request.id) {
                request.cancel.cancel();
                request.job.cancel();
                retired += 1;
                released += request.reserved;
                false
            } else {
                true
            }
        });
        let mut n = self.vm.native.lock().unwrap();
        n.in_flight -= retired;
        n.reserved -= released;
    }
    fn pump(&mut self, config: &Config) -> Result<()> {
        let (queued, aborted, logs) = {
            let mut n = self.vm.native.lock().unwrap();
            // A waiting fetch that was aborted held its body only.
            let aborted = n.aborted.clone();
            let (mut dropped, mut released) = (0, 0);
            n.deferred.retain(|(id, r)| {
                let keep = !aborted.contains(id);
                if !keep {
                    dropped += 1;
                    released += crate::vm::held(r, false);
                }
                keep
            });
            n.in_flight = n.in_flight.saturating_sub(dropped);
            n.reserved = n.reserved.saturating_sub(released);
            n.admit_deferred();
            (
                std::mem::take(&mut n.queued),
                std::mem::take(&mut n.aborted),
                std::mem::take(&mut n.logs),
            )
        };
        for (level, message) in logs {
            config.host.console(&self.generation, &level, &message);
        }
        let mut completed = Vec::new();
        let (mut retired, mut released) = (0, 0);
        for (id, request) in queued {
            let reserved = crate::vm::held(&request, true);
            if aborted.contains(&id) {
                retired += 1;
                released += reserved;
                continue;
            }
            let cancellation = Cancellation::default();
            let deadline = request.deadline;
            match config
                .host
                .fetch(&self.generation, request, cancellation.clone())
            {
                Ok(job) => self.requests.push(Request {
                    id,
                    job,
                    cancel: cancellation,
                    deadline,
                    reserved,
                }),
                Err(error) => {
                    completed.push(json!([id,{"error":error.to_string()}]));
                    retired += 1;
                    released += reserved;
                }
            }
        }
        let mut bodies = Vec::new();
        self.requests.retain_mut(|request|{
            let result=if aborted.contains(&request.id)||Instant::now()>=request.deadline {request.cancel.cancel();request.job.cancel();Some(Err(Error::Host("Aborted or timed out".into())))}else{request.job.poll()};
            if let Some(result)=result {
                retired+=1;
                released+=request.reserved;
                let data=match result {
                    Ok(response) if response.body.len()<=config.limits.max_response_bytes && response.headers.iter().map(|(a,b)|a.len()+b.len()).sum::<usize>()<=config.limits.max_op_bytes =>{
                        // The body goes over as bytes (`__body`), not JSON.
                        bodies.push((request.id,response.body));
                        json!({"status":response.status,"url":response.url,"headers":response.headers,"body":request.id})
                    },
                    Ok(_)=>{request.cancel.cancel();request.job.cancel();json!({"error":"Host response quota"})},
                    Err(error)=>json!({"error":error.to_string()})
                };completed.push(json!([request.id,data]));false
            }else{true}
        });
        {
            let mut n = self.vm.native.lock().unwrap();
            n.in_flight = n.in_flight.saturating_sub(retired);
            n.reserved = n.reserved.saturating_sub(released);
            // Room that just came free goes to waiting fetches; the next pump starts them.
            n.admit_deferred();
            n.bodies.extend(bodies);
        }
        let pumped = self.vm.pump(json!(completed));
        // A body JS didn't take (its request was gone) is dropped here.
        self.vm.native.lock().unwrap().bodies.clear();
        pumped
    }
    fn run(
        &mut self,
        source: &str,
        _id: u64,
        config: &Config,
        control: &Arc<Control>,
    ) -> Result<String> {
        let started = self.vm.start(source);
        self.cancel_aborted();
        let value = started.map_err(|error| {
            if self.vm.task_cancelled() {
                Error::Terminated
            } else {
                error
            }
        })?;
        loop {
            if self.vm.task_cancelled() {
                self.vm.handle().terminate_execution();
                return Err(Error::Terminated);
            }
            if control.closing.load(Ordering::Acquire) {
                self.vm.handle().terminate_execution();
                return Err(Error::Closed);
            }
            let settled = self.vm.settle(&value);
            self.cancel_aborted();
            if let Some(value) = settled? {
                return Ok(value);
            }
            self.pump(config)?;
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn commit(
        &mut self,
        previous: Option<&Generation>,
        config: &Config,
        retained: bool,
    ) -> Result<()> {
        let effects = self.vm.effects()?;
        let ops = effects["ops"]
            .as_array()
            .ok_or(Error::Invalid("Invalid staged operations"))?;
        if ops.len() > config.limits.max_ops
            || serde_json::to_vec(ops)
                .map_err(|e| Error::JavaScript(e.to_string()))?
                .len()
                > config.limits.max_op_bytes
        {
            return Err(Error::Quota("staged operation bytes/count"));
        }
        // Finish every fallible JS step before the kernel's atomic publication.
        self.update_schedule()?;
        if retained {
            config.kernel.commit_retained(&self.generation, ops)?;
        } else {
            config.kernel.commit_activation(
                previous,
                &self.generation,
                ops,
                &effects["storage"],
            )?;
            self.storage = effects["storage"].clone();
            self.hot = effects["hot"].clone();
        }
        Ok(())
    }
    fn update_schedule(&mut self) -> Result<()> {
        let delay = self.vm.text("__pm.nextDelay(__now())")?;
        let delay = delay
            .parse::<f64>()
            .ok()
            .map(|n| Duration::from_secs_f64(n.max(0.) / 1000.));
        let native = self.vm.native.lock().unwrap();
        let io_delay = if !native.queued.is_empty() || !native.logs.is_empty() {
            Some(Duration::ZERO)
        } else if !self.requests.is_empty() || !native.deferred.is_empty() {
            Some(Duration::from_millis(5))
        } else {
            None
        };
        self.next_wake = delay
            .into_iter()
            .chain(io_delay)
            .min()
            .map(|d| Instant::now() + d);
        Ok(())
    }
    fn restore_storage(&mut self) -> Result<()> {
        self.vm.restore(&self.storage, &self.hot)?;
        self.update_schedule()
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        self.cancel_io();
    }
}
fn fail_generation(live: &mut Live, config: &Config) {
    live.cancel_io();
    config.kernel.revoke_generation(&live.generation);
    config.kernel.seal_open_gestures(&live.generation);
}
// Losing the worker closes admission even if it unwinds before disposal.
struct CloseMailbox<'a>(&'a Mailbox);

impl Drop for CloseMailbox<'_> {
    fn drop(&mut self) {
        let mailbox = self.0;
        mailbox.close();
        for command in mailbox.take_closed() {
            match command {
                Command::MemoryPressure => continue,
                Command::Wake => continue,
                Command::Measure { reply } => complete(reply, Err(Error::Closed)),
                Command::Pin { reply } => complete(reply, Err(Error::Closed)),
                Command::Eval { reply, .. }
                | Command::Reload { reply, .. }
                | Command::Retained { reply, .. } => complete(reply, Err(Error::Closed)),
            }
        }
    }
}

fn worker(
    mailbox: Arc<Mailbox>,
    control: Arc<Control>,
    name: String,
    kind: RealmKind,
    config: Config,
) {
    let receiver = CloseMailbox(&mailbox);
    let notifier = Arc::downgrade(&mailbox);
    let _watchdog = WatchdogThread::new(&control, &config);
    let pressure_sender = notifier.clone();
    let memory = pm_memory::Registration::pressure_only(move |_, level| {
        if level != pm_memory::Pressure::Normal
            && let Some(tx) = pressure_sender.upgrade()
        {
            let _ = tx.send(Command::MemoryPressure, Delivery::Immediate);
        }
    });
    let generation = Generation {
        realm: name.clone(),
        number: next_generation(),
    };
    // Its heap, on its own thread: a full account makes this isolate wait its
    // turn (the tasks sent meanwhile queue behind it) instead of refusing
    // work someone asked for; closing the realm stops the wait.
    let admitted = config
        .admission
        .reserve_waiting(HeapPlan::new(&config.limits).initial(), || {
            !control.closing.load(Ordering::Acquire)
        });
    *control.admitted.lock().unwrap() = Some(admitted.as_ref().map(|_| ()).map_err(Clone::clone));
    control.decided.notify_all();
    let initial = admitted
        .and_then(|reservation| {
            config.kernel.load_storage(&name).and_then(|storage| {
                Live::new(generation, &kind, &config, storage, json!({}), reservation)
            })
        });
    let mut current = initial;
    let mut retained: HashMap<u64, Live> = HashMap::new();
    // Set after work that grew a heap: an isolate idle this long gives back
    // what it grew past its live size.
    let mut trim_at = Some(Instant::now() + IDLE_TRIM);
    while !control.closing.load(Ordering::Acquire) {
        retained.retain(|_, live| Arc::strong_count(&live.pins) > 1);
        let held = current
            .as_ref()
            .ok()
            .map(|live| live.vm.memory())
            .into_iter()
            .chain(retained.values().map(|live| live.vm.memory()))
            .map(|m| m.physical_heap_bytes as u64 + m.external_bytes as u64)
            .sum();
        memory.report(pm_memory::other(held));
        let wake = current.as_ref().ok().and_then(|live| live.next_wake);
        let deadline = match (wake, trim_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let command = match mailbox.receive(deadline) {
            Ok(Some(command)) => {
                pm_perf::stall::stall(pm_perf::stall::Stage::V8);
                trim_at = Some(Instant::now() + IDLE_TRIM);
                command
            }
            Err(_) => break,
            Ok(None) => {
                if trim_at.is_some_and(|at| at <= Instant::now()) {
                    trim_at = None;
                    if let Ok(live) = &mut current {
                        live.vm.trim();
                    }
                    for live in retained.values_mut() {
                        live.vm.trim();
                    }
                    // Native host replies are allocated on this owner too.
                    // UI/render collection cannot drain its remote frees.
                    pm_memory::pressure_relief();
                }
                if wake.is_none_or(|wake| wake > Instant::now()) {
                    continue;
                }
                if let Ok(live) = &mut current {
                    // Timers and pending host operations run without a submitted task.
                    let guard = Guard::new(
                        &live.vm,
                        &live.generation,
                        control.ids.fetch_add(1, Ordering::Relaxed),
                        &config,
                        &control,
                    );
                    let result = live
                        .pump(&config)
                        .and_then(|_| live.commit(Some(&live.generation.clone()), &config, false));
                    drop(guard);
                    if let Err(error) = result {
                        if control.report_panel_memory(&error, Some(live), &name, &config.limits) {
                            break;
                        }
                        fail_generation(live, &config);
                        drop(std::mem::replace(&mut current, Err(Error::Closed)));
                        current = recover(&name, &kind, &config);
                    }
                }
                continue;
            }
        };
        match command {
            Command::MemoryPressure => {
                if let Ok(live) = &mut current {
                    live.vm.trim();
                }
                for live in retained.values_mut() {
                    live.vm.trim();
                }
                pm_memory::pressure_relief();
            }
            Command::Wake => continue,
            Command::Measure { reply } => {
                let mut measurements = Vec::new();
                if let Ok(live) = &current {
                    measurements.push(GenerationMemory {
                        generation: live.generation.clone(),
                        retired: false,
                        memory: live.vm.memory(),
                    });
                }
                for live in retained.values() {
                    measurements.push(GenerationMemory {
                        generation: live.generation.clone(),
                        retired: true,
                        memory: live.vm.memory(),
                    });
                }
                complete(reply, Ok(measurements));
            }
            Command::Pin { reply } => complete(
                reply,
                current
                    .as_ref()
                    .map(|live| GenerationPin {
                        generation: live.generation.clone(),
                        live: Some(live.pins.clone()),
                        notifier: notifier.clone(),
                    })
                    .map_err(Clone::clone),
            ),
            Command::Eval {
                id,
                source,
                expected,
                svelte,
                reply,
            } => {
                let source = match source {
                    Source::Text(source) => source,
                    Source::Prepare(prepare) => match prepare() {
                        Ok(source) if source.len() <= config.limits.max_source_bytes => source,
                        Ok(_) => {
                            complete(reply, Err(Error::Quota("source bytes")));
                            continue;
                        }
                        Err(error) => {
                            complete(reply, Err(error));
                            continue;
                        }
                    },
                };
                let source = if let Some((filename, unit)) = svelte {
                    match crate::compiler::compile_expression(&source, &filename, unit) {
                        Ok(source) => source,
                        Err(e) => {
                            complete(reply, Err(e));
                            continue;
                        }
                    }
                } else {
                    source
                };
                let result = match &mut current {
                    Ok(live) if expected.as_ref().is_none_or(|g| g == &live.generation) => {
                        // Keep the watchdog armed through effect serialization and commit.
                        let guard = Guard::new(&live.vm, &live.generation, id, &config, &control);
                        let result = live.run(&source, id, &config, &control).and_then(|value| {
                            live.commit(Some(&live.generation.clone()), &config, false)?;
                            Ok(Evaluation {
                                generation: live.generation.clone(),
                                value,
                                cache_hits: live.vm.cache_hits,
                            })
                        });
                        drop(guard);
                        result
                    }
                    Ok(_) => Err(Error::StaleGeneration),
                    Err(error) => Err(error.clone()),
                };
                if result.as_ref().is_err_and(|error| {
                    control.report_panel_memory(error, current.as_ref().ok(), &name, &config.limits)
                }) {
                    complete(reply, result);
                    break;
                }
                if let Ok(live) = &mut current {
                    if matches!(
                        result,
                        Err(Error::Terminated
                            | Error::Quota(HEAP_FULL | "heap" | "external memory")
                            | Error::Closed)
                    ) {
                        fail_generation(live, &config);
                        drop(std::mem::replace(&mut current, Err(Error::Closed)));
                        current = recover(&name, &kind, &config);
                    } else if result.is_err() {
                        let guard = Guard::new(&live.vm, &live.generation, id, &config, &control);
                        let restored = live.restore_storage();
                        drop(guard);
                        if restored.is_err() {
                            fail_generation(live, &config);
                            drop(std::mem::replace(&mut current, Err(Error::Closed)));
                            current = recover(&name, &kind, &config);
                        }
                    }
                }
                complete(reply, result);
            }
            Command::Reload { id, source, reply } => {
                if !matches!(kind, RealmKind::V4Extension(_)) {
                    complete(
                        reply,
                        Err(Error::Invalid(
                            "Only v4 extensions hot reload in separate generations",
                        )),
                    );
                    continue;
                }
                let result = (|| -> Result<(Live, String)> {
                    let old = current.as_ref().map_err(Clone::clone)?;
                    let reservation = config
                        .admission
                        .reserve(HeapPlan::new(&config.limits).initial())?;
                    let generation = Generation {
                        realm: name.clone(),
                        number: next_generation(),
                    };
                    let mut candidate = Live::new(
                        generation,
                        &kind,
                        &config,
                        old.storage.clone(),
                        old.hot.clone(),
                        reservation,
                    )?;
                    let guard =
                        Guard::new(&candidate.vm, &candidate.generation, id, &config, &control);
                    let result = candidate
                        .run(&source, id, &config, &control)
                        .and_then(|value| {
                            candidate.commit(Some(&old.generation), &config, false)?;
                            Ok(value)
                        });
                    drop(guard);
                    match result {
                        Ok(value) => Ok((candidate, value)),
                        Err(error) => {
                            fail_generation(&mut candidate, &config);
                            Err(error)
                        }
                    }
                })();
                match result {
                    Ok((candidate, value)) => {
                        let result = Evaluation {
                            generation: candidate.generation.clone(),
                            value,
                            cache_hits: candidate.vm.cache_hits,
                        };
                        if let Ok(mut old) = std::mem::replace(&mut current, Ok(candidate)) {
                            old.cancel_io();
                            let guard = Guard::new(&old.vm, &old.generation, id, &config, &control);
                            let retired = old.vm.retire();
                            drop(guard);
                            config.kernel.revoke_generation(&old.generation);
                            config.kernel.seal_open_gestures(&old.generation);
                            if retired.is_ok() && Arc::strong_count(&old.pins) > 1 {
                                retained.insert(old.generation.number, old);
                            }
                        }
                        complete(reply, Ok(result));
                    }
                    Err(error) => complete(reply, Err(error)),
                }
            }
            Command::Retained {
                id,
                pin,
                source,
                reply,
            } => {
                let is_current = current.as_ref().is_ok_and(|v| {
                    v.generation == pin.generation
                        && pin.live.as_ref().is_some_and(|p| Arc::ptr_eq(&v.pins, p))
                });
                let live = if is_current {
                    current.as_mut().ok()
                } else {
                    retained.get_mut(&pin.generation.number).filter(|v| {
                        v.generation == pin.generation
                            && pin.live.as_ref().is_some_and(|p| Arc::ptr_eq(&v.pins, p))
                    })
                };
                let result = if let Some(live) = live {
                    let guard = Guard::new(&live.vm, &live.generation, id, &config, &control);
                    let result = live.run(&source, id, &config, &control).and_then(|value| {
                        live.commit(None, &config, true)?;
                        Ok(Evaluation {
                            generation: live.generation.clone(),
                            value,
                            cache_hits: live.vm.cache_hits,
                        })
                    });
                    drop(guard);
                    result
                } else {
                    Err(Error::StaleGeneration)
                };
                let mut failed = matches!(
                    result,
                    Err(Error::Terminated
                        | Error::Quota(HEAP_FULL | "heap" | "external memory")
                        | Error::Closed)
                );
                if result.is_err() && !failed {
                    let live = if is_current {
                        current.as_mut().ok()
                    } else {
                        retained.get_mut(&pin.generation.number)
                    };
                    if let Some(live) = live {
                        let guard = Guard::new(&live.vm, &live.generation, id, &config, &control);
                        failed = live.restore_storage().is_err();
                        drop(guard);
                    }
                }
                if failed {
                    if is_current {
                        if let Ok(live) = &mut current {
                            fail_generation(live, &config);
                        }
                        drop(std::mem::replace(&mut current, Err(Error::Closed)));
                        current = recover(&name, &kind, &config);
                    } else if let Some(mut live) = retained.remove(&pin.generation.number) {
                        fail_generation(&mut live, &config);
                    }
                }
                complete(reply, result);
            }
        }
    }
    if let Ok(live) = &mut current {
        fail_generation(live, &config);
    }
    for live in retained.values_mut() {
        fail_generation(live, &config);
    }
    drop(current);
    drop(retained);
    control.closing.store(true, Ordering::Release);
    drop(receiver);
    control.disposed.store(true, Ordering::Release);
}
/// How long an isolate that grew waits without work before it trims.
const IDLE_TRIM: Duration = Duration::from_secs(5);
fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
fn recover(name: &str, kind: &RealmKind, config: &Config) -> Result<Live> {
    let generation = Generation {
        realm: name.into(),
        number: next_generation(),
    };
    let reservation = config
        .admission
        .reserve(HeapPlan::new(&config.limits).initial())?;
    let storage = config.kernel.load_storage(name)?;
    Live::new(generation, kind, config, storage, json!({}), reservation)
}
