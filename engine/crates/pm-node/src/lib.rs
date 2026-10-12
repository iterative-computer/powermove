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

/// The job engine runs at utility QoS, never in the Darwin background
/// state (contract 13, invariant 10: background slows its reaction to
/// deadline signals and has no GPU effect). The interactive engine keeps
/// the default class; its render and audio threads raise their own.
pub fn apply_process_qos(process: EngineProcess) -> Result<(), String> {
    if process != EngineProcess::Job {
        return Ok(());
    }
    #[cfg(target_vendor = "apple")]
    {
        // SAFETY: sets the calling thread's QoS class; no pointers involved.
        let status =
            unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) };
        if status != 0 {
            return Err(format!("pthread_set_qos_class_self_np failed ({status})"));
        }
    }
    Ok(())
}

/// The calling thread's QoS class as text (for status and tests).
pub fn current_qos() -> &'static str {
    #[cfg(target_vendor = "apple")]
    {
        let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
        let mut priority = 0;
        // SAFETY: both out-pointers are valid for writes.
        let status = unsafe {
            libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut priority)
        };
        if status != 0 {
            return "unknown";
        }
        match class {
            libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE => "userInteractive",
            libc::qos_class_t::QOS_CLASS_USER_INITIATED => "userInitiated",
            libc::qos_class_t::QOS_CLASS_DEFAULT => "default",
            libc::qos_class_t::QOS_CLASS_UTILITY => "utility",
            libc::qos_class_t::QOS_CLASS_BACKGROUND => "background",
            _ => "unspecified",
        }
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        "unspecified"
    }
}

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
