//! Optional, separately locked real-V8 backing; missing backing is a blocker.
use crate::fixture::Scratch;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub const CASES: &[(&str, &str, &str, bool)] = &[
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
            .map_err(|_| "cannot start native V8 acceptance backing")?;
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
                        "native V8 backing wait failed"
                    } else {
                        "native V8 backing timed out"
                    }
                    .into());
                }
            }
        };
        if !status.success() {
            return Err(format!(
                "native V8 acceptance {id} failed ({status}); run backing directly for synthetic diagnostics"
            ));
        }
        if std::fs::metadata(&out).map_err(|e| e.to_string())?.len() > 1024 {
            return Err("native V8 result exceeded 1024 bytes".into());
        }
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out).map_err(|e| e.to_string())?)
                .map_err(|_| "malformed native V8 result")?;
        if result != serde_json::json!({"ok":true}) {
            return Err("native V8 backing did not certify its assertion".into());
        }
        Ok(())
    }
}
