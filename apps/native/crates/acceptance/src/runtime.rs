//! Optional, separately locked native worker backing; missing backing is a blocker.
use crate::fixture::Scratch;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub const CASES: &[(&str, &str, &str, bool)] = &[
    (
        "A690",
        "W040",
        "Actual v1–3 StorageAPI get/set/delete, isolation and reload in native V8; disk restart not claimed",
        true,
    ),
    (
        "W040.compat-kernel-history",
        "W040",
        "Actual V8 history closures, native kernel writes and exact Undo/Redo; history.selection remains unattached",
        true,
    ),
    (
        "W040.compat-kernel-edits",
        "W040",
        "Actual V8 typed edit begin/dispatch/commit/cancel/apply through native kernel; structural mutate remains unattached",
        true,
    ),
    (
        "W040.compat-native-pixels",
        "W040",
        "Actual compat JS recorder to native CPU pixels with retained source version; full Canvas and WebKit remain open",
        true,
    ),
    (
        "A693",
        "W040",
        "Actual v1–3 Activation-frozen VarsAPI in native V8; no UI claimed",
        true,
    ),
    (
        "A695",
        "W040",
        "Actual v1–3 All five menu contribution locations in native V8; no UI claimed",
        true,
    ),
    (
        "A696",
        "W040",
        "Actual v1–3 Conditional Inspector sections and placements in native V8; no UI claimed",
        true,
    ),
    (
        "A698",
        "W040",
        "Actual v1–3 Status contribution metadata and live callbacks in native V8; no UI claimed",
        true,
    ),
    (
        "A510",
        "W037",
        "MCP forks synthetic shipped sources into isolated staging, retaining pristine merge base and archives",
        false,
    ),
    (
        "A660",
        "W040",
        "Actual V8 KeybindingsAPI; no native UI claimed",
        true,
    ),
    (
        "A661",
        "W040",
        "Actual V8 EffectsAPI; no native UI claimed",
        true,
    ),
    (
        "A662",
        "W040",
        "Actual V8 TransitionsAPI; no native UI claimed",
        true,
    ),
    (
        "A663",
        "W040",
        "Actual V8 ExtensionLayersAPI; no native UI claimed",
        true,
    ),
    (
        "A666",
        "W040",
        "Actual V8 ThemeAPI data and events; no native UI claimed",
        true,
    ),
    (
        "A668",
        "W040",
        "Actual V8 MenusAPI synchronous and asynchronous contributions; no native UI claimed",
        true,
    ),
    (
        "A669",
        "W040",
        "Actual V8 StatusAPI live callbacks; no native UI claimed",
        true,
    ),
    (
        "A678",
        "W040",
        "Actual V8 InspectorAPI section registration and disposal; no native UI claimed",
        true,
    ),
    (
        "B568",
        "W073",
        "Actual V8 Frozen control binding/gesture unit contract with explicit recording API sink; no native UI claimed",
        true,
    ),
    (
        "B569",
        "W073",
        "Actual V8 Frozen control binding/gesture unit contract with explicit recording API sink; no native UI claimed",
        true,
    ),
    (
        "B570",
        "W073",
        "Actual V8 Frozen control binding/gesture unit contract with explicit recording API sink; no native UI claimed",
        true,
    ),
    (
        "B610",
        "W073",
        "Actual V8 Frozen control binding/gesture unit contract with explicit recording API sink; no native UI claimed",
        true,
    ),
    (
        "B611",
        "W073",
        "Actual V8 Frozen control binding/gesture unit contract with explicit recording API sink; no native UI claimed",
        true,
    ),
    (
        "B612",
        "W073",
        "Actual V8 Frozen control binding/gesture unit contract with explicit recording API sink; no native UI claimed",
        true,
    ),
    (
        "B613",
        "W073",
        "Actual V8 Frozen control binding/gesture unit contract with explicit recording API sink; no native UI claimed",
        true,
    ),
    (
        "B614",
        "W073",
        "Actual V8 Frozen control binding/gesture unit contract with explicit recording API sink; no native UI claimed",
        true,
    ),
    (
        "A659",
        "W040",
        "Actual v1–3 CommandsAPI registrations, exact returns and override/disposal in V8",
        true,
    ),
    (
        "A689",
        "W040",
        "Actual v1–3 ServicesAPI live object identity and owned stack disposal in V8",
        true,
    ),
    (
        "A691",
        "W040",
        "Actual v1–3 EventsAPI synchronous payload identity, reentrancy and owned disposal in V8",
        true,
    ),
    (
        "W040.compat-remote-dom",
        "W040",
        "Current remote DOM shim in real V8 preserves input identity, callback state and refused-batch retry; WebKit remains open",
        true,
    ),
    (
        "W022.proxy-identity",
        "W022",
        "Merged promedia proxy-only catalog retains original identity, warnings and exact rate while refusing mismatched relinks",
        false,
    ),
    (
        "W037.v8-reload",
        "W037",
        "Real V8 failed activation preserves live generation and storage",
        false,
    ),
    (
        "W038.v8-stop",
        "W038",
        "Real V8 explicit stop preserves a second isolate; process isolation still needs macOS",
        false,
    ),
    (
        "W037.compat-services",
        "W037",
        "Actual v1–3 compat services preserve live identity and provider disposal in V8",
        true,
    ),
    (
        "W040.compat-recorder",
        "W040",
        "Actual compat recorder pins immutable commands; native pixels remain unverified",
        true,
    ),
    (
        "W001.service-entry",
        "W001",
        "Merged service file-open queue retries without losing or duplicating requests; OS delivery remains open",
        false,
    ),
    (
        "W012.service-storage",
        "W012",
        "Merged service storage retains unknown settings, tombstones and backups through reopening",
        false,
    ),
    (
        "W019.background-tasks",
        "W019",
        "Merged promedia cancellation and pause fence stale worker completion",
        false,
    ),
    (
        "W022.media-archive",
        "W022",
        "Merged promedia storage refuses referenced purge and archives all source classes",
        false,
    ),
    (
        "W077.render-cache",
        "W077",
        "Merged render cache retains Keep Rendered and invalidates edited graph identity",
        false,
    ),
    (
        "W040.native-raster",
        "W040",
        "Merged recorder and raster execute exact CPU pixels and reject stale command replay",
        false,
    ),
    (
        "W040.remote-router",
        "W040",
        "Merged remote DOM router preserves batch/event identity under backpressure",
        false,
    ),
    (
        "W032.native-surface",
        "W032",
        "Merged six-op surface preserves atomic edits and retries original-binding cancellation",
        false,
    ),
    (
        "W038.store-view",
        "W038",
        "Merged Store host view recovery fences stale generations and preserves extension data identity",
        false,
    ),
];
pub trait RuntimeHook {
    fn has_compat_bundle(&self) -> bool;
    fn check(&self, id: &str) -> Result<(), String>;
}
pub struct NativeRuntime {
    pub executable: PathBuf,
    pub compat_bundle: Option<PathBuf>,
}
impl RuntimeHook for NativeRuntime {
    fn has_compat_bundle(&self) -> bool {
        self.compat_bundle.is_some()
    }
    fn check(&self, id: &str) -> Result<(), String> {
        self.check_timeout(id, Duration::from_secs(20))
    }
}
impl NativeRuntime {
    pub fn check_timeout(&self, id: &str, timeout: Duration) -> Result<(), String> {
        let scratch = Scratch::new().map_err(|e| e.to_string())?;
        let out = scratch.0.join("runtime.json");
        let output = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&self.executable);
        cmd.arg(id);
        if let Some(bundle) = &self.compat_bundle {
            cmd.arg(bundle);
        }
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "cannot start native acceptance worker acceptance backing")?;
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(if result.is_err() {
                        "native acceptance worker backing wait failed"
                    } else {
                        "native acceptance worker backing timed out"
                    }
                    .into());
                }
            }
        };
        if !status.success() {
            return Err(format!(
                "native acceptance worker acceptance {id} failed ({status}); run backing directly for synthetic diagnostics"
            ));
        }
        if std::fs::metadata(&out).map_err(|e| e.to_string())?.len() > 1024 {
            return Err("native acceptance worker result exceeded 1024 bytes".into());
        }
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out).map_err(|e| e.to_string())?)
                .map_err(|_| "malformed native acceptance worker result")?;
        if result != serde_json::json!({"ok":true}) {
            return Err("native acceptance worker backing did not certify its assertion".into());
        }
        Ok(())
    }
}
