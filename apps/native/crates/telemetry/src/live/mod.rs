//! The running telemetry: a bounded channel the app's threads `try_send`
//! into, one thread that batches and sends, the panic hook and the hang
//! watchdog.
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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Events waiting for the telemetry thread before new ones are dropped.
const QUEUE: usize = 2048;
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

enum Msg {
    Event(Event, i64),
    Launched {
        first_window_ms: u64,
        interactive_ms: u64,
        first_run: bool,
        at: i64,
    },
    Identify(String),
    Sharing(bool),
    Record(Box<Record>),
    Flush(SyncSender<()>),
    Quit(bool, SyncSender<()>),
}

/// What the panic hook and the watchdog need without the thread.
struct Shared {
    tx: SyncSender<Msg>,
    distinct_id: String,
    supers: Map<String, Value>,
    outbox: Outbox,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
/// Running and shared: the one check every capture makes.
static ON: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(true);
static DROPPED: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn on() -> bool {
    ON.load(Relaxed)
}

#[cfg(test)]
pub fn force_on_for_tests() {
    ON.store(true, Relaxed);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

pub fn start(config: Config, mut state: prefs::State, mode: Mode) {
    let (tx, rx) = sync_channel(QUEUE);
    let distinct_id = state
        .install_id
        .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
        .clone();
    let (channel, internal) = match mode {
        Mode::Production { channel } => (channel, false),
        _ => ("dev", true),
    };
    let (language, region) = system::locale();
    let supers = SuperProperties {
        app_version: config.app_version.clone(),
        build: config.build.clone(),
        channel,
        macos_version: system::macos_version(),
        chip: system::chip(),
        ram_bucket: ram_bucket(system::ram_bytes()),
        display_count: config.displays.0,
        display_max_scale: config.displays.1,
        locale: language,
        region,
        is_internal: internal,
        session_id: uuid::Uuid::now_v7().to_string(),
    };
    let supers = match serde_json::to_value(&supers) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    };
    let shared = Shared {
        tx,
        distinct_id,
        supers,
        outbox: Outbox::new(&config.profile),
    };
    if SHARED.set(shared).is_err() {
        return;
    }
    ON.store(state.sharing, Relaxed);
    install_panic_hook();
    let bundle_id = config.bundle_id.clone().unwrap_or_default();
    let spawned = std::thread::Builder::new()
        .name("pm-telemetry".into())
        .spawn(move || Worker::new(config, state, bundle_id).run(rx));
    if spawned.is_err() {
        ON.store(false, Relaxed);
        return;
    }
    let _ = std::thread::Builder::new()
        .name("pm-telemetry-hang".into())
        .spawn(watch_hangs);
}

#[inline]
pub fn capture(event: Event) {
    if !on() {
        return;
    }
    if let Some(shared) = SHARED.get()
        && shared.tx.try_send(Msg::Event(event, now_ms())).is_err()
    {
        DROPPED.fetch_add(1, Relaxed);
    }
}

pub fn launched(first_window_ms: u64, interactive_ms: u64, first_run: bool) {
    if let Some(shared) = SHARED.get()
        && on()
    {
        let _ = shared.tx.try_send(Msg::Launched {
            first_window_ms,
            interactive_ms,
            first_run,
            at: now_ms(),
        });
    }
}

pub fn since_process_start_ms() -> u64 {
    system::process_start().map_or(0, |start| {
        ((now_ms() as f64 / 1000. - start).max(0.) * 1000.) as u64
    })
}

pub fn identify(account_id: &str) {
    if let (Some(shared), Some(id)) = (SHARED.get(), clean_id(&account_id.to_ascii_lowercase()))
        && on()
    {
        let _ = shared.tx.try_send(Msg::Identify(id));
    }
}

pub fn set_active(active: bool) {
    ACTIVE.store(active, Relaxed);
}

/// Whether a telemetry thread took the change (it saves the choice).
pub fn set_sharing(on: bool) -> bool {
    let Some(shared) = SHARED.get() else {
        return false;
    };
    // Captures stop (or resume) now; the thread clears what is waiting.
    ON.store(on, Relaxed);
    if let Err(std::sync::mpsc::TrySendError::Full(msg)) = shared.tx.try_send(Msg::Sharing(on)) {
        let tx = shared.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(msg);
        });
    }
    true
}

pub fn quit(clean: bool) {
    let Some(shared) = SHARED.get() else {
        return;
    };
    let (ack, done) = sync_channel(1);
    if shared.tx.try_send(Msg::Quit(clean, ack)).is_ok() {
        let _ = done.recv_timeout(QUIT_WAIT);
    }
}

fn record(name: &str, mut properties: Map<String, Value>, timestamp_ms: i64) -> Option<Record> {
    let shared = SHARED.get()?;
    for (key, value) in &shared.supers {
        properties
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    Some(Record {
        uuid: uuid::Uuid::now_v7().to_string(),
        event: name.to_owned(),
        distinct_id: shared.distinct_id.clone(),
        timestamp_ms,
        properties,
    })
}

fn event_record(event: &Event, timestamp_ms: i64) -> Option<Record> {
    let properties = match serde_json::to_value(event)
        .ok()?
        .get_mut("properties")?
        .take()
    {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    record(event.name(), properties, timestamp_ms)
}

enum Delivery {
    Sent,
    /// Offline or PostHog busy: keep it for later.
    Retry,
    /// Refused for good (a malformed batch): drop it.
    Refused,
}

struct Worker {
    profile: PathBuf,
    state: prefs::State,
    bundle_id: String,
    metal: Option<Arc<dyn Fn() -> u64 + Send + Sync>>,
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
}

impl Worker {
    fn new(config: Config, state: prefs::State, bundle_id: String) -> Worker {
        let now = Instant::now();
        Worker {
            profile: config.profile,
            state,
            bundle_id,
            metal: config.metal_bytes,
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
        }
    }

    fn outbox(&self) -> &'static Outbox {
        &SHARED.get().expect("started").outbox
    }

    fn save(&self) {
        let _ = prefs::store(&self.profile, &self.state);
    }

    fn run(mut self, rx: Receiver<Msg>) {
        self.crash_last_session = self.state.session_open;
        let boot = system::boot_time();
        self.cold = boot.is_some() && self.state.last_boot != boot;
        self.state.last_boot = boot;
        self.state.session_open = true;
        self.state.install_id = Some(SHARED.get().expect("started").distinct_id.clone());
        self.save();
        if self.state.sharing {
            self.client = client();
            self.read_crash_reports();
        }
        loop {
            match rx.recv_timeout(TICK) {
                Ok(Msg::Event(event, at)) => self.add(event, at),
                Ok(Msg::Launched {
                    first_window_ms,
                    interactive_ms,
                    first_run,
                    at,
                }) => {
                    let event = Event::AppLaunched(AppLaunched {
                        kind: if self.cold {
                            LaunchKind::Cold
                        } else {
                            LaunchKind::Warm
                        },
                        launch_to_first_window_ms: first_window_ms,
                        launch_to_interactive_ms: interactive_ms,
                        first_run,
                        crash_last_session: self.crash_last_session,
                    });
                    self.add(event, at);
                }
                Ok(Msg::Identify(id)) => self.identify(id),
                Ok(Msg::Sharing(on)) => self.sharing(on),
                Ok(Msg::Record(record)) => {
                    self.pending.push(*record);
                    self.next_send = Instant::now();
                }
                Ok(Msg::Flush(ack)) => {
                    self.send();
                    let _ = ack.send(());
                }
                Ok(Msg::Quit(clean, ack)) => {
                    self.quit(clean);
                    let _ = ack.send(());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.tick();
        }
    }

    fn add(&mut self, event: Event, at: i64) {
        if !on() {
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
        if let Some(record) = event_record(&event, at) {
            self.pending.push(record);
        }
    }

    fn flush_fold(&mut self) {
        if let Some((command, at, _)) = self.folding.take()
            && let Some(record) = event_record(&Event::CommandInvoked(command), at)
        {
            self.pending.push(record);
        }
    }

    fn identify(&mut self, id: String) {
        if self.state.identified.as_ref() == Some(&id) {
            return;
        }
        let shared = SHARED.get().expect("started");
        let mut props = Map::new();
        props.insert(
            "$anon_distinct_id".into(),
            shared.distinct_id.clone().into(),
        );
        if let Some(mut record) = record("$identify", props, now_ms()) {
            record.distinct_id = id.clone();
            self.pending.push(record);
            self.next_send = Instant::now();
        }
        self.state.identified = Some(id);
        self.save();
    }

    fn sharing(&mut self, on: bool) {
        self.state.sharing = on;
        if on {
            self.client = self.client.take().or_else(client);
            self.add(
                Event::SettingsChanged(SettingsChanged::Telemetry(true)),
                now_ms(),
            );
            self.next_send = Instant::now();
        } else {
            // Nothing that was waiting leaves; crashes from now on stay.
            self.pending.clear();
            self.folding = None;
            let _ = perf::take();
            self.peaks = perf::Peaks::default();
            self.outbox().clear();
            self.client = None;
            self.state.last_crash_report = Some(now_ms() as f64 / 1000.);
        }
        self.save();
    }

    fn tick(&mut self) {
        if !on() {
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
        if (due && !self.pending.is_empty())
            || (self.pending.len() >= SEND_AT && self.failures == 0)
        {
            self.send();
        } else if due {
            // Nothing new: still try what an earlier session left.
            self.next_send = Instant::now() + SEND_EVERY;
            if self.failures == 0 {
                self.drain_outbox();
            }
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
            playback_fps_target: playback.map(|p| p.1),
            peak_footprint_mb: peaks.footprint >> 20,
            peak_metal_mb: peaks.metal.map(|m| m >> 20),
            thermal_state_max: perf::thermal_name(peaks.thermal),
            on_battery: peaks.on_battery,
            telemetry_dropped: DROPPED.swap(0, Relaxed),
        }));
        self.add(event, now_ms());
    }

    fn deliver(&self, records: &[Record]) -> Delivery {
        let Some(client) = &self.client else {
            return Delivery::Retry;
        };
        let events = records.iter().map(Record::to_posthog).collect();
        match client.capture_batch_immediate(events, false) {
            Ok(_) => Delivery::Sent,
            Err(
                posthog::Error::BadRequest(_)
                | posthog::Error::Serialization(_)
                | posthog::Error::InvalidArgument(_),
            ) => Delivery::Refused,
            Err(_) => Delivery::Retry,
        }
    }

    fn send(&mut self) {
        self.flush_fold();
        if self.client.is_none() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        let ok = batch.is_empty()
            || match self.deliver(&batch) {
                Delivery::Sent | Delivery::Refused => true,
                Delivery::Retry => {
                    let _ = self.outbox().write(&Outbox::batch_name("batch"), &batch);
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
    /// takes them.
    fn drain_outbox(&mut self) {
        if self.client.is_none() {
            return;
        }
        for file in self.outbox().pending(false).into_iter().take(10) {
            let records = Outbox::read(&file);
            match self.deliver(&records) {
                Delivery::Sent | Delivery::Refused => {
                    let _ = std::fs::remove_file(&file);
                }
                Delivery::Retry => {
                    self.failures += 1;
                    return;
                }
            }
        }
    }

    fn read_crash_reports(&mut self) {
        let Some(folder) = crash::reports_folder() else {
            return;
        };
        // The first session looks back a day.
        let since = self
            .state
            .last_crash_report
            .unwrap_or(now_ms() as f64 / 1000. - 86_400.);
        let mut newest = since;
        for found in crash::new_reports(&folder, &self.bundle_id, since) {
            if let Some(record) =
                record("$exception", found.properties, (found.time * 1000.) as i64)
            {
                self.pending.push(record);
            }
            newest = newest.max(found.time);
        }
        self.state.last_crash_report = Some(newest);
        self.save();
        // A hang the last session never recovered from is in the outbox.
        if self.outbox().dir().join(outbox::HANG_PENDING).exists() {
            let from = self.outbox().dir().join(outbox::HANG_PENDING);
            let _ = std::fs::rename(&from, self.outbox().dir().join(Outbox::batch_name("hang")));
        }
        self.next_send = Instant::now();
    }

    fn quit(&mut self, clean: bool) {
        if on() {
            self.summary();
            let event = Event::AppQuit(AppQuit {
                session_minutes: self.started.elapsed().as_secs() / 60,
                clean,
            });
            self.add(event, now_ms());
            self.flush_fold();
            // Kept first, so nothing is lost if the process ends mid-send.
            let batch = std::mem::take(&mut self.pending);
            if !batch.is_empty()
                && let Ok(file) = self.outbox().write(&Outbox::batch_name("quit"), &batch)
            {
                match self.deliver(&batch) {
                    Delivery::Sent | Delivery::Refused => {
                        let _ = std::fs::remove_file(file);
                    }
                    Delivery::Retry => {}
                }
            }
        }
        self.state.session_open = !clean;
        self.save();
    }
}

/// PostHog, or with `PM_TELEMETRY_DEV=1` the `PM_TELEMETRY_HOST` a check
/// points it at (a closed port for the offline check, a local listener).
fn host() -> String {
    let dev = std::env::var("PM_TELEMETRY_DEV").as_deref() == Ok("1");
    match std::env::var("PM_TELEMETRY_HOST") {
        Ok(host) if dev && !host.is_empty() => host,
        _ => crate::HOST.to_owned(),
    }
}

fn client() -> Option<posthog::Client> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let options = posthog::ClientOptionsBuilder::default()
        .api_key(crate::PROJECT_TOKEN.to_owned())
        .host(host())
        .is_server(false)
        .disable_geoip(true)
        .request_timeout_seconds(10)
        .max_capture_attempts(3)
        .retry_initial_backoff_ms(250)
        .retry_max_backoff_ms(2000)
        .capture_compression(posthog::CaptureCompression::Gzip)
        .shutdown_timeout_ms(200)
        .build()
        .ok()?;
    Some(posthog::client(options))
}

/// Runs inside every panic before the previous hook: the exception is
/// written to the outbox by the panicking thread itself (the app may end
/// right after), then the telemetry thread is asked to send it.
fn install_panic_hook() {
    static IN_PANIC: AtomicBool = AtomicBool::new(false);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if on() && !IN_PANIC.swap(true, Relaxed) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| report_panic(info)));
            IN_PANIC.store(false, Relaxed);
        }
        previous(info);
    }));
}

#[allow(deprecated)]
fn report_panic(info: &std::panic::PanicInfo<'_>) {
    let Some(shared) = SHARED.get() else {
        return;
    };
    let mut addrs = vec![];
    backtrace::trace(|frame| {
        addrs.push(frame.ip() as u64);
        addrs.len() < 128
    });
    let mut frames = exception::resolve(&addrs);
    // Our own hook plumbing sits inside the panic dispatcher: drop it.
    if let Some(dispatcher) = frames.iter().take(24).position(|f| {
        f.function
            .as_deref()
            .is_some_and(|n| n.contains("rust_panic_with_hook") || n.contains("panic_with_hook"))
    }) {
        frames.drain(..dispatcher);
    }
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into());
    let mut props = exception::properties(
        exception::Kind {
            exception_type: "Panic",
            message: &message,
            mechanism: "panic",
            level: "fatal",
        },
        &frames,
        &exception::loaded_images(),
    );
    if let Some(location) = info.location() {
        if let Some(file) = exception::scrub_source_path(location.file()) {
            props.insert("$exception_panic_file".into(), file.into());
        }
        props.insert("$exception_panic_line".into(), location.line().into());
    }
    let thread = std::thread::current();
    if let Some(name) = thread
        .name()
        .and_then(|n| clean_id(&n.to_ascii_lowercase()))
    {
        props.insert("thread".into(), name.into());
    }
    let Some(record) = record("$exception", props, now_ms()) else {
        return;
    };
    if shared
        .outbox
        .write(&Outbox::batch_name("panic"), &[record])
        .is_err()
    {
        return;
    }
    if thread.name() != Some("pm-telemetry") {
        let (ack, done) = sync_channel(1);
        if shared.tx.try_send(Msg::Flush(ack)).is_ok() {
            let _ = done.recv_timeout(Duration::from_secs(2));
        }
    }
}

/// Watches the main thread's heartbeat: blocked for more than 2 s while the
/// app is active, it samples the main thread's stack once and keeps a pending
/// `hang` in the outbox (sent at the next launch if the app is ended while it
/// hangs); when the main thread answers, the pending one is replaced by the
/// event with its duration.
fn watch_hangs() {
    let images = exception::loaded_images();
    let mut last = Instant::now();
    let mut hang: Option<(u64, Record)> = None;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let held = last.elapsed() > Duration::from_millis(1500);
        last = Instant::now();
        let late = crate::main_thread::since_beat_ms();
        match hang.take() {
            None => {
                // A held process (App Nap, a debugger) is not a hang.
                if held
                    || late <= HANG_MS
                    || !on()
                    || !ACTIVE.load(Relaxed)
                    || !crate::main_thread::installed()
                {
                    continue;
                }
                let mut buf = [0u64; crate::main_thread::DEPTH];
                let n = crate::main_thread::sample(&mut buf);
                let stack = stack_lines(&buf[..n], &images);
                let event = Event::Hang(Hang {
                    duration: "unrecovered",
                    stack,
                });
                if let Some(record) = event_record(&event, now_ms()) {
                    if let Some(shared) = SHARED.get() {
                        let _ = shared
                            .outbox
                            .write(outbox::HANG_PENDING, std::slice::from_ref(&record));
                    }
                    hang = Some((crate::main_thread::now_ms().saturating_sub(late), record));
                }
            }
            Some((began, mut record)) if late < 1000 => {
                let seconds = crate::main_thread::now_ms().saturating_sub(began) as f64 / 1000.;
                record
                    .properties
                    .insert("duration".into(), hang_bucket(seconds).into());
                if let Some(shared) = SHARED.get() {
                    let _ = std::fs::remove_file(shared.outbox.dir().join(outbox::HANG_PENDING));
                    let _ = shared.tx.try_send(Msg::Record(Box::new(record)));
                }
            }
            still => hang = still,
        }
    }
}

/// `binary+0xoffset symbol`, innermost first.
fn stack_lines(addrs: &[u64], images: &[exception::Image]) -> Vec<String> {
    exception::resolve(addrs)
        .into_iter()
        .map(|frame| {
            let image = images
                .iter()
                .find(|i| frame.addr >= i.base && frame.addr < i.base + i.size.max(1));
            let place = match image {
                Some(i) => format!("{}+{:#x}", i.name, frame.addr - i.base),
                None => format!("{:#x}", frame.addr),
            };
            match frame.function {
                Some(f) => format!("{place} {}", exception::normalize(&f)),
                None => place,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
