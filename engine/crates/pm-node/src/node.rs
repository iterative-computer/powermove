//! The addon surface (feature `node`), used by
//! `apps/desktop/src/main/engine/engine-process.ts`.

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use pm_engine::{Action, LogLevel};

use crate::EngineHost;

/// One thing the utility process must do, in order.
#[napi(object)]
pub struct EngineAction {
    /// `"send"`, `"close"`, or `"log"`.
    pub kind: String,
    /// The port for `send` and `close`.
    pub port: u32,
    /// One encoded envelope, for `send`.
    pub bytes: Option<Uint8Array>,
    /// `"info"`, `"warn"`, or `"error"`, for `log`.
    pub level: Option<String>,
    pub message: Option<String>,
}

fn convert(actions: Vec<Action>) -> Vec<EngineAction> {
    actions
        .into_iter()
        .map(|action| match action {
            Action::Send { port, bytes } => EngineAction {
                kind: "send".into(),
                port: port.0,
                bytes: Some(Uint8Array::new(bytes)),
                level: None,
                message: None,
            },
            Action::Close { port } => EngineAction {
                kind: "close".into(),
                port: port.0,
                bytes: None,
                level: None,
                message: None,
            },
            Action::Log { level, message } => EngineAction {
                kind: "log".into(),
                port: 0,
                bytes: None,
                level: Some(
                    match level {
                        LogLevel::Info => "info",
                        LogLevel::Warn => "warn",
                        LogLevel::Error => "error",
                    }
                    .into(),
                ),
                message: Some(message),
            },
        })
        .collect()
}

type ActionCallback = ThreadsafeFunction<Vec<EngineAction>, (), Vec<EngineAction>, Status, false>;

fn err(message: impl Into<String>) -> Error {
    Error::from_reason(message.into())
}

/// The engine of this utility process.
#[napi(js_name = "EngineHost")]
pub struct JsEngineHost {
    inner: EngineHost,
}

#[napi]
impl JsEngineHost {
    /// `process` is `"interactive"` or `"job"`. The job engine moves the
    /// calling thread (the utility process's JavaScript thread) to utility QoS.
    ///
    /// `on_actions` receives actions the engine produces off this thread
    /// (the viewer presenter's frames, clock samples, and frame responses);
    /// carry them out like the actions the other methods return. Without it
    /// the interactive engine does not present viewer frames.
    #[napi(constructor)]
    pub fn new(process: String, on_actions: Option<ActionCallback>) -> Result<Self> {
        let process = crate::parse_process(&process).map_err(err)?;
        let sink: Option<crate::ActionSink> = on_actions.map(|callback| {
            let callback = std::sync::Arc::new(callback);
            std::sync::Arc::new(move |actions: Vec<Action>| {
                callback.call(convert(actions), ThreadsafeFunctionCallMode::NonBlocking);
            }) as crate::ActionSink
        });
        Ok(Self {
            inner: EngineHost::with_sink(process, sink, |key| std::env::var(key).ok())
                .map_err(err)?,
        })
    }

    /// A port from main. `peer` is `"host"`, `"renderer"`, or `"peer"`.
    #[napi]
    pub fn attach(&mut self, port: u32, peer: String) -> Result<Vec<EngineAction>> {
        Ok(convert(self.inner.attach(port, &peer).map_err(err)?))
    }

    /// The port closed.
    #[napi]
    pub fn detach(&mut self, port: u32) -> Vec<EngineAction> {
        convert(self.inner.detach(port))
    }

    /// One message's bytes from `port`.
    #[napi]
    pub fn receive(&mut self, port: u32, bytes: Uint8Array) -> Vec<EngineAction> {
        convert(self.inner.receive(port, &bytes))
    }

    /// Periodic work (lease reports, breadcrumbs). Call every 100 ms.
    #[napi]
    pub fn tick(&mut self) -> Vec<EngineAction> {
        convert(self.inner.tick())
    }

    /// What the job engine's lanes produced off this thread (agent frames,
    /// job status, host calls). Call after `onActions` fires.
    #[napi]
    pub fn pump(&mut self) -> Vec<EngineAction> {
        convert(self.inner.pump())
    }

    /// The engine's state as JSON (sessions, ports, lease, QoS, presenter).
    #[napi]
    pub fn status(&self) -> String {
        self.inner.status_json()
    }

    /// Destroys the viewer presenter's GPU device as a real loss would (the
    /// recovery test); false when this engine does not present.
    #[napi]
    pub fn simulate_gpu_loss(&self) -> bool {
        self.inner.simulate_gpu_loss()
    }

    /// The viewer presenter's per-frame trace as JSON, emptied (the S1
    /// harness; recorded only with `POWERMOVE_BRIDGE_TRACE=1`).
    #[napi]
    pub fn present_trace(&self) -> String {
        self.inner.take_present_trace()
    }
}

/// The protocol build id this addon speaks (`PROTOCOL_BUILD` in TypeScript).
#[napi]
pub fn build_id() -> String {
    pm_protocol::version::SOURCE_BUILD.to_owned()
}

/// The shared host clock in microseconds (contract 13 `HostMicros`).
#[napi]
pub fn host_now_us() -> f64 {
    pm_engine::clock::host_now().get() as f64
}
