//! The running telemetry. The app's threads `try_send` into a bounded
//! channel; one thread (the worker) owns everything that leaves the process
//! or touches disk: batching, sending, the outbox, crash import.
//!
//! Consent (Sol review, P0): Settings' sharing value is published in two
//! atomics outside the event queue, `CONSENT` (unknown, on, off) and
//! `GENERATION` (bumped on every change). Until Settings is known, events
//! wait in memory only. The worker checks consent and generation before every
//! request and every disk write; a revocation drops what is waiting in memory
//! and on disk, and every message captured before it (by generation). The
//! SDK never retries by itself: one attempt per request, retries are the
//! worker's own, under the same checks. A request already on the wire when
//! sharing is turned off cannot be recalled; nothing starts after.
mod crash;
pub mod exception;
mod outbox;
pub mod perf;
mod system;

use crate::events::*;
use crate::{Config, Mode, prefs};
use outbox::{Outbox, Record};
use serde_json::{Map, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Events waiting for the telemetry thread before new ones are dropped.
const QUEUE: usize = 2048;
/// Events the thread keeps in memory (waiting for Settings, or between sends).
const MAX_PENDING: usize = 1000;
/// The thread wakes this often to fold commands and keep its schedule.
const TICK: Duration = Duration::from_secs(1);
const SEND_EVERY: Duration = Duration::from_secs(30);
const SEND_AT: usize = 50;
const SUMMARY_EVERY: Duration = Duration::from_secs(300);
const SAMPLE_EVERY: Duration = Duration::from_secs(10);
const FOLD: Duration = Duration::from_secs(1);
/// The longest a quit waits for the last events to leave.
const QUIT_WAIT: Duration = Duration::from_millis(1500);
const HANG_MS: u64 = 2000;
/// Crash reports sent per launch (the rest wait for the next).
const CRASHES_PER_LAUNCH: usize = 10;

const UNKNOWN: u8 = 0;
const ON: u8 = 1;
const OFF: u8 = 2;
static CONSENT: AtomicU8 = AtomicU8::new(UNKNOWN);
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// The generation of the last turn to off: everything captured before it is
/// dropped, even if sharing is back on before the worker looks.
static REVOKED: AtomicU64 = AtomicU64::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(true);
static DROPPED: AtomicU64 = AtomicU64::new(0);

enum Msg {
    Event(Event, i64, u64),
    Launched(u64, u64, i64, u64),
    Identify(String, u64),
    /// Raw return addresses and a scrubbed location, from the panic hook.
    Panic(Vec<u64>, Option<(String, u32)>, i64, u64),
    HangBegan(i64, u64),
    HangEnded(&'static str, i64, u64),
    StoreInstalled(String, bool, i64, u64),
    StoreRemoved(String, i64, u64),
    Wake,
    #[cfg(test)]
    Flush(SyncSender<()>),
    Quit(bool, SyncSender<()>),
}

struct Shared {
    tx: SyncSender<Msg>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

pub fn running() -> bool {
    SHARED.get().is_some()
}

/// Captures are taken (sharing on, or Settings not read yet: held in memory).
#[inline]
pub fn taking() -> bool {
    CONSENT.load(Relaxed) != OFF && SHARED.get().is_some()
}

fn generation() -> u64 {
    GENERATION.load(Relaxed)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn send(msg: Msg) -> bool {
    SHARED.get().is_some_and(|s| s.tx.try_send(msg).is_ok())
}

pub fn start(config: Config, mode: Mode) {
    let (tx, rx) = sync_channel(QUEUE);
    if SHARED.set(Shared { tx }).is_err() {
        return;
    }
    install_panic_hook();
    let spawned = std::thread::Builder::new()
        .name("pm-telemetry".into())
        .spawn(move || Worker::new(config, mode).run(rx));
    if spawned.is_ok() {
        let _ = std::thread::Builder::new()
            .name("pm-telemetry-hang".into())
            .spawn(watch_hangs);
    }
}

/// Settings' value changed (or arrived): published at once, outside the
/// event queue; the worker follows it before its next request or write.
pub fn set_sharing(on: bool) {
    let value = if on { ON } else { OFF };
    if CONSENT.swap(value, Relaxed) != value {
        let generation = GENERATION.fetch_add(1, Relaxed) + 1;
        if !on {
            REVOKED.store(generation, Relaxed);
            perf::clear();
        }
        send(Msg::Wake);
    }
}

#[inline]
pub fn capture(event: Event) {
    if taking() && !send(Msg::Event(event, now_ms(), generation())) {
        DROPPED.fetch_add(1, Relaxed);
    }
}

pub fn launched(first_window_ms: u64, interactive_ms: u64) {
    if taking() {
        send(Msg::Launched(
            first_window_ms,
            interactive_ms,
            now_ms(),
            generation(),
        ));
    }
}

pub fn since_process_start_ms() -> u64 {
    system::process_start().map_or(0, |start| {
        ((now_ms() as f64 / 1000. - start).max(0.) * 1000.) as u64
    })
}

pub fn identify(account_id: &str) {
    if let Some(id) = clean_id(&account_id.to_ascii_lowercase())
        && taking()
    {
        send(Msg::Identify(id, generation()));
    }
}

pub fn set_active(active: bool) {
    ACTIVE.store(active, Relaxed);
}

pub fn store_installed(id: &str, fresh: bool) {
    if let Some(id) = clean_id(id) {
        send(Msg::StoreInstalled(id, fresh, now_ms(), generation()));
    }
}

pub fn store_removed(id: &str) {
    if let Some(id) = clean_id(id) {
        send(Msg::StoreRemoved(id, now_ms(), generation()));
    }
}

pub fn quit(clean: bool) {
    let (ack, done) = sync_channel(1);
    if send(Msg::Quit(clean, ack)) {
        let _ = done.recv_timeout(QUIT_WAIT);
    }
}

enum Delivery {
    Sent,
    /// Offline or PostHog busy: keep it for later.
    Retry,
    /// Refused for good, or sharing changed: drop it.
    Dropped,
}

struct Worker {
    profile: PathBuf,
    mode: Mode,
    build: String,
    app_version: String,
    displays: (u32, f32),
    bundle_id: String,
    metal: Option<Arc<dyn Fn() -> u64 + Send + Sync>>,
    state: prefs::State,
    outbox: Outbox,
    distinct_id: String,
    supers: Map<String, Value>,
    main: Option<exception::Image>,
    client: Option<posthog::Client>,
    pending: Vec<Record>,
    /// A command waiting for repeats: the event, its first time, its last.
    folding: Option<(CommandInvoked, i64, Instant)>,
    started: Instant,
    last_summary: Instant,
    last_sample: Instant,
    next_send: Instant,
    failures: u32,
    peaks: perf::Peaks,
    crash_last_session: bool,
    cold: bool,
    first_run: bool,
    /// The consent generation this worker last acted on.
    seen: u64,
    /// The generation of the last revocation: older work is dropped.
    revoked: u64,
    was_off: bool,
    crashes_read: bool,
    hanging: bool,
}

impl Worker {
    fn new(config: Config, mode: Mode) -> Worker {
        let now = Instant::now();
        Worker {
            outbox: Outbox::new(&config.profile),
            profile: config.profile,
            mode,
            build: config.build,
            app_version: config.app_version,
            displays: config.displays,
            bundle_id: config.bundle_id.unwrap_or_default(),
            metal: config.metal_bytes,
            state: prefs::State::default(),
            distinct_id: String::new(),
            supers: Map::new(),
            main: None,
            client: None,
            pending: vec![],
            folding: None,
            started: now,
            last_summary: now,
            last_sample: now - SAMPLE_EVERY,
            next_send: now + Duration::from_secs(5),
            failures: 0,
            peaks: perf::Peaks::default(),
            crash_last_session: false,
            cold: false,
            first_run: false,
            seen: 0,
            revoked: 0,
            was_off: false,
            crashes_read: false,
            hanging: false,
        }
    }

    fn internal(&self) -> bool {
        self.mode == Mode::Internal
    }

    fn save(&self) {
        let _ = prefs::store(&self.profile, &self.state);
    }

    /// Off the main thread: the state, the Mac's facts, this session's record.
    fn begin(&mut self) {
        self.state = prefs::load(&self.profile);
        crate::remember_store_extensions(&self.state.store_installs);
        self.crash_last_session = self.state.session_open;
        self.first_run = self.state.install_id.is_none();
        self.distinct_id = self
            .state
            .install_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        let boot = system::boot_time();
        self.cold = boot.is_some() && self.state.last_boot != boot;
        self.state.last_boot = boot;
        self.state.session_open = true;
        self.state.sessions.push(prefs::Session {
            pid: std::process::id() as u64,
            started: system::process_start().unwrap_or(now_ms() as f64 / 1000.),
            internal: self.internal(),
            ..Default::default()
        });
        prefs::cap(&mut self.state.sessions, prefs::MAX_SESSIONS);
        self.save();
        self.main = exception::main_image();
        let channel = match self.mode {
            Mode::Production { channel } => channel,
            _ => "dev",
        };
        let (language, region) = system::locale();
        let supers = SuperProperties {
            app_version: self.app_version.clone(),
            build: self.build.clone(),
            channel,
            macos_version: system::macos_version(),
            chip: system::chip(),
            ram_bucket: ram_bucket(system::ram_bytes()),
            display_count: self.displays.0,
            display_max_scale: self.displays.1,
            locale: language,
            region,
            is_internal: self.internal(),
            session_id: uuid::Uuid::now_v7().to_string(),
        };
        if let Ok(Value::Object(map)) = serde_json::to_value(&supers) {
            self.supers = map;
        }
    }

    fn session(&mut self) -> Option<&mut prefs::Session> {
        let pid = std::process::id() as u64;
        self.state.sessions.iter_mut().rev().find(|s| s.pid == pid)
    }

    fn run(mut self, rx: Receiver<Msg>) {
        self.begin();
        loop {
            self.follow_consent();
            match rx.recv_timeout(TICK) {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.follow_consent();
            self.tick();
        }
    }

    /// Acts on a consent change: off purges everything waiting (memory and
    /// disk); on starts the client and, once, reads crash reports.
    fn follow_consent(&mut self) {
        let generation = generation();
        if generation == self.seen {
            return;
        }
        self.seen = generation;
        // A turn to off since the last look (even one already turned back
        // on): everything waiting goes.
        let revoked = REVOKED.load(Relaxed);
        if revoked != self.revoked {
            self.revoked = revoked;
            self.purge();
            self.was_off = true;
            if let Some(session) = self.session() {
                session.revoked = true;
            }
            self.save();
        }
        if CONSENT.load(Relaxed) == ON {
            if self.client.is_none() {
                self.client = client();
            }
            if std::mem::take(&mut self.was_off) {
                let event = Event::SettingsChanged(SettingsChanged::Telemetry(true));
                self.add(event, now_ms(), generation);
            }
            if let Some(session) = self.session() {
                session.consented = true;
            }
            self.save();
            if !self.crashes_read {
                self.read_crash_reports();
            }
            self.next_send = Instant::now();
        }
    }

    fn purge(&mut self) {
        self.client = None;
        self.pending.clear();
        self.folding = None;
        self.hanging = false;
        perf::clear();
        self.peaks = perf::Peaks::default();
        DROPPED.store(0, Relaxed);
        self.outbox.clear();
    }

    /// Sharing on, and no change since the worker last looked.
    fn may_write(&self) -> bool {
        CONSENT.load(Relaxed) == ON && generation() == self.seen
    }

    fn may_send(&self) -> bool {
        self.may_write() && self.client.is_some()
    }

    /// Work captured before the last revocation, or while off, is dropped.
    fn current(&self, captured: u64) -> bool {
        captured >= REVOKED.load(Relaxed) && CONSENT.load(Relaxed) != OFF
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Event(event, at, g) if self.current(g) => self.add(event, at, g),
            Msg::Launched(first_window_ms, interactive_ms, at, g) if self.current(g) => {
                let event = Event::AppLaunched(AppLaunched {
                    kind: if self.cold {
                        LaunchKind::Cold
                    } else {
                        LaunchKind::Warm
                    },
                    launch_to_first_window_ms: first_window_ms,
                    launch_to_interactive_ms: interactive_ms,
                    first_run: self.first_run,
                    crash_last_session: self.crash_last_session,
                });
                self.add(event, at, g);
            }
            Msg::Identify(id, g) if self.current(g) => self.identify(id),
            Msg::Panic(addrs, location, at, g) if self.current(g) => {
                self.panic(addrs, location, at)
            }
            Msg::HangBegan(at, g) if self.current(g) => {
                self.hanging = true;
                // Kept in case the app is ended while it hangs.
                if self.may_write()
                    && let Some(record) = self.event_record(&hang_event("unrecovered"), at)
                {
                    let _ = self.outbox.write(outbox::HANG_PENDING, &[record]);
                }
            }
            Msg::HangEnded(duration, at, g) => {
                let _ = std::fs::remove_file(self.outbox.dir().join(outbox::HANG_PENDING));
                if std::mem::take(&mut self.hanging) && self.current(g) {
                    self.add(hang_event(duration), at, g);
                }
            }
            Msg::StoreInstalled(id, fresh, at, g) => {
                // Provenance is remembered whatever sharing says; the event
                // follows consent.
                if !self.state.store_installs.contains(&id) {
                    self.state.store_installs.push(id.clone());
                    prefs::cap(&mut self.state.store_installs, prefs::MAX_STORE_INSTALLS);
                    crate::remember_store_extensions(&self.state.store_installs);
                    self.save();
                }
                if fresh && self.current(g) {
                    let event = Event::StoreExtensionInstalled(StoreExtension { extension_id: id });
                    self.add(event, at, g);
                }
            }
            Msg::StoreRemoved(id, at, g) => {
                if self.state.store_installs.contains(&id) {
                    self.state.store_installs.retain(|s| s != &id);
                    crate::remember_store_extensions(&self.state.store_installs);
                    self.save();
                    if self.current(g) {
                        let event =
                            Event::StoreExtensionRemoved(StoreExtension { extension_id: id });
                        self.add(event, at, g);
                    }
                }
            }
            #[cfg(test)]
            Msg::Flush(ack) => {
                self.send();
                let _ = ack.send(());
            }
            Msg::Quit(clean, ack) => {
                self.quit(clean);
                let _ = ack.send(());
            }
            _ => {}
        }
    }

    fn record(&self, name: &str, mut properties: Map<String, Value>, at: i64) -> Record {
        for (key, value) in &self.supers {
            properties
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
        // A development check's events are always marked internal.
        properties.insert("is_internal".into(), self.internal().into());
        Record {
            uuid: uuid::Uuid::now_v7().to_string(),
            event: name.to_owned(),
            distinct_id: self.distinct_id.clone(),
            timestamp_ms: at,
            properties,
        }
    }

    fn event_record(&self, event: &Event, at: i64) -> Option<Record> {
        let properties = match serde_json::to_value(event)
            .ok()?
            .get_mut("properties")?
            .take()
        {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        Some(self.record(event.name(), properties, at))
    }

    fn keep(&mut self, record: Record) {
        if self.pending.len() >= MAX_PENDING {
            self.pending.remove(0);
            DROPPED.fetch_add(1, Relaxed);
        }
        self.pending.push(record);
    }

    fn add(&mut self, event: Event, at: i64, captured: u64) {
        if !self.current(captured) {
            return;
        }
        if let Event::CommandInvoked(command) = &event {
            if let Some((held, _, last)) = &mut self.folding
                && held.command_id == command.command_id
                && held.via == command.via
                && last.elapsed() < FOLD
            {
                held.count += command.count;
                *last = Instant::now();
                return;
            }
            self.flush_fold();
            self.folding = Some((command.clone(), at, Instant::now()));
            return;
        }
        if let Some(record) = self.event_record(&event, at) {
            self.keep(record);
        }
    }

    fn flush_fold(&mut self) {
        if let Some((command, at, _)) = self.folding.take()
            && let Some(record) = self.event_record(&Event::CommandInvoked(command), at)
        {
            self.keep(record);
        }
    }

    fn identify(&mut self, id: String) {
        if self.state.identified.as_ref() == Some(&id) {
            return;
        }
        let mut props = Map::new();
        props.insert("$anon_distinct_id".into(), self.distinct_id.clone().into());
        let mut record = self.record("$identify", props, now_ms());
        record.distinct_id = id.clone();
        self.keep(record);
        self.state.identified = Some(id);
        self.save();
    }

    fn panic(&mut self, addrs: Vec<u64>, location: Option<(String, u32)>, at: i64) {
        let mut frames = exception::resolve(&addrs, self.main.as_ref());
        // The hook's own frames sit inside the panic dispatcher.
        let name = |f: &exception::Frame| f.function.clone().unwrap_or_default();
        if let Some(dispatcher) = frames
            .iter()
            .take(64)
            .position(|f| name(f).contains("panic_with_hook"))
        {
            frames.drain(..dispatcher);
        }
        let value = match &location {
            Some((file, line)) => format!("panic at {file}:{line}"),
            None => "panic".into(),
        };
        let mut props = exception::properties(
            exception::Kind {
                exception_type: "Panic",
                value,
                mechanism: "panic",
            },
            &frames,
            self.main.as_ref(),
        );
        if let Some((file, line)) = location {
            props.insert("$exception_panic_file".into(), file.into());
            props.insert("$exception_panic_line".into(), line.into());
        }
        let record = self.record("$exception", props, at);
        self.keep(record);
        self.next_send = Instant::now();
    }

    fn tick(&mut self) {
        if CONSENT.load(Relaxed) != ON {
            return;
        }
        if self
            .folding
            .as_ref()
            .is_some_and(|(_, _, last)| last.elapsed() >= FOLD)
        {
            self.flush_fold();
        }
        if self.last_sample.elapsed() >= SAMPLE_EVERY {
            self.last_sample = Instant::now();
            self.peaks.sample(self.metal.as_deref());
        }
        if self.last_summary.elapsed() >= SUMMARY_EVERY {
            self.summary();
        }
        let due = Instant::now() >= self.next_send;
        if due || (self.pending.len() >= SEND_AT && self.failures == 0) {
            self.send();
        }
    }

    fn summary(&mut self) {
        let window_s = self.last_summary.elapsed().as_secs();
        self.last_summary = Instant::now();
        self.peaks.sample(self.metal.as_deref());
        let (frames, playback) = perf::take();
        let peaks = std::mem::take(&mut self.peaks);
        if frames.0.iter().all(Option::is_none) {
            return;
        }
        let event = Event::PerfSummary(Box::new(PerfSummary {
            window_s,
            frames,
            playback_fps_achieved: playback.map(|p| p.0),
            playback_fps_target: playback.and_then(|p| p.1),
            peak_footprint_mb: peaks.footprint >> 20,
            peak_metal_mb: peaks.metal.map(|m| m >> 20),
            thermal_state_max: perf::thermal_name(peaks.thermal),
            on_battery: peaks.on_battery,
            telemetry_dropped: DROPPED.swap(0, Relaxed),
        }));
        self.add(event, now_ms(), self.seen);
    }

    /// One attempt, under the consent check; a development check's batch
    /// never carries an event that isn't marked internal.
    fn deliver(&mut self, records: &[Record]) -> Delivery {
        if !self.may_send() {
            return Delivery::Dropped;
        }
        let internal = self.internal();
        let events: Vec<posthog::Event> = records
            .iter()
            .filter(|r| admissible(internal, r))
            .map(Record::to_posthog)
            .collect();
        if events.is_empty() {
            return Delivery::Sent;
        }
        let Some(client) = &self.client else {
            return Delivery::Dropped;
        };
        let result = client.capture_batch_immediate(events, false);
        // Sharing may have been turned off while the request ran.
        self.follow_consent();
        match result {
            Ok(_) => Delivery::Sent,
            Err(
                posthog::Error::BadRequest(_)
                | posthog::Error::Serialization(_)
                | posthog::Error::InvalidArgument(_),
            ) => Delivery::Dropped,
            Err(_) if self.may_write() => Delivery::Retry,
            Err(_) => Delivery::Dropped,
        }
    }

    fn send(&mut self) {
        self.flush_fold();
        if !self.may_send() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        let ok = batch.is_empty()
            || match self.deliver(&batch) {
                Delivery::Sent | Delivery::Dropped => true,
                Delivery::Retry => {
                    let _ = self.outbox.write(&Outbox::batch_name("batch"), &batch);
                    false
                }
            };
        if ok {
            self.failures = 0;
            self.next_send = Instant::now() + SEND_EVERY;
            self.drain_outbox();
        } else {
            self.failures += 1;
            let backoff = SEND_EVERY * 2u32.saturating_pow(self.failures.min(5));
            self.next_send = Instant::now() + backoff.min(Duration::from_secs(600));
        }
    }

    /// Batches earlier sends or sessions left, oldest first, while PostHog
    /// takes them and sharing stays on.
    fn drain_outbox(&mut self) {
        for file in self.outbox.pending(false).into_iter().take(10) {
            if !self.may_send() {
                return;
            }
            let records = Outbox::read(&file);
            match self.deliver(&records) {
                Delivery::Retry => {
                    self.failures += 1;
                    self.next_send = Instant::now() + SEND_EVERY;
                    return;
                }
                _ if self.may_write() => {
                    let _ = std::fs::remove_file(&file);
                }
                _ => return,
            }
        }
    }

    /// Crash reports of recorded sessions that were sending, each once.
    fn read_crash_reports(&mut self) {
        self.crashes_read = true;
        let Some(folder) = crash::reports_folder() else {
            return;
        };
        let now = now_ms() as f64 / 1000.;
        let mut sent = 0;
        for path in crash::candidates(&folder, now) {
            if !self.may_write() || sent >= CRASHES_PER_LAUNCH {
                break;
            }
            let Some(report) = crash::read(&path).and_then(|t| crash::parse(&t, &self.bundle_id))
            else {
                continue;
            };
            if self.state.crash_incidents.contains(&report.incident) {
                continue;
            }
            let session = sending_session(&self.state.sessions, report.pid, report.launched);
            if let Some(session) = session {
                let mut record = self.record(
                    "$exception",
                    report.properties,
                    (report.time * 1000.) as i64,
                );
                record.uuid = crash::event_uuid(&report.incident);
                record
                    .properties
                    .insert("is_internal".into(), session.internal.into());
                // Kept on disk before it counts as handled.
                if self
                    .outbox
                    .write(&Outbox::batch_name("crash"), &[record])
                    .is_err()
                {
                    break;
                }
                sent += 1;
            }
            self.state.crash_incidents.push(report.incident);
            prefs::cap(&mut self.state.crash_incidents, prefs::MAX_INCIDENTS);
            self.save();
        }
        // A hang the last session never recovered from.
        let pending = self.outbox.dir().join(outbox::HANG_PENDING);
        if pending.exists() {
            let _ = std::fs::rename(&pending, self.outbox.dir().join(Outbox::batch_name("hang")));
        }
    }

    fn quit(&mut self, clean: bool) {
        self.state.session_open = !clean;
        self.save();
        if !self.may_send() {
            return;
        }
        self.summary();
        let event = Event::AppQuit(AppQuit {
            session_minutes: self.started.elapsed().as_secs() / 60,
            clean,
        });
        self.add(event, now_ms(), self.seen);
        self.flush_fold();
        // Kept first, so nothing is lost if the process ends mid-send.
        let batch = std::mem::take(&mut self.pending);
        if !batch.is_empty()
            && let Ok(file) = self.outbox.write(&Outbox::batch_name("quit"), &batch)
            && matches!(self.deliver(&batch), Delivery::Sent | Delivery::Dropped)
        {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// A development check (`PM_TELEMETRY_DEV`) sends only events marked
/// internal: the last check before anything leaves.
fn admissible(internal: bool, record: &Record) -> bool {
    !internal || record.properties.get("is_internal") == Some(&Value::Bool(true))
}

/// The recorded session a crash belongs to (same process id, started within
/// two seconds), if it was sending and never turned sharing off.
fn sending_session(sessions: &[prefs::Session], pid: u64, launched: f64) -> Option<prefs::Session> {
    sessions
        .iter()
        .find(|s| s.pid == pid && (s.started - launched).abs() < 2.)
        .filter(|s| s.consented && !s.revoked)
        .cloned()
}

fn hang_event(duration: &'static str) -> Event {
    Event::Hang(Hang { duration })
}

#[cfg(test)]
pub static TEST_HOST: OnceLock<String> = OnceLock::new();

/// PostHog, with one attempt per request: retries are the worker's, under
/// its consent checks.
fn client() -> Option<posthog::Client> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    #[cfg(test)]
    let host = TEST_HOST
        .get()
        .cloned()
        .unwrap_or_else(|| crate::HOST.to_owned());
    #[cfg(not(test))]
    let host = crate::HOST.to_owned();
    let options = posthog::ClientOptionsBuilder::default()
        .api_key(crate::PROJECT_TOKEN.to_owned())
        .host(host)
        .is_server(false)
        .disable_geoip(true)
        .request_timeout_seconds(10)
        .max_capture_attempts(1)
        .capture_compression(posthog::CaptureCompression::Gzip)
        .shutdown_timeout_ms(200)
        .build()
        .ok()?;
    Some(posthog::client(options))
}

/// Runs inside every panic before the previous hook, and never blocks: up
/// to 64 return addresses (no lock, no symbol lookup, no file) and the
/// panic's location if it is a repository path, offered to the worker once
/// without waiting. Never the panic's message.
fn install_panic_hook() {
    static IN_PANIC: AtomicBool = AtomicBool::new(false);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if taking() && !IN_PANIC.swap(true, Relaxed) {
            let mut addrs = [0u64; 64];
            let mut n = 0;
            // SAFETY: the unwinder walks this thread's own stack; no other
            // backtrace runs on it while it panics.
            unsafe {
                backtrace::trace_unsynchronized(|frame| {
                    if let Some(slot) = addrs.get_mut(n) {
                        *slot = frame.ip() as u64;
                        n += 1;
                    }
                    n < addrs.len()
                });
            }
            let location = info
                .location()
                .and_then(|l| exception::panic_file(l.file()).map(|f| (f, l.line())));
            send(Msg::Panic(
                addrs[..n].to_vec(),
                location,
                now_ms(),
                generation(),
            ));
            IN_PANIC.store(false, Relaxed);
        }
        previous(info);
    }));
}

/// Watches the main thread's heartbeat: blocked for more than 2 s while the
/// app is active, it tells the worker (which keeps a pending `hang`, sent at
/// the next launch if the app is ended while it hangs); when the main thread
/// answers, the hang is sent with its duration. No stack is taken: the main
/// thread is never suspended for telemetry.
fn watch_hangs() {
    let mut last = Instant::now();
    let mut began: Option<u64> = None;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let held = last.elapsed() > Duration::from_millis(1500);
        last = Instant::now();
        let late = crate::main_thread::since_beat_ms();
        match began {
            None => {
                // A held process (App Nap, a debugger) is not a hang.
                if !held
                    && late > HANG_MS
                    && taking()
                    && ACTIVE.load(Relaxed)
                    && crate::main_thread::installed()
                {
                    began = Some(crate::main_thread::now_ms().saturating_sub(late));
                    send(Msg::HangBegan(now_ms(), generation()));
                }
            }
            Some(start) if late < 1000 => {
                let seconds = crate::main_thread::now_ms().saturating_sub(start) as f64 / 1000.;
                send(Msg::HangEnded(hang_bucket(seconds), now_ms(), generation()));
                began = None;
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests;
