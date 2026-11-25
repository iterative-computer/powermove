//! Events waiting to be sent, on disk in `<profile>/telemetry-outbox/`: one
//! JSON file per batch that could not be sent (offline, PostHog down, the app
//! quitting), per imported crash, and the hang in progress. Only the
//! telemetry thread writes here, and only while sharing is on. Every write
//! keeps the folder within [`MAX_FILES`] and [`MAX_BYTES`] (the oldest go);
//! a batch over [`MAX_FILE_BYTES`] is not kept. Sent oldest first and deleted
//! once PostHog has them; every event keeps its uuid, so a resend after a lost
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

pub const MAX_FILES: usize = 200;
pub const MAX_BYTES: u64 = 8 << 20;
pub const MAX_FILE_BYTES: u64 = 512 << 10;
/// The in-progress hang, replaced by the real event once the app answers.
pub const HANG_PENDING: &str = "hang-pending.json";

pub struct Outbox {
    dir: PathBuf,
}

impl Outbox {
    /// Leftover temporary files (a write the app did not finish) go at once.
    pub fn new(profile: &Path) -> Outbox {
        let outbox = Outbox {
            dir: profile.join("telemetry-outbox"),
        };
        if let Ok(entries) = std::fs::read_dir(&outbox.dir) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with('.') {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        outbox
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes one batch under `name`, then trims the folder to its caps.
    pub fn write(&self, name: &str, records: &[Record]) -> std::io::Result<PathBuf> {
        let bytes = serde_json::to_vec(records)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(std::io::Error::other("batch over the outbox file limit"));
        }
        std::fs::create_dir_all(&self.dir)?;
        let file = self.dir.join(name);
        let tmp = self
            .dir
            .join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
        let written = std::fs::write(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, &file));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written?;
        self.trim();
        Ok(file)
    }

    /// A batch name that sorts by time.
    pub fn batch_name(kind: &str) -> String {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        format!("{ms:015}-{kind}-{}.json", uuid::Uuid::new_v4().simple())
    }

    /// Kept files, oldest first, with their sizes.
    fn files(&self) -> Vec<(PathBuf, u64)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return vec![];
        };
        let mut files: Vec<(PathBuf, u64)> = entries
            .flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.ends_with(".json") && !name.starts_with('.')
            })
            .map(|e| (e.path(), e.metadata().map_or(0, |m| m.len())))
            .collect();
        files.sort();
        files
    }

    /// The oldest go until the folder is within its caps.
    fn trim(&self) {
        let mut files = self.files();
        let mut total: u64 = files.iter().map(|(_, n)| n).sum();
        while files.len() > MAX_FILES || total > MAX_BYTES {
            let (oldest, size) = files.remove(0);
            let _ = std::fs::remove_file(oldest);
            total = total.saturating_sub(size);
        }
    }

    /// Waiting files, oldest first; the pending hang is left alone (the
    /// telemetry thread owns it while this session runs).
    pub fn pending(&self, include_hang: bool) -> Vec<PathBuf> {
        self.files()
            .into_iter()
            .map(|(p, _)| p)
            .filter(|p| include_hang || p.file_name().is_some_and(|n| n != HANG_PENDING))
            .collect()
    }

    /// A kept batch; one over the size limit or unreadable is empty.
    pub fn read(file: &Path) -> Vec<Record> {
        if std::fs::metadata(file).map_or(true, |m| m.len() > MAX_FILE_BYTES) {
            return vec![];
        }
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

    /// Sol P1 (w99 review): offline writes can't grow without bound.
    #[test]
    fn every_outbox_write_respects_the_caps() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = Outbox::new(dir.path());
        for n in 0..(MAX_FILES as i64 + 40) {
            outbox
                .write(&format!("{n:015}-batch.json"), &[record(n)])
                .unwrap();
        }
        let files = outbox.pending(true);
        assert_eq!(files.len(), MAX_FILES);
        // The oldest went.
        assert_eq!(Outbox::read(&files[0])[0].timestamp_ms, 40);
        // A batch over the file limit is refused, not written.
        let mut big = record(0);
        big.properties
            .insert("x".into(), "y".repeat(MAX_FILE_BYTES as usize).into());
        assert!(outbox.write("big.json", &[big]).is_err());
        assert!(!outbox.dir().join("big.json").exists());
        // Bytes are capped too.
        let mut fat = record(0);
        fat.properties
            .insert("x".into(), "y".repeat(300 << 10).into());
        for n in 0..40 {
            outbox
                .write(
                    &format!("{:015}-fat.json", 1000 + n),
                    std::slice::from_ref(&fat),
                )
                .unwrap();
        }
        let total: u64 = outbox.files().iter().map(|(_, n)| n).sum();
        assert!(total <= MAX_BYTES, "{total}");
        // A temporary file a crash left is cleaned up when the outbox opens.
        std::fs::write(outbox.dir().join(".left.tmp"), b"x").unwrap();
        let reopened = Outbox::new(dir.path());
        assert!(!reopened.dir().join(".left.tmp").exists());
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
