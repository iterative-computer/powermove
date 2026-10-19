//! Opt-in cold-start timing. Disabled in normal playback; no callback work.
use std::{
    cell::{Cell, RefCell},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, channel, sync_channel},
    },
    time::{Duration, Instant},
};

const WORKER_EVENT_CAP: usize = 2_048;
static WORKER_LOGGER: OnceLock<WorkerLogger> = OnceLock::new();

enum WorkerEvent {
    Line(String),
    Flush(std::sync::mpsc::Sender<()>),
}

struct WorkerLogger {
    sender: SyncSender<WorkerEvent>,
    dropped: AtomicU64,
    flush_failures: AtomicU64,
}

impl WorkerLogger {
    fn new(spawn: impl FnOnce(Receiver<WorkerEvent>)) -> Self {
        let (sender, receiver) = sync_channel(WORKER_EVENT_CAP);
        // A failed spawn drops the receiver and leaves a disconnected sender.
        // Audio continues, with observable diagnostic-loss counters.
        spawn(receiver);
        Self {
            sender,
            dropped: AtomicU64::new(0),
            flush_failures: AtomicU64::new(0),
        }
    }

    fn line(&self, line: String) {
        if self.sender.try_send(WorkerEvent::Line(line)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn flush(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (reply, acknowledgement) = channel();
        let mut event = WorkerEvent::Flush(reply);
        loop {
            match self.sender.try_send(event) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) => {
                    event = returned;
                    if Instant::now() >= deadline {
                        self.dropped.fetch_add(1, Ordering::Relaxed);
                        self.flush_failures.fetch_add(1, Ordering::Relaxed);
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    self.flush_failures.fetch_add(1, Ordering::Relaxed);
                    return false;
                }
            }
        }
        if acknowledgement
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_ok()
        {
            true
        } else {
            self.flush_failures.fetch_add(1, Ordering::Relaxed);
            false
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WorkerEventStats {
    pub(crate) dropped: u64,
    pub(crate) flush_failures: u64,
}

/// Called from the ordinary Playback constructor before the preparation pool
/// starts. A failed diagnostic thread cannot fail audio playback: events then
/// report a disconnected channel through the drop and flush counters.
pub(crate) fn init_worker_events() {
    if !enabled() {
        return;
    }
    WORKER_LOGGER.get_or_init(|| {
        WorkerLogger::new(|receiver| {
            let _ = std::thread::Builder::new()
                .name("pm-audio-profile-writer".into())
                .spawn(move || {
                    while let Ok(event) = receiver.recv() {
                        match event {
                            WorkerEvent::Line(line) => eprintln!("{line}"),
                            WorkerEvent::Flush(reply) => {
                                let _ = reply.send(());
                            }
                        }
                    }
                });
        })
    });
}

/// Formatting happens after the worker's audio guard is dropped. Backpressure
/// discards diagnostics instead of waiting for stderr or queue capacity.
/// `try_send` can still perform a bounded receiver wake after leaving RT.
pub(crate) fn worker_event(line: String) {
    let Some(logger) = WORKER_LOGGER.get() else {
        debug_assert!(!enabled(), "worker logger was not initialized");
        return;
    };
    logger.line(line);
}

pub(crate) fn worker_event_stats() -> WorkerEventStats {
    WORKER_LOGGER
        .get()
        .map_or(WorkerEventStats::default(), |logger| WorkerEventStats {
            dropped: logger.dropped.load(Ordering::Relaxed),
            flush_failures: logger.flush_failures.load(Ordering::Relaxed),
        })
}

/// A bounded barrier for an ordinary shutdown caller after the producer has
/// joined. It preserves event order without waiting indefinitely on stderr.
pub(crate) fn flush_worker_events() -> bool {
    if !enabled() {
        return true;
    }
    debug_assert!(!crate::realtime::has_active_guard());
    let Some(logger) = WORKER_LOGGER.get() else {
        return false;
    };
    if crate::realtime::has_active_guard() {
        logger.flush_failures.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    logger.flush(Duration::from_millis(100))
}

#[cfg(test)]
mod worker_logger_tests {
    use super::*;

    #[test]
    fn full_and_disconnected_writers_report_dropped_events() {
        let (sender, receiver) = sync_channel(1);
        let logger = WorkerLogger {
            sender,
            dropped: AtomicU64::new(0),
            flush_failures: AtomicU64::new(0),
        };
        logger.line("first".into());
        logger.line("full".into());
        assert_eq!(logger.dropped.load(Ordering::Relaxed), 1);
        assert!(!logger.flush(Duration::ZERO));
        assert_eq!(logger.flush_failures.load(Ordering::Relaxed), 1);
        assert!(matches!(receiver.recv().unwrap(), WorkerEvent::Line(line) if line == "first"));
        drop(receiver);
        logger.line("disconnected".into());
        assert_eq!(logger.dropped.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn a_failed_writer_spawn_is_observable_without_failing_audio() {
        let logger = WorkerLogger::new(drop);
        logger.line("discarded".into());
        assert!(!logger.flush(Duration::ZERO));
        assert_eq!(logger.dropped.load(Ordering::Relaxed), 2);
        assert_eq!(logger.flush_failures.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_stalled_writer_cannot_turn_a_flush_timeout_into_success() {
        let (sender, receiver) = sync_channel(1);
        let logger = WorkerLogger {
            sender,
            dropped: AtomicU64::new(0),
            flush_failures: AtomicU64::new(0),
        };
        assert!(!logger.flush(Duration::ZERO));
        assert_eq!(logger.flush_failures.load(Ordering::Relaxed), 1);
        assert!(matches!(receiver.recv().unwrap(), WorkerEvent::Flush(_)));
    }

    #[test]
    fn a_flush_acknowledges_only_after_earlier_lines() {
        let (seen, lines) = channel();
        let logger = WorkerLogger::new(|receiver| {
            std::thread::spawn(move || {
                while let Ok(event) = receiver.recv() {
                    match event {
                        WorkerEvent::Line(line) => seen.send(line).unwrap(),
                        WorkerEvent::Flush(reply) => reply.send(()).unwrap(),
                    }
                }
            });
        });
        logger.line("ordered".into());
        assert!(logger.flush(Duration::from_secs(5)));
        assert_eq!(lines.try_recv().unwrap(), "ordered");
        assert_eq!(logger.dropped.load(Ordering::Relaxed), 0);
        assert_eq!(logger.flush_failures.load(Ordering::Relaxed), 0);
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub bounds_us: u64,
    pub map_us: u64,
    pub read_us: u64,
    pub dsp_us: u64,
    pub dsp_init_us: u64,
    pub dsp_seek_us: u64,
    pub dsp_process_us: u64,
    pub hold_us: u64,
    /// Producer wall wait, overlaps the summed worker scopes above.
    pub prepare_wait_us: u64,
    pub first_render_us: u64,
    pub max_unit_ns: u64,
    pub max_unit_phase: &'static str,
}
impl Profile {
    /// Merge scopes from an owned task which may resume on another worker.
    pub(crate) fn merge(&mut self, other: Self) {
        self.bounds_us += other.bounds_us;
        self.map_us += other.map_us;
        self.read_us += other.read_us;
        self.dsp_us += other.dsp_us;
        self.dsp_init_us += other.dsp_init_us;
        self.dsp_seek_us += other.dsp_seek_us;
        self.dsp_process_us += other.dsp_process_us;
        self.hold_us += other.hold_us;
        self.prepare_wait_us += other.prepare_wait_us;
        self.first_render_us += other.first_render_us;
        if other.max_unit_ns > self.max_unit_ns {
            self.max_unit_ns = other.max_unit_ns;
            self.max_unit_phase = other.max_unit_phase;
        }
    }
}
thread_local! { static COUNTS: Cell<Profile> = Cell::new(Profile::default()); }
thread_local! { static PHASE: Cell<&'static str> = const { Cell::new("") }; }
const CONTROL_EVENT_CAP: usize = 128;
struct ControlEvent {
    label: &'static str,
    start_us: u64,
    end_us: u64,
    cpu_ns: u64,
    throttle_sleep_ns: u64,
}
#[derive(Default)]
struct ControlEvents {
    events: Vec<ControlEvent>,
    dropped: u64,
}
thread_local! { static CONTROL_EVENTS: RefCell<ControlEvents> = RefCell::new(ControlEvents::default()); }

pub(crate) fn init_control_events() {
    if enabled() {
        CONTROL_EVENTS.with(|cell| cell.borrow_mut().events.reserve_exact(CONTROL_EVENT_CAP));
    }
}

/// A point event for control receipt state, with no overlapping duration.
pub(crate) fn control_mark(label: &'static str) {
    if !enabled() {
        return;
    }
    let at = crate::clock::host_now_us();
    CONTROL_EVENTS.with(|cell| {
        let mut events = cell.borrow_mut();
        if events.events.len() < CONTROL_EVENT_CAP {
            events.events.push(ControlEvent {
                label,
                start_us: at,
                end_us: at,
                cpu_ns: 0,
                throttle_sleep_ns: 0,
            });
        } else {
            events.dropped += 1;
        }
    });
}

pub(crate) struct ControlTimer(Option<(&'static str, u64, u64, u64)>);
pub(crate) fn control_timer(label: &'static str) -> ControlTimer {
    ControlTimer(enabled().then(|| {
        (
            label,
            crate::clock::host_now_us(),
            crate::realtime::thread_cpu_ns(),
            crate::realtime::thread_throttle_sleep_ns(),
        )
    }))
}
impl Drop for ControlTimer {
    fn drop(&mut self) {
        let Some((label, start_us, cpu_before, throttle_before)) = self.0 else {
            return;
        };
        let event = ControlEvent {
            label,
            start_us,
            end_us: crate::clock::host_now_us(),
            cpu_ns: crate::realtime::thread_cpu_ns().saturating_sub(cpu_before),
            throttle_sleep_ns: crate::realtime::thread_throttle_sleep_ns()
                .saturating_sub(throttle_before),
        };
        CONTROL_EVENTS.with(|cell| {
            let mut events = cell.borrow_mut();
            if events.events.len() < CONTROL_EVENT_CAP {
                events.events.push(event);
            } else {
                events.dropped += 1;
            }
        });
    }
}

/// Called only after leaving the producer's real-time schedule. The control
/// trace is independent of `reset`, which Play uses for its DSP counters.
pub(crate) fn flush_control_events() {
    if !enabled() {
        return;
    }
    debug_assert!(!crate::realtime::has_active_guard());
    CONTROL_EVENTS.with(|cell| {
        let mut events = cell.borrow_mut();
        for event in events.events.drain(..) {
            eprintln!(
                "pm-audio-control label={} start_us={} end_us={} wall_us={} cpu_ns={} throttle_sleep_ns={}",
                event.label,
                event.start_us,
                event.end_us,
                event.end_us.saturating_sub(event.start_us),
                event.cpu_ns,
                event.throttle_sleep_ns,
            );
        }
        if events.dropped != 0 {
            eprintln!("pm-audio-control dropped={}", events.dropped);
            events.dropped = 0;
        }
    });
}
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PM_AUDIO_PROFILE").is_some())
}
pub(crate) fn reset() {
    COUNTS.set(Profile::default());
    PHASE.set("");
}
pub(crate) fn snapshot() -> Profile {
    COUNTS.get()
}
pub(crate) fn phase(label: &'static str) {
    PHASE.set(label);
}
#[cfg(target_os = "macos")]
pub(crate) fn record_unit(unit_ns: u64) {
    let label = PHASE.get();
    COUNTS.with(|counts| {
        let mut profile = counts.get();
        if unit_ns > profile.max_unit_ns {
            profile.max_unit_ns = unit_ns;
            profile.max_unit_phase = label;
            counts.set(profile);
        }
    });
}

pub(crate) enum Kind {
    Bounds,
    Map,
    Read,
    Dsp,
    DspInit,
    DspSeek,
    DspProcess,
    Hold,
    PrepareWait,
    FirstRender,
}
pub(crate) struct Timer(Option<Instant>, Kind);
pub(crate) fn timer(kind: Kind) -> Timer {
    Timer(enabled().then(Instant::now), kind)
}
impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(start) = self.0 {
            let us = start.elapsed().as_nanos() as f64 / 1000.0;
            COUNTS.with(|c| {
                let mut p = c.get();
                let field = match self.1 {
                    Kind::Bounds => &mut p.bounds_us,
                    Kind::Map => &mut p.map_us,
                    Kind::Read => &mut p.read_us,
                    Kind::Dsp => &mut p.dsp_us,
                    Kind::DspInit => &mut p.dsp_init_us,
                    Kind::DspSeek => &mut p.dsp_seek_us,
                    Kind::DspProcess => &mut p.dsp_process_us,
                    Kind::Hold => &mut p.hold_us,
                    Kind::PrepareWait => &mut p.prepare_wait_us,
                    Kind::FirstRender => &mut p.first_render_us,
                };
                *field += us.round() as u64;
                c.set(p);
            });
        }
    }
}

pub(crate) fn add(other: Profile) {
    COUNTS.with(|c| {
        let mut p = c.get();
        p.merge(other);
        c.set(p);
    });
}
