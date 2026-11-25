//! Events waiting to be sent, on disk in `<profile>/telemetry-outbox/`: one
//! JSON file per batch that could not be sent (offline, PostHog down, the
//! app quitting), per panic (written by the panicking thread before the app
//! may end), and per hang in progress. Sent oldest first and deleted once
//! PostHog has them; every event keeps its uuid, so a resend after a lost
//! acknowledgement is deduplicated by PostHog.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One event as it is sent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub uuid: String,
    pub event: String,
    pub distinct_id: String,
    /// Milliseconds since 1970.
    pub timestamp_ms: i64,
    pub properties: serde_json::Map<String, serde_json::Value>,
}

impl Record {
    pub fn to_posthog(&self) -> posthog::Event {
        let mut event = posthog::Event::new(self.event.clone(), self.distinct_id.clone());
        for (key, value) in &self.properties {
            let _ = event.insert_prop(key.clone(), value);
        }
        if let Some(ts) = chrono::DateTime::from_timestamp_millis(self.timestamp_ms) {
            let _ = event.set_timestamp(ts);
        }
        if let Ok(uuid) = uuid::Uuid::parse_str(&self.uuid) {
            event.set_uuid(uuid);
        }
        event
    }
}

/// The outbox keeps at most this many files (the oldest go first).
const MAX_FILES: usize = 200;
/// The in-progress hang, replaced by the real event once the app answers.
pub const HANG_PENDING: &str = "hang-pending.json";

pub struct Outbox {
    dir: PathBuf,
}

impl Outbox {
    pub fn new(profile: &Path) -> Outbox {
        Outbox {
            dir: profile.join("telemetry-outbox"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes one batch; `name` keeps panics and the pending hang apart.
    pub fn write(&self, name: &str, records: &[Record]) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        let file = self.dir.join(name);
        let tmp = self.dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec(records)?)?;
        std::fs::rename(&tmp, &file)?;
        Ok(file)
    }

    /// A batch name that sorts by time.
    pub fn batch_name(kind: &str) -> String {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        format!("{ms:015}-{kind}-{}.json", uuid::Uuid::new_v4().simple())
    }

    /// Waiting files, oldest first, trimmed to the cap; the pending hang is
    /// left alone (the watchdog owns it while this session runs).
    pub fn pending(&self, include_hang: bool) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return vec![];
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                name.ends_with(".json")
                    && !name.starts_with('.')
                    && (include_hang || name != HANG_PENDING)
            })
            .collect();
        files.sort();
        while files.len() > MAX_FILES {
            let _ = std::fs::remove_file(files.remove(0));
        }
        files
    }

    pub fn read(file: &Path) -> Vec<Record> {
        std::fs::read(file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Sharing turned off: everything waiting goes.
    pub fn clear(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(n: i64) -> Record {
        Record {
            uuid: uuid::Uuid::now_v7().to_string(),
            event: "app_quit".into(),
            distinct_id: "install".into(),
            timestamp_ms: n,
            properties: Default::default(),
        }
    }

    #[test]
    fn batches_come_back_oldest_first_and_clear_removes_them() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = Outbox::new(dir.path());
        outbox.write("002-batch.json", &[record(2)]).unwrap();
        outbox.write("001-batch.json", &[record(1)]).unwrap();
        outbox.write(HANG_PENDING, &[record(3)]).unwrap();
        let files = outbox.pending(false);
        assert_eq!(files.len(), 2);
        assert_eq!(Outbox::read(&files[0])[0].timestamp_ms, 1);
        assert_eq!(outbox.pending(true).len(), 3);
        outbox.clear();
        assert!(outbox.pending(true).is_empty());
    }

    #[test]
    fn a_record_becomes_the_same_posthog_event_every_time() {
        let mut r = record(1_700_000_000_000);
        r.properties.insert("clean".into(), true.into());
        let a = serde_json::to_value(r.to_posthog()).unwrap();
        let b = serde_json::to_value(r.to_posthog()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a["uuid"], r.uuid);
        assert_eq!(a["properties"]["clean"], true);
    }
}
