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
    /// Unix milliseconds; 0 for records written before timestamps.
    #[serde(default)]
    at_ms: u64,
}
/// A persisted log row for the local Error Log window. Never exported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub level: Level,
    pub code: String,
    pub text: String,
    pub at_ms: u64,
}
const CLEARED: &str = "services.log.cleared";
const CRASHES: &str = "services.crash.index";
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashRecord {
    pub id: String,
    pub process: String,
    pub reason: String,
    pub recovered: bool,
    /// Local-only display name for the next-launch recovery notice (review item 6).
    /// Shell substitutes it in: Powermove recovered “Launch Film” after an unexpected quit.
    /// Render with pm-design's reading style and one Show button, as specified.
    /// Never export this user-authored name in diagnostics bundles or logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovered_document: Option<String>,
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
            at_ms: now_ms(),
        };
        let dir = storage.directory("logs", "events")?;
        storage.publish(&dir, serde_json::to_vec(&log)?.as_slice())?;
        Ok(())
    }
    /// The newest `limit` rows since the last Clear Log, oldest first.
    pub fn recent(storage: &ServiceStorage, limit: usize) -> Result<Vec<LogEntry>> {
        let cleared = storage
            .setting(CLEARED)?
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let dir = storage.directory("logs", "events")?;
        let mut out = Vec::new();
        for (_, path) in crate::storage::generations(&dir)?.into_iter().rev() {
            if out.len() >= limit {
                break;
            }
            let log: Log =
                serde_json::from_slice(&crate::storage::read_capped(&path, 128 * 1024)?)?;
            if log.at_ms <= cleared {
                break;
            }
            component(&log.code)?;
            out.push(LogEntry {
                level: log.level,
                code: log.code,
                text: log.text,
                at_ms: log.at_ms,
            });
        }
        out.reverse();
        Ok(out)
    }
    /// Error Log ▸ Clear Log. Records stay on disk (immutable generations);
    /// the window shows only what came after.
    pub fn clear(storage: &mut ServiceStorage) -> Result<()> {
        storage.set(CLEARED, serde_json::json!(now_ms()))?;
        Ok(())
    }
    /// Every recorded crash id, oldest first.
    pub fn crashes(storage: &ServiceStorage) -> Result<Vec<String>> {
        Ok(storage
            .setting(CRASHES)?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }
    pub fn crash(storage: &mut ServiceStorage, crash: CrashRecord) -> Result<()> {
        crash.validate()?;
        let mut index = Self::crashes(storage)?;
        if !index.contains(&crash.id) {
            index.push(crash.id.clone());
            ensure!(index.len() <= 4096, TooLarge; "too many crash records");
            storage.set(CRASHES, serde_json::to_value(&index)?)?;
        }
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
                // Explicit allowlist: recovery names are local UI data, not diagnostics.
                crashes.push(serde_json::json!({
                    "id": crash.id, "process": crash.process,
                    "reason": crash.reason, "recovered": crash.recovered,
                }));
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
