//! What a later launch does with the original app. The rule:
//!
//! * The full-screen migration shows once: on the launch that imports the
//!   original library (`Launch::Migrate`). After that the person has moved to
//!   native, and the library here is theirs.
//! * A later launch never waits on the original app and never shows a window
//!   for it. Before the library opens it does one thing only: it finishes a
//!   capture that is already taken (`first-launch-deltas/pending`), unless that
//!   capture's last attempt failed. It never starts a new capture.
//! * Newer work in the original app is found with the source stamp (a metadata
//!   walk, no copy), off the launch path, and offered on Home and in
//!   Settings › Storage ([`newer_work`]).
//! * Bringing it in is the person's choice ([`request`]): the next launch runs
//!   it before the library opens, with progress ([`bring_in`]).
//! * A failed attempt is recorded with its reason ([`last_failure`]), and no
//!   launch retries it. Only the person's next request does.
use crate::{
    Config, DeltaPrepared, Event, LowDisk, Outcome, State, Step, checkpoint, delta, disk, validate,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

const DELTAS: &str = "first-launch-deltas";
const REQUEST: &str = "first-launch-deltas/requested";
const FAILURE: &str = "first-launch-deltas/last-failure.json";
const PENDING: &str = "first-launch-deltas/pending";
const STAGE: &str = "pending.capture";

/// Why the last attempt to bring in recent work stopped.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Failure {
    /// Milliseconds since 1970.
    pub at_ms: u64,
    /// Safe to show: no paths or names.
    pub reason: String,
    /// The free space it needed and what there was, when that was the reason.
    pub low_disk: Option<LowDiskRecord>,
    /// The capture it was finishing (its `checkpoint.json` hash), if one was taken.
    pub capture: Option<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LowDiskRecord {
    pub needed: u64,
    pub available: u64,
}

fn state(native: &Path) -> Option<State> {
    disk::json::<State>(&native.join("migration.json"))
        .ok()
        .filter(|s| s.completed.contains(&Step::Media))
}
/// The first launch brought the original library in here (so recent work can follow).
pub fn migrated(native: &Path) -> bool {
    state(native).is_some()
}
/// The original app has work this library hasn't brought in. Metadata only:
/// a walk of the original app's library folders, never a copy. Run off the
/// main thread. A taken capture that isn't merged yet counts as newer work.
pub fn newer_work(config: &Config) -> Result<bool> {
    if !config.electron_profile.is_dir() {
        return Ok(false);
    }
    let (source, native) = validate(config)?;
    let Some(state) = state(&native) else {
        return Ok(false);
    };
    if native.join(PENDING).exists() {
        return Ok(true);
    }
    Ok(state.source_stamp.as_deref() != Some(&checkpoint::source_stamp(&source)?))
}
/// The person asked to bring in recent work: the next launch does it before
/// the library opens. Clears the last failure, so that launch tries again.
pub fn request(native: &Path) -> Result<()> {
    disk::private_dir(&native.join(DELTAS))?;
    let _ = fs::remove_file(native.join(FAILURE));
    disk::bytes(&native.join(REQUEST), b"1\n")
}
pub fn requested(native: &Path) -> bool {
    native.join(REQUEST).is_file()
}
/// Not now after all: the next launch opens as usual.
pub fn cancel_request(native: &Path) {
    let _ = fs::remove_file(native.join(REQUEST));
}
pub fn last_failure(native: &Path) -> Option<Failure> {
    disk::json(&native.join(FAILURE)).ok()
}
fn capture_identity(native: &Path) -> Option<String> {
    let checkpoint = native.join(PENDING).join("checkpoint.json");
    checkpoint
        .is_file()
        .then(|| disk::hash(&checkpoint).ok().map(|h| h.0))
        .flatten()
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
/// Records why an attempt stopped, so no launch tries it again on its own.
pub fn record_failure(native: &Path, error: &anyhow::Error) -> Result<Failure> {
    let low_disk = error.downcast_ref::<LowDisk>().map(|l| LowDiskRecord {
        needed: l.needed,
        available: l.available,
    });
    let reason = if low_disk.is_some() {
        "There wasn’t enough free space on this Mac.".to_owned()
    } else if error
        .chain()
        .filter_map(|e| e.downcast_ref::<std::io::Error>())
        .any(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
    {
        "Some of the original app’s files couldn’t be read.".to_owned()
    } else {
        "Something in the original app’s library couldn’t be read.".to_owned()
    };
    let failure = Failure {
        at_ms: now_ms(),
        reason,
        low_disk,
        capture: capture_identity(native),
    };
    disk::private_dir(&native.join(DELTAS))?;
    disk::atomic(&native.join(FAILURE), &failure)?;
    Ok(failure)
}
/// The launch path, before the library opens: finishes a capture already
/// taken (an earlier attempt stopped between capture and merge), and nothing
/// else. A capture whose last attempt failed waits for the person to ask.
pub fn resume_pending(
    config: &Config,
    mut hook: impl FnMut(Event) -> Result<()>,
) -> Result<DeltaPrepared> {
    if !config.electron_profile.is_dir() {
        return Ok(DeltaPrepared::Unchanged);
    }
    let (_, native) = validate(config)?;
    let Some(capture) = capture_identity(&native) else {
        return Ok(DeltaPrepared::Unchanged);
    };
    if last_failure(&native).is_some_and(|f| f.capture.as_deref() == Some(&capture)) {
        return Ok(DeltaPrepared::Unchanged);
    }
    let result = delta::delta_store(config, &mut hook);
    if let Err(error) = &result {
        record_failure(&native, error)?;
    }
    result
}
/// The person's request, run before the library opens: capture, merge and
/// media in one go. Success clears the request and any failure. A failure is
/// recorded and clears the request too: the next launch opens as usual.
pub fn bring_in(config: &Config, hook: impl FnMut(Event) -> Result<()>) -> Result<Outcome> {
    let (_, native) = validate(config)?;
    let result = delta::delta(config, hook);
    match &result {
        Ok(Outcome::WaitingForElectron) => {}
        Ok(_) => {
            let _ = fs::remove_file(native.join(FAILURE));
            cancel_request(&native);
        }
        Err(error) => {
            record_failure(&native, error)?;
            cancel_request(&native);
        }
    }
    let _ = tidy(&native);
    result
}
/// Removes capture stages a stopped attempt left empty. Only empty folders go
/// (`remove_dir` refuses anything else), and only while no attempt is running.
pub fn tidy(native: &Path) -> Result<usize> {
    let deltas = native.join(DELTAS);
    if !deltas.is_dir() {
        return Ok(0);
    }
    let Ok(_lock) = delta::lease(native) else {
        return Ok(0);
    };
    let mut removed = 0;
    for entry in fs::read_dir(&deltas)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name == STAGE || name.starts_with(&format!("{STAGE}.archive-"))) {
            continue;
        }
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.is_dir() && fs::remove_dir(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_low_disk_failure_keeps_its_numbers() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let error = anyhow::Error::new(LowDisk {
            needed: 7,
            available: 3,
        });
        let failure = record_failure(&root, &error).unwrap();
        assert_eq!(
            failure.low_disk,
            Some(LowDiskRecord {
                needed: 7,
                available: 3
            })
        );
        assert_eq!(last_failure(&root), Some(failure));
        request(&root).unwrap();
        assert!(requested(&root));
        assert_eq!(last_failure(&root), None);
    }
    #[test]
    fn tidy_removes_only_empty_stages() {
        let temp = tempfile::tempdir().unwrap();
        let deltas = temp.path().join(DELTAS);
        fs::create_dir_all(deltas.join(STAGE)).unwrap();
        fs::create_dir_all(deltas.join("pending.capture.archive-0000000000000000")).unwrap();
        fs::create_dir_all(deltas.join("pending.capture.archive-0000000000000001/profile"))
            .unwrap();
        fs::create_dir_all(deltas.join("base")).unwrap();
        fs::write(temp.path().join("first-launch.lock"), b"").unwrap();
        assert_eq!(tidy(temp.path()).unwrap(), 2);
        assert!(!deltas.join(STAGE).exists());
        assert!(
            deltas
                .join("pending.capture.archive-0000000000000001")
                .exists()
        );
        assert!(deltas.join("base").exists());
    }
}
