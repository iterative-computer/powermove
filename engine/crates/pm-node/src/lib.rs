//! napi-rs host for engine processes (plan section 19).
//!
//! The addon runs inside an Electron utility process (the interactive or
//! the job engine). The utility process's JavaScript
//! (`apps/desktop/src/main/engine/engine-process.ts`) receives
//! `MessagePortMain`s from main, hands each message's bytes to
//! [`EngineHost::receive`], and carries out the returned actions: post bytes
//! on a port, close a port, or log. Engine state lives in [`pm_engine::Engine`];
//! this crate only parses the strings JavaScript passes, sets the process's
//! scheduling class, and converts actions.

use pm_engine::{Action, Engine, EngineConfig, EngineStatus, PeerKind};
use pm_protocol::error::EngineProcess;
use pm_protocol::host::PortId;

// napi registers the exports; unit tests never call them.
#[cfg(feature = "node")]
#[cfg_attr(test, allow(dead_code))]
mod node;

#[cfg(target_os = "macos")]
pub mod present;

#[cfg(target_os = "macos")]
pub mod jobs;

#[cfg(target_os = "macos")]
mod memory;

pub mod fonts;

/// Parses `"interactive"` or `"job"`.
pub fn parse_process(text: &str) -> Result<EngineProcess, String> {
    match text {
        "interactive" => Ok(EngineProcess::Interactive),
        "job" => Ok(EngineProcess::Job),
        other => Err(format!("unknown engine process {other:?}")),
    }
}

/// The peer kind of main's font port to the interactive engine. It is not a
/// protocol peer: its messages go to [`fonts::FontService`], never to the
/// engine's protocol state machine.
pub const FONTS_PEER: &str = "fonts";

/// Parses `"host"`, `"renderer"`, or `"peer"`.
pub fn parse_peer(text: &str) -> Result<PeerKind, String> {
    match text {
        "host" => Ok(PeerKind::Host),
        "renderer" => Ok(PeerKind::Renderer),
        "peer" => Ok(PeerKind::PeerEngine),
        other => Err(format!("unknown peer kind {other:?}")),
    }
}

pub use pm_engine::qos::{apply_process_qos, current_qos};

/// Where actions produced off the JavaScript thread go (the viewer
/// presenter's frames): pm-node's threadsafe function posts them on the
/// utility process's ports.
pub type ActionSink = std::sync::Arc<dyn Fn(Vec<Action>) + Send + Sync>;

/// Set to `0` to start the job engine without its render and export service
/// (it then refuses jobs and agent frames, and the host routes them to the
/// current engine).
pub const JOBS_ENV: &str = "POWERMOVE_ENGINE_JOBS";

#[cfg(target_os = "macos")]
type PresenterStats = std::sync::Arc<std::sync::Mutex<present::Stats>>;
#[cfg(not(target_os = "macos"))]
type PresenterStats = ();

#[cfg(target_os = "macos")]
type JobStats = jobs::StatsHandle;
#[cfg(not(target_os = "macos"))]
type JobStats = ();

/// One engine process as its JavaScript host sees it.
pub struct EngineHost {
    engine: Engine,
    presenter: Option<PresenterStats>,
    #[cfg(target_os = "macos")]
    control: Option<present::PresenterControl>,
    jobs: Option<JobStats>,
    #[cfg(target_os = "macos")]
    memory_inventory: bool,
    /// The font service: created with the host when it runs on the
    /// process's main thread (AppKit's font manager), else when main first
    /// attaches a font port.
    fonts: Option<fonts::FontService>,
    font_ports: std::collections::BTreeSet<u32>,
    /// Render threads' requests to the font service (`fonts::hub`).
    font_hub: std::sync::Arc<fonts::hub::FontHub>,
    /// Where the engine's rendering threads ask the font service for
    /// registry answers and full fonts (`fonts::render`).
    broker: fonts::render::FontBroker,
}

impl EngineHost {
    pub fn new(process: EngineProcess) -> Result<Self, String> {
        Self::with_sink(process, None, |key| std::env::var(key).ok())
    }

    /// An engine whose asynchronous actions go to `sink`. The interactive
    /// engine installs the frame-bridge presenter when main passed it a
    /// bridge name (`POWERMOVE_FRAME_BRIDGE`) and a sink exists.
    pub fn with_sink(
        process: EngineProcess,
        sink: Option<ActionSink>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        apply_process_qos(process)?;
        #[cfg(target_os = "macos")]
        let memory_inventory = env("POWERMOVE_ENGINE_ALLOCATION_INVENTORY").as_deref() == Some("1");
        #[allow(unused_mut)]
        let mut engine = Engine::new(EngineConfig::new(process));
        // Render threads lay text out over the font service (both engines:
        // the viewer, and the job engine's exports and agent frames). Their
        // requests wake this thread through the sink, which calls `pump`.
        let font_hub = std::sync::Arc::new(fonts::hub::FontHub::new());
        if let Some(sink) = sink.clone() {
            font_hub.set_wake(std::sync::Arc::new(move || sink(Vec::new())));
        }
        // On the process's main thread the registry can start now. Elsewhere
        // (unit tests, platforms without a registry) it can start only on a
        // font port, and render threads get no fonts: their text and
        // captions layers stay unsupported, as without a font service.
        let font_service = fonts::FontService::new().ok();
        let render_fonts = font_service.as_ref().map(|_| font_hub.clone());
        #[allow(unused_mut)]
        let mut presenter = None;
        #[allow(unused_mut)]
        let mut job_stats = None;
        #[cfg(target_os = "macos")]
        let mut control = None;
        let broker = fonts::render::FontBroker::new();
        if let Some(sink) = sink.clone() {
            // An empty action list makes the utility process call `pump`,
            // which answers the asks on this (the main) thread.
            broker.set_wake(std::sync::Arc::new(move || sink(Vec::new())));
        }
        #[cfg(target_os = "macos")]
        {
            if let (Some(sink), Some(mut config)) = (
                sink.clone(),
                present::PresenterConfig::from_env(process, &env),
            ) {
                config.thread.fonts = render_fonts
                    .clone()
                    .map(|hub| fonts::render::RenderFonts::with_hub(broker.clone(), hub));
                let bridge = present::BridgePresenter::start(config, sink)?;
                presenter = Some(bridge.stats_handle());
                control = Some(bridge.control());
                engine.set_presenter(Box::new(bridge));
            }
            // The job engine renders exports and agent frames (sections 17
            // and 20). The interactive engine needs only CPU media facts;
            // its probe worker starts on first use. Both wake the utility
            // process through the sink, which then calls `pump`.
            if let Some(sink) = sink {
                let wake: jobs::Wake = std::sync::Arc::new(move || sink(Vec::new()));
                if process == EngineProcess::Interactive {
                    engine
                        .set_job_service(Box::new(jobs::JobEngineService::start_probes_only(wake)));
                } else if env(JOBS_ENV).as_deref() != Some("0") {
                    let service =
                        jobs::JobEngineService::start_with_fonts(wake, render_fonts.clone())?;
                    job_stats = Some(service.stats_handle());
                    engine.set_job_service(Box::new(service));
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (sink, env, render_fonts);
        Ok(Self {
            engine,
            presenter,
            #[cfg(target_os = "macos")]
            control,
            jobs: job_stats,
            #[cfg(target_os = "macos")]
            memory_inventory,
            fonts: font_service,
            font_ports: Default::default(),
            font_hub,
            broker,
        })
    }

    /// Answers the rendering threads' font asks (on this thread, which
    /// AppKit's font manager needs). The service is created on the first ask
    /// when main has not attached a font port yet; main's port then opens
    /// it at its generation base and replays the project fonts.
    fn serve_broker_fonts(&mut self) {
        let asks = self.broker.take_asks();
        if asks.is_empty() {
            return;
        }
        if self.fonts.is_none() {
            match fonts::FontService::new() {
                Ok(service) => self.fonts = Some(service),
                Err(error) => {
                    for ask in asks {
                        ask.answer(Err(format!(
                            "the engine's font registry is unavailable: {error}"
                        )));
                    }
                    return;
                }
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(service) = self.fonts.as_mut() {
            self.broker.set_generation(service.generation());
            fonts::render::serve(service, asks);
        }
        #[cfg(not(target_os = "macos"))]
        for ask in asks {
            ask.answer(Err("the native font registry is macOS only".into()));
        }
    }

    /// The font service's generation moved (a project font, a variable
    /// source, a reopened port): rendering threads install fresh fonts and
    /// the viewer draws again.
    fn fonts_changed(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(service) = self.fonts.as_ref() {
            let generation = service.generation();
            if generation != self.broker.generation() {
                self.broker.set_generation(generation);
                if let Some(control) = &self.control {
                    control.fonts_changed();
                }
            }
        }
    }

    /// The broker rendering threads ask (tests and the job engine).
    pub fn font_broker(&self) -> &fonts::render::FontBroker {
        &self.broker
    }

    /// Simulates a GPU device loss in the viewer presenter (the recovery
    /// test): the device is destroyed as a real loss would leave it, and
    /// the presenter rebuilds it, makes every pool again, and redraws.
    /// False when this engine does not present.
    pub fn simulate_gpu_loss(&self) -> bool {
        #[cfg(target_os = "macos")]
        if let Some(control) = &self.control {
            control.lose_device();
            return true;
        }
        false
    }

    /// The presenter's per-frame trace as JSON (S1 harness), emptied.
    pub fn take_present_trace(&self) -> String {
        #[cfg(target_os = "macos")]
        if let Some(stats) = &self.presenter {
            let trace = std::mem::take(&mut stats.lock().unwrap_or_else(|e| e.into_inner()).trace);
            return serde_json::to_string(&trace).unwrap_or_else(|_| "[]".into());
        }
        "[]".into()
    }

    /// A port main connected. `owner` names the window a renderer client's
    /// port belongs to (its layouts measure with that window's font cache
    /// seeds).
    pub fn attach(
        &mut self,
        port: u32,
        peer: &str,
        owner: Option<u32>,
    ) -> Result<Vec<Action>, String> {
        if peer == FONTS_PEER {
            return self.attach_fonts(port);
        }
        let actions = self.engine.attach(PortId(port), parse_peer(peer)?);
        self.font_hub.set_port_owner(port, owner);
        Ok(actions)
    }

    /// Main's font port. Both engines host the registry: the interactive
    /// engine serves pm-wasm (through main) and its viewer, the job engine
    /// its exports and agent frames; main replays project fonts and seeds to
    /// each. The service is kept for the launch, so a reconnecting main keeps
    /// its project fonts and face ids.
    fn attach_fonts(&mut self, port: u32) -> Result<Vec<Action>, String> {
        if self.fonts.is_none() {
            self.fonts = Some(fonts::FontService::new()?);
        }
        self.font_ports.insert(port);
        self.fonts_changed();
        Ok(Vec::new())
    }

    /// Answers render threads' font requests (on this thread).
    fn serve_fonts(&mut self) {
        self.serve_broker_fonts();
        if !self.font_hub.has_requests() {
            return;
        }
        let service = &mut self.fonts;
        self.font_hub.drain(|queries| match service.as_mut() {
            Some(service) => service.serve(queries),
            None => Err("this engine has no font registry".into()),
        });
    }

    /// The render threads' font hub (tests and status).
    pub fn font_hub(&self) -> &std::sync::Arc<fonts::hub::FontHub> {
        &self.font_hub
    }

    pub fn detach(&mut self, port: u32) -> Vec<Action> {
        self.sync_runtime_qos();
        if self.font_ports.remove(&port) {
            return Vec::new();
        }
        self.font_hub.set_port_owner(port, None);
        let actions = self.engine.detach(PortId(port));
        self.sync_runtime_qos();
        actions
    }

    pub fn receive(&mut self, port: u32, bytes: &[u8]) -> Vec<Action> {
        self.sync_runtime_qos();
        if self.font_ports.contains(&port) {
            let generation = pm_text::served::FontServer::generation(&*self.font_hub);
            let reply = match self.fonts.as_mut() {
                Some(service) => {
                    let reply = service.handle(bytes);
                    // What render threads lay out with follows the service:
                    // its generation (project fonts, a new generation base)
                    // and main's seeds; the open lets their requests go out.
                    self.font_hub.note_registry_generation(service.generation());
                    self.font_hub.set_seeds(service.seeds().clone());
                    if service.opened() {
                        self.font_hub.opened();
                    }
                    reply
                }
                None => return Vec::new(),
            };
            self.fonts_changed();
            #[cfg(target_os = "macos")]
            if generation != pm_text::served::FontServer::generation(&*self.font_hub)
                && let Some(control) = &self.control
            {
                control.fonts_changed();
            }
            #[cfg(not(target_os = "macos"))]
            let _ = generation;
            self.serve_fonts();
            return vec![Action::Send {
                port: PortId(port),
                bytes: reply,
            }];
        }
        let actions = self
            .engine
            .receive(PortId(port), bytes, pm_engine::clock::host_now());
        self.sync_runtime_qos();
        actions
    }

    /// The font service, once main has attached a font port: the engine's
    /// own text rendering lays text out and reads outlines through it.
    pub fn fonts(&mut self) -> Option<&mut fonts::FontService> {
        self.fonts.as_mut()
    }

    pub fn tick(&mut self) -> Vec<Action> {
        self.sync_runtime_qos();
        self.serve_fonts();
        let actions = self.engine.tick(pm_engine::clock::host_now());
        self.sync_runtime_qos();
        actions
    }

    /// What the job service produced off this thread (`Engine::pump`), and
    /// the rendering threads' font asks.
    pub fn pump(&mut self) -> Vec<Action> {
        self.sync_runtime_qos();
        self.serve_fonts();
        let actions = self.engine.pump();
        self.sync_runtime_qos();
        actions
    }

    #[cfg(target_os = "macos")]
    fn sync_runtime_qos(&self) {
        if let Some(jobs) = &self.jobs {
            let desired = jobs.runtime_qos();
            let name = match desired {
                pm_engine::qos::JobQos::Default => "default",
                pm_engine::qos::JobQos::Utility => "utility",
            };
            if current_qos() != name {
                pm_engine::qos::set_job_qos(desired).expect("valid Darwin job QoS class");
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn sync_runtime_qos(&self) {}

    pub fn status(&self) -> EngineStatus {
        self.engine.status()
    }

    /// Status as JSON, with the thread's QoS class and the presenter's
    /// counters added.
    pub fn status_json(&self) -> String {
        self.sync_runtime_qos();
        let mut value = serde_json::to_value(self.status()).unwrap_or_default();
        if let Some(object) = value.as_object_mut() {
            #[cfg(target_os = "macos")]
            if self.memory_inventory {
                object.insert(
                    "mallocMemory".into(),
                    serde_json::to_value(memory::sample()).unwrap_or_default(),
                );
            }
            object.insert("qos".into(), current_qos().into());
            object.insert("fontPorts".into(), self.font_ports.len().into());
            object.insert(
                "fonts".into(),
                serde_json::to_value(self.font_hub.stats()).unwrap_or_default(),
            );
            #[cfg(target_os = "macos")]
            object.insert(
                "fontMemory".into(),
                self.fonts
                    .as_ref()
                    .and_then(|service| serde_json::to_value(service.memory_usage()).ok())
                    .unwrap_or_default(),
            );
            let (asked, answered, failed) = self.broker.counts();
            object.insert(
                "renderFonts".into(),
                serde_json::json!({ "asked": asked, "answered": answered, "failed": failed, "generation": self.broker.generation() }),
            );
            #[cfg(target_os = "macos")]
            if let Some(jobs) = &self.jobs {
                object.insert("jobs".into(), jobs.json());
            }
            #[cfg(target_os = "macos")]
            if let Some(stats) = &self.presenter {
                let stats = stats.lock().unwrap_or_else(|e| e.into_inner());
                object.insert(
                    "presenter".into(),
                    serde_json::to_value(&*stats).unwrap_or_default(),
                );
            }
        }
        value.to_string()
    }

    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }
}

impl Drop for EngineHost {
    /// Render threads waiting for fonts stop waiting before the presenter
    /// and the job lanes are joined (the engine drops after this).
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if self.jobs.is_some() {
            pm_engine::qos::set_job_qos(pm_engine::qos::JobQos::Utility)
                .expect("valid Darwin job QoS class");
        }
        self.broker.close();
        self.font_hub.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_engine::engine::testing::{hello, sent};
    use pm_protocol::envelope::Message;
    use pm_protocol::version::PeerRole;

    #[test]
    fn strings_from_javascript_parse_strictly() {
        assert_eq!(parse_process("job"), Ok(EngineProcess::Job));
        assert!(parse_process("Job").is_err());
        assert_eq!(parse_peer("peer"), Ok(PeerKind::PeerEngine));
        assert!(parse_peer("main").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn malloc_attribution_is_opt_in_and_reports_process_zones() {
        let ordinary = EngineHost::with_sink(EngineProcess::Interactive, None, |_| None).unwrap();
        let ordinary: serde_json::Value = serde_json::from_str(&ordinary.status_json()).unwrap();
        assert!(ordinary.get("mallocMemory").is_none());
        let measured = EngineHost::with_sink(EngineProcess::Interactive, None, |key| {
            (key == "POWERMOVE_ENGINE_ALLOCATION_INVENTORY").then(|| "1".into())
        })
        .unwrap();
        let measured: serde_json::Value = serde_json::from_str(&measured.status_json()).unwrap();
        let malloc = &measured["mallocMemory"];
        assert!(malloc["blocksInUse"].as_u64().unwrap() > 0);
        assert!(malloc["sizeInUseBytes"].as_u64().unwrap() > 0);
        assert!(
            malloc["reservedBytes"].as_u64().unwrap() >= malloc["sizeInUseBytes"].as_u64().unwrap()
        );
        assert_eq!(malloc.as_object().unwrap().len(), 4);
    }

    #[test]
    fn the_job_engine_runs_at_utility_qos() {
        // Run on a fresh thread so the test runner's thread keeps its class.
        let qos = std::thread::spawn(|| {
            let mut host = EngineHost::new(EngineProcess::Job).unwrap();
            host.attach(1, "host", None).unwrap();
            let out = host.receive(1, &hello(PeerRole::Host));
            assert!(matches!(
                sent(&out, PortId(1))[0].message,
                Message::Welcome(_)
            ));
            let status: serde_json::Value = serde_json::from_str(&host.status_json()).unwrap();
            assert_eq!(status["process"], "job");
            status["qos"].as_str().unwrap().to_owned()
        })
        .join()
        .unwrap();
        if cfg!(target_vendor = "apple") {
            assert_eq!(qos, "utility");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shared_job_runtime_follows_only_running_user_exports() {
        use pm_protocol::priority::{JobState, PriorityClass};

        std::thread::spawn(|| {
            let sink: ActionSink = std::sync::Arc::new(|_| {});
            let mut host = EngineHost::with_sink(EngineProcess::Job, Some(sink), |_| None).unwrap();
            let jobs = host.jobs.clone().expect("job service started");
            assert_eq!(current_qos(), "utility");

            jobs.test_set_job("agent", PriorityClass::AgentExport, JobState::Running);
            jobs.test_set_job("user", PriorityClass::UserJob, JobState::Queued);
            host.pump();
            assert_eq!(current_qos(), "utility");

            jobs.test_set_job("user", PriorityClass::UserJob, JobState::Running);
            host.pump();
            assert_eq!(current_qos(), "default");
            let worker = pm_engine::qos::spawn_engine_thread(
                EngineProcess::Job,
                "pm-test-agent-qos",
                current_qos,
            )
            .unwrap();
            assert_eq!(worker.join().unwrap(), "utility");

            jobs.test_set_playback(true);
            host.tick();
            assert_eq!(current_qos(), "utility");
            jobs.test_set_playback(false);
            host.pump();
            assert_eq!(current_qos(), "default");

            jobs.test_set_job("second", PriorityClass::UserJob, JobState::Running);
            jobs.test_set_job("user", PriorityClass::UserJob, JobState::Done);
            host.pump();
            assert_eq!(current_qos(), "default");
            jobs.test_set_job("second", PriorityClass::UserJob, JobState::Cancelled);
            host.pump();
            assert_eq!(current_qos(), "utility");

            jobs.test_set_job("user", PriorityClass::UserJob, JobState::Running);
            host.pump();
            assert_eq!(current_qos(), "default");
            drop(host);
            assert_eq!(current_qos(), "utility");
        })
        .join()
        .unwrap();
    }
}
