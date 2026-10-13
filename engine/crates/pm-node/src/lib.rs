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

#[cfg(feature = "node")]
mod node;

/// Parses `"interactive"` or `"job"`.
pub fn parse_process(text: &str) -> Result<EngineProcess, String> {
    match text {
        "interactive" => Ok(EngineProcess::Interactive),
        "job" => Ok(EngineProcess::Job),
        other => Err(format!("unknown engine process {other:?}")),
    }
}

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

/// One engine process as its JavaScript host sees it.
pub struct EngineHost {
    engine: Engine,
}

impl EngineHost {
    pub fn new(process: EngineProcess) -> Result<Self, String> {
        apply_process_qos(process)?;
        Ok(Self {
            engine: Engine::new(EngineConfig::new(process)),
        })
    }

    pub fn attach(&mut self, port: u32, peer: &str) -> Result<Vec<Action>, String> {
        Ok(self.engine.attach(PortId(port), parse_peer(peer)?))
    }

    pub fn detach(&mut self, port: u32) -> Vec<Action> {
        self.engine.detach(PortId(port))
    }

    pub fn receive(&mut self, port: u32, bytes: &[u8]) -> Vec<Action> {
        self.engine
            .receive(PortId(port), bytes, pm_engine::clock::host_now())
    }

    pub fn tick(&mut self) -> Vec<Action> {
        self.engine.tick(pm_engine::clock::host_now())
    }

    pub fn status(&self) -> EngineStatus {
        self.engine.status()
    }

    /// Status as JSON, with the thread's QoS class added.
    pub fn status_json(&self) -> String {
        let mut value = serde_json::to_value(self.status()).unwrap_or_default();
        if let Some(object) = value.as_object_mut() {
            object.insert("qos".into(), current_qos().into());
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
