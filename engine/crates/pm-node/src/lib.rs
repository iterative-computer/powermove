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

#[cfg(target_os = "macos")]
type PresenterStats = std::sync::Arc<std::sync::Mutex<present::Stats>>;
#[cfg(not(target_os = "macos"))]
type PresenterStats = ();

/// One engine process as its JavaScript host sees it.
pub struct EngineHost {
    engine: Engine,
    presenter: Option<PresenterStats>,
    #[cfg(target_os = "macos")]
    control: Option<present::PresenterControl>,
    process: EngineProcess,
    /// The font service, created when main first attaches a font port.
    fonts: Option<fonts::FontService>,
    font_ports: std::collections::BTreeSet<u32>,
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
        #[allow(unused_mut)]
        let mut engine = Engine::new(EngineConfig::new(process));
        #[allow(unused_mut)]
        let mut presenter = None;
        #[cfg(target_os = "macos")]
        let mut control = None;
        #[cfg(target_os = "macos")]
        if let (Some(sink), Some(config)) = (sink, present::PresenterConfig::from_env(process, env))
        {
            let bridge = present::BridgePresenter::start(config, sink)?;
            presenter = Some(bridge.stats_handle());
            control = Some(bridge.control());
            engine.set_presenter(Box::new(bridge));
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (sink, env);
        Ok(Self {
            engine,
            presenter,
            #[cfg(target_os = "macos")]
            control,
            process,
            fonts: None,
            font_ports: Default::default(),
        })
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

    pub fn attach(&mut self, port: u32, peer: &str) -> Result<Vec<Action>, String> {
        if peer == FONTS_PEER {
            return self.attach_fonts(port);
        }
        Ok(self.engine.attach(PortId(port), parse_peer(peer)?))
    }

    /// Main's font port: only the interactive engine hosts the registry.
    /// The service is created on the first attach (on this thread, the
    /// process's main thread) and kept for the launch, so a reconnecting
    /// main keeps its project fonts and face ids.
    fn attach_fonts(&mut self, port: u32) -> Result<Vec<Action>, String> {
        if self.process != EngineProcess::Interactive {
            return Err("only the interactive engine serves fonts".into());
        }
        if self.fonts.is_none() {
            self.fonts = Some(fonts::FontService::new()?);
        }
        self.font_ports.insert(port);
        Ok(Vec::new())
    }

    pub fn detach(&mut self, port: u32) -> Vec<Action> {
        if self.font_ports.remove(&port) {
            return Vec::new();
        }
        self.engine.detach(PortId(port))
    }

    pub fn receive(&mut self, port: u32, bytes: &[u8]) -> Vec<Action> {
        if self.font_ports.contains(&port) {
            let reply = match self.fonts.as_mut() {
                Some(service) => service.handle(bytes),
                None => return Vec::new(),
            };
            return vec![Action::Send {
                port: PortId(port),
                bytes: reply,
            }];
        }
        self.engine
            .receive(PortId(port), bytes, pm_engine::clock::host_now())
    }

    /// The font service, once main has attached a font port: the engine's
    /// own text rendering lays text out and reads outlines through it.
    pub fn fonts(&mut self) -> Option<&mut fonts::FontService> {
        self.fonts.as_mut()
    }

    pub fn tick(&mut self) -> Vec<Action> {
        self.engine.tick(pm_engine::clock::host_now())
    }

    pub fn status(&self) -> EngineStatus {
        self.engine.status()
    }

    /// Status as JSON, with the thread's QoS class and the presenter's
    /// counters added.
    pub fn status_json(&self) -> String {
        let mut value = serde_json::to_value(self.status()).unwrap_or_default();
        if let Some(object) = value.as_object_mut() {
            object.insert("qos".into(), current_qos().into());
            object.insert("fontPorts".into(), self.font_ports.len().into());
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

    #[test]
    fn the_job_engine_runs_at_utility_qos() {
        // Run on a fresh thread so the test runner's thread keeps its class.
        let qos = std::thread::spawn(|| {
            let mut host = EngineHost::new(EngineProcess::Job).unwrap();
            host.attach(1, "host").unwrap();
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
}
