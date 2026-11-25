//! `telemetry.json` in the profile: the person's sharing choice, the
//! anonymous install id and the telemetry thread's own bookkeeping. Written
//! whole through a temporary file and a rename.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct State {
    /// "Share anonymous usage and crash reports" (on unless turned off).
    pub sharing: bool,
    /// The distinct id: random, made on the first launch that sends.
    pub install_id: Option<String>,
    /// The newest crash report already sent (seconds since 1970).
    pub last_crash_report: Option<f64>,
    /// Set while a session runs; still set at the next launch means the
    /// session did not end with a clean quit.
    pub session_open: bool,
    /// The account id last sent with `$identify`.
    pub identified: Option<String>,
    /// When the Mac had started at the last launch (a different value means
    /// this launch is the first since the Mac started: a cold launch).
    pub last_boot: Option<f64>,
}

impl Default for State {
    fn default() -> Self {
        State {
            sharing: true,
            install_id: None,
            last_crash_report: None,
            session_open: false,
            identified: None,
            last_boot: None,
        }
    }
}

pub fn path(profile: &Path) -> PathBuf {
    profile.join("telemetry.json")
}

/// The stored state; the default when there is none or it can't be read.
pub fn load(profile: &Path) -> State {
    std::fs::read(path(profile))
        .ok()
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
    #[test]
    fn sharing_is_on_until_turned_off_and_survives_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        assert!(super::load(dir.path()).sharing);
        let state = super::State {
            sharing: false,
            ..Default::default()
        };
        super::store(dir.path(), &state).unwrap();
        assert_eq!(super::load(dir.path()), state);
        std::fs::write(super::path(dir.path()), b"not json").unwrap();
        assert!(super::load(dir.path()).sharing);
    }
}
