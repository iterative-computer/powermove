//! Local records only. No crash uploader, profile discovery, or project/file contents.
use crate::error::Result;
use crate::service_ensure as ensure;
use crate::storage::{ServiceStorage, component};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Level {
    Info,
    Warn,
    Error,
    Uncaught,
}
/// Host redaction runs before local log persistence. Bundle exports omit all free text.
pub trait LogRedactor {
    fn redact(&mut self, text: &str) -> String;
}
pub struct OmitText;
impl LogRedactor for OmitText {
    fn redact(&mut self, _: &str) -> String {
        "[text omitted]".into()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Log {
    level: Level,
    code: String,
    text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashRecord {
    pub id: String,
    pub process: String,
    pub reason: String,
    pub recovered: bool,
}
impl CrashRecord {
    fn validate(&self) -> Result<()> {
        component(&self.id)?;
        ensure!(
            ["kernel", "engine", "extension", "shell"].contains(&self.process.as_str()),
            "unknown crash process"
        );
        ensure!(
            ["panic", "terminated", "hang", "out-of-memory", "unknown"]
                .contains(&self.reason.as_str()),
            "unknown crash reason"
        );
        Ok(())
    }
}
pub struct Diagnostics;
impl Diagnostics {
    pub fn log(
        storage: &mut ServiceStorage,
        redactor: &mut impl LogRedactor,
        level: Level,
        code: &str,
        text: &str,
    ) -> Result<()> {
        component(code)?;
        ensure!(text.len() <= 64 * 1024, TooLarge; "log message too large");
        let text: String = redactor
            .redact(text)
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(16 * 1024)
            .collect();
        let log = Log {
            level,
            code: code.into(),
            text,
        };
        let dir = storage.directory("logs", "events")?;
        storage.publish(&dir, serde_json::to_vec(&log)?.as_slice())?;
        Ok(())
    }
    pub fn crash(storage: &mut ServiceStorage, crash: CrashRecord) -> Result<()> {
        crash.validate()?;
        storage.set(
            &format!("services.crash.{}", crash.id),
            serde_json::to_value(crash)?,
        )?;
        Ok(())
    }
    pub fn crash_status(storage: &ServiceStorage, id: &str) -> Result<Option<CrashRecord>> {
        component(id)?;
        let value = storage.setting(&format!("services.crash.{id}"))?;
        value
            .map(|value| {
                let crash: CrashRecord = serde_json::from_value(value)?;
                crash.validate()?;
                ensure!(crash.id == id, "crash identity mismatch");
                Ok(crash)
            })
            .transpose()
    }
    /// Explicitly selected crashes only; never walks profiles, attachments, or raw OS dumps.
    /// This deterministic JSON bundle can be inspected or saved by the native shell.
    pub fn bundle(storage: &ServiceStorage, crash_ids: &[String]) -> Result<Vec<u8>> {
        ensure!(crash_ids.len() <= 128, "too many crashes");
        let mut crashes = Vec::new();
        for id in crash_ids {
            if let Some(crash) = Self::crash_status(storage, id)? {
                crashes.push(crash);
            }
        }
        let dir = storage.directory("logs", "events")?;
        let entries = crate::storage::generations(&dir)?;
        let mut logs = Vec::new();
        for (_, path) in entries.into_iter().rev().take(256).rev() {
            let log: Log =
                serde_json::from_slice(&crate::storage::read_capped(&path, 128 * 1024)?)?;
            component(&log.code)?;
            logs.push(serde_json::json!({"level":log.level,"code":log.code}));
        }
        Ok(serde_json::to_vec(
            &serde_json::json!({"format":"powermove.diagnostics","version":1,"logs":logs,"crashes":crashes}),
        )?)
    }
}
