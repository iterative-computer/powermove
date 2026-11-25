//! `telemetry.json` in the profile: the telemetry thread's own bookkeeping
//! (the anonymous install id, the sessions that sent, crash reports already
//! handled, Store installs it saw). Never the sharing choice: that is
//! Settings' own value (`shareUsageData`), which the kernel reports when the
//! library attaches and when it changes; deleting or damaging this file can
//! never turn sharing on. Written whole through a temporary file and a rename.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct State {
    /// The distinct id: random, made on the first launch that sends.
    pub install_id: Option<String>,
    /// Set while a session runs; still set at the next launch means the
    /// session did not end with a clean quit.
    pub session_open: bool,
    /// The account id last sent with `$identify`.
    pub identified: Option<String>,
    /// When the Mac had started at the last launch (a different value means
    /// this launch is the first since the Mac started: a cold launch).
    pub last_boot: Option<f64>,
    /// Recent eligible sessions: only their crashes are ever sent.
    pub sessions: Vec<Session>,
    /// Crash report incident ids already handled (sent or refused).
    pub crash_incidents: Vec<String>,
    /// Extensions installed from the Store catalogue (their public ids).
    pub store_installs: Vec<String>,
}

/// One eligible session of this app (a window app on a shipping bundle, or
/// a development check), and whether it was sending.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Session {
    pub pid: u64,
    /// Process start (seconds since 1970).
    pub started: f64,
    pub internal: bool,
    /// Sharing was on at some point.
    pub consented: bool,
    /// Sharing was turned off during it: none of its crashes leave.
    pub revoked: bool,
}

pub const MAX_SESSIONS: usize = 16;
pub const MAX_INCIDENTS: usize = 256;
pub const MAX_STORE_INSTALLS: usize = 200;

/// Keeps the newest `max` entries.
pub fn cap<T>(list: &mut Vec<T>, max: usize) {
    let extra = list.len().saturating_sub(max);
    list.drain(..extra);
}

pub fn path(profile: &Path) -> PathBuf {
    profile.join("telemetry.json")
}

/// The stored state; the default when there is none or it can't be read.
pub fn load(profile: &Path) -> State {
    std::fs::read(path(profile))
        .ok()
        .filter(|bytes| bytes.len() < 1 << 20)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn store(profile: &Path, state: &State) -> std::io::Result<()> {
    let file = path(profile);
    let tmp = file.with_extension("json.tmp");
    std::fs::create_dir_all(profile)?;
    std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
    std::fs::rename(tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_survives_a_reload_and_a_bad_file_reads_as_new() {
        let dir = tempfile::tempdir().unwrap();
        let state = State {
            install_id: Some("id".into()),
            session_open: true,
            crash_incidents: vec!["A".into()],
            ..Default::default()
        };
        store(dir.path(), &state).unwrap();
        assert_eq!(load(dir.path()), state);
        std::fs::write(path(dir.path()), b"not json").unwrap();
        assert_eq!(load(dir.path()), State::default());
        // An old file's sharing flag means nothing now.
        std::fs::write(path(dir.path()), br#"{"sharing":true}"#).unwrap();
        assert_eq!(load(dir.path()), State::default());
    }

    #[test]
    fn lists_keep_their_newest_entries() {
        let mut list: Vec<u32> = (0..20).collect();
        cap(&mut list, 16);
        assert_eq!(list.first(), Some(&4));
        assert_eq!(list.len(), 16);
    }
}
