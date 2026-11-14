//! Extension-only startup transfer. No kernel/library writer or credential API.
use crate::{Config, State, checkpoint::Checkpoint, disk, extension_import};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanStatus {
    New,
    Changed,
    AlreadyPresent,
    Skipped,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanEntry {
    pub id: String,
    pub status: ScanStatus,
    pub reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Scan {
    pub found: usize,
    pub new: usize,
    pub changed: usize,
    pub already_present: usize,
    pub entries: Vec<ScanEntry>,
    pub source_issue: Option<String>,
    pub declined: bool,
}
impl Scan {
    pub fn has_new_or_changed(&self) -> bool {
        self.new + self.changed > 0
    }
    pub fn should_offer(&self) -> bool {
        !self.declined && self.has_new_or_changed()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub current: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferOutcome {
    BroughtIn,
    AlreadyPresent,
    CouldNotLoad,
    Skipped,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtensionResult {
    pub id: String,
    pub outcome: TransferOutcome,
    pub reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TransferResult {
    pub brought_in: usize,
    pub already_present: usize,
    pub could_not_load: usize,
    pub entries: Vec<ExtensionResult>,
}
#[derive(Default, Serialize, Deserialize)]
struct Preferences {
    declined: bool,
}

/// Persistent Not Now. Settings can call set_declined(profile, false) to re-offer.
pub fn declined(profile: &Path) -> Result<bool> {
    let p = profile.join("extension-import/preferences.json");
    Ok(if p.exists() {
        disk::json::<Preferences>(&p)?.declined
    } else {
        false
    })
}
pub fn set_declined(profile: &Path, value: bool) -> Result<()> {
    let native = profile.canonicalize()?;
    let private = native.join("extension-import");
    disk::private_dir(&private)?;
    disk::atomic(
        &private.join("preferences.json"),
        &Preferences { declined: value },
    )
}
fn state(native: &Path, source: &Path) -> Result<State> {
    let receipt = native.join("extension-import/state.json");
    let migration = native.join("migration.json");
    let state: State = if receipt.exists() {
        disk::json(&receipt)?
    } else if migration.exists() {
        disk::json(&migration)?
    } else {
        crate::empty_state(source.into())
    };
    ensure!(
        state.source == source,
        "Extension receipts belong to another original profile"
    );
    Ok(state)
}
fn paths(config: &Config) -> Result<(PathBuf, PathBuf)> {
    let source = config.electron_profile.canonicalize()?;
    let native = config.native_profile.canonicalize()?;
    ensure!(
        !source.starts_with(&native) && !native.starts_with(&source),
        "Profiles overlap"
    );
    // Only these paths are writable. Refuse links, including nested extension links.
    for name in [
        "extensions",
        "extensions-shared",
        "extensions-provenance.json",
        "extension-import",
    ] {
        let p = native.join(name);
        ensure!(
            !fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()),
            "Linked extension destination"
        );
    }
    disk::walk(&native.join("extensions"))?;
    disk::walk(&native.join("extensions-shared"))?;
    Ok((source, native))
}
/// Read-only. Hashes only extension trees and reads receipts; does not compile or
/// execute code. Run off the UI thread for a large library/network volume.
pub fn scan(config: &Config) -> Result<Scan> {
    let mut result = Scan {
        declined: declined(&config.native_profile)?,
        ..Default::default()
    };
    if !config.electron_profile.exists() {
        result.source_issue = Some("The original app's profile is missing or unavailable".into());
        return Ok(result);
    }
    let (source, native) = paths(config)?;
    let state = state(&native, &source)?;
    let provenance = |root: &Path| -> Result<BTreeMap<String, Value>> {
        let p = root.join("extensions-provenance.json");
        if p.exists() {
            disk::json(&p)
        } else {
            Ok(BTreeMap::new())
        }
    };
    let incoming_provenance = provenance(&source)?;
    let current_provenance = provenance(&native)?;
    let entries = match fs::read_dir(source.join("extensions")) {
        Ok(entries) => entries,
        Err(error) => {
            result.source_issue = Some(format!(
                "Cannot read the original extension folder: {error}"
            ));
            return Ok(result);
        }
    };
    for entry in entries {
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if id.starts_with('.') {
            continue;
        }
        let dir = entry.path();
        let (status, reason) = (|| -> Result<(ScanStatus, String)> {
            if !entry.file_type()?.is_dir() {
                return Ok((ScanStatus::Skipped, "It isn’t an extension folder.".into()));
            }
            if !dir.join("manifest.json").is_file() {
                return Ok((
                    ScanStatus::Skipped,
                    "Its folder is empty or incomplete, so there’s nothing to bring in.".into(),
                ));
            }
            result.found += 1;
            let manifest: Value = disk::json(&dir.join("manifest.json"))?;
            ensure!(manifest.is_object(), "Manifest must be a JSON object");
            if let Some(manifest_id) = manifest.get("id") {
                ensure!(
                    manifest_id == &Value::String(id.clone()),
                    "Manifest id differs from its folder name"
                );
            }
            let next = extension_import::tree(&dir)?.context("Source disappeared")?;
            let current = extension_import::tree(&native.join("extensions").join(&id))?;
            let shared = extension_import::tree(&native.join("extensions-shared").join(&id))?;
            if current.as_ref() == Some(&next) {
                if incoming_provenance.contains_key(&id)
                    && incoming_provenance.get(&id) != current_provenance.get(&id)
                {
                    if current_provenance.get(&id) == state.extension_provenance.get(&id) {
                        return Ok((
                            ScanStatus::Changed,
                            "Its original app information was updated.".into(),
                        ));
                    }
                    return Ok((
                        ScanStatus::Skipped,
                        "You changed it in Powermove, so your copy is kept.".into(),
                    ));
                }
                return Ok((ScanStatus::AlreadyPresent, "Already in Powermove.".into()));
            }
            let receipt = state
                .extension_receipts
                .get(&id)
                .cloned()
                .unwrap_or_default();
            if current.is_none()
                && receipt.hash.is_empty()
                && shared.as_ref().is_some_and(|h| h != &next)
            {
                return Ok((
                    ScanStatus::Skipped,
                    "Powermove already has a different extension with this name, so yours is kept."
                        .into(),
                ));
            }
            if let Some(current) = current {
                if current != receipt.hash {
                    return Ok((
                        ScanStatus::Skipped,
                        "You changed it in Powermove, so your copy is kept.".into(),
                    ));
                }
                Ok((
                    ScanStatus::Changed,
                    "Updated in the original app since it was last brought in.".into(),
                ))
            } else {
                Ok((ScanStatus::New, "New in the original app.".into()))
            }
        })()
        .unwrap_or_else(|e| (ScanStatus::Skipped, format!("It couldn’t be read: {e}.")));
        match status {
            ScanStatus::New => result.new += 1,
            ScanStatus::Changed => result.changed += 1,
            ScanStatus::AlreadyPresent => result.already_present += 1,
            ScanStatus::Skipped => {}
        }
        result.entries.push(ScanEntry { id, status, reason });
    }
    result.entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

/// Explicit user action, even when declined. Call before extension hosts start,
/// or pause/rescan the app's hosts around this worker. `check` MUST activate in
/// an isolated synthetic realm, never against the user's open project. Return a
/// human-readable summary on success or the real compiler/activation error.
/// Files copied successfully but failing activation remain installed with their
/// receipt; subsequent runs retry validation without overwriting those files.
pub fn run(
    config: &Config,
    mut progress: impl FnMut(Progress),
    mut check: impl FnMut(&Path, &Value) -> std::result::Result<String, String>,
) -> Result<TransferResult> {
    let inventory = scan(config)?;
    ensure!(
        inventory.source_issue.is_none(),
        "{}",
        inventory.source_issue.as_deref().unwrap_or("")
    );
    let (source, native) = paths(config)?;
    let private = native.join("extension-import");
    disk::private_dir(&private)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(native.join("first-launch.lock"))?;
    lock.try_lock_exclusive()
        .context("Another extension import is running")?;
    // Re-scan under lock: another worker might have finished since the offer.
    let inventory = scan(config)?;
    let mut state = state(&native, &source)?;
    let receipt = private.join("state.json");
    disk::private_dir(&private.join("captures"))?;
    let count = inventory.entries.len();
    let total = count * 2;
    let mut result = TransferResult::default();
    for (index, entry) in inventory.entries.into_iter().enumerate() {
        progress(Progress {
            done: index,
            total,
            current: entry.id.clone(),
        });
        let mut outcome = match entry.status {
            ScanStatus::New | ScanStatus::Changed => TransferOutcome::BroughtIn,
            ScanStatus::AlreadyPresent => TransferOutcome::AlreadyPresent,
            ScanStatus::Skipped => TransferOutcome::Skipped,
        };
        let mut reason = entry.reason;
        if matches!(
            outcome,
            TransferOutcome::BroughtIn | TransferOutcome::AlreadyPresent
        ) {
            let attempt = (|| -> Result<String> {
                if outcome == TransferOutcome::BroughtIn {
                    let capture = private.join("captures").join(&entry.id);
                    if capture.exists() {
                        disk::archive(&capture)?;
                    }
                    disk::private_dir(&capture.join("profile/extensions"))?;
                    let dir = source.join("extensions").join(&entry.id);
                    let before = extension_import::tree(&dir)?;
                    let mut files = BTreeMap::new();
                    for suffix in disk::walk(&dir)?.keys() {
                        let relative = Path::new("extensions").join(&entry.id).join(suffix);
                        let out = capture.join("profile").join(&relative);
                        disk::private_dir(out.parent().unwrap())?;
                        files.insert(relative.clone(), disk::copy(&source.join(&relative), &out)?);
                    }
                    ensure!(
                        before == extension_import::tree(&dir)?,
                        "Source changed while copying; retry transfer"
                    );
                    ensure!(
                        before
                            == extension_import::tree(
                                &capture.join("profile/extensions").join(&entry.id)
                            )?,
                        "Captured extension differs from source"
                    );
                    // Only this extension's provenance may be merged.
                    let provenance = source.join("extensions-provenance.json");
                    if provenance.exists() {
                        let values: BTreeMap<String, Value> = disk::json(&provenance)?;
                        let incoming: BTreeMap<_, _> = values
                            .into_iter()
                            .filter(|(id, _)| id == &entry.id)
                            .collect();
                        disk::atomic(
                            &capture.join("profile/extensions-provenance.json"),
                            &incoming,
                        )?;
                    }
                    let cp = Checkpoint {
                        version: 1,
                        source: source.clone(),
                        files,
                        logical_keys: Default::default(),
                        projects: Default::default(),
                        indexeddb: vec![],
                        chromium_payloads: Default::default(),
                        local_storage: None,
                    };
                    disk::atomic(&capture.join("checkpoint.json"), &cp)?;
                    Checkpoint::load(&capture)?;
                    extension_import::import_with_publisher(
                        &cp,
                        &capture,
                        &native,
                        &mut state,
                        |s| disk::atomic(&receipt, s),
                    )?;
                    ensure!(
                        extension_import::tree(&native.join("extensions").join(&entry.id))?
                            == before,
                        "Native extension changed during transfer; your copy was kept"
                    );
                }
                Ok(String::new())
            })();
            match attempt {
                Ok(summary) => {
                    reason = format!("{reason} {summary}");
                }
                Err(error) => {
                    outcome = TransferOutcome::CouldNotLoad;
                    reason = error.to_string();
                }
            }
        }
        match outcome {
            TransferOutcome::BroughtIn => result.brought_in += 1,
            TransferOutcome::AlreadyPresent => result.already_present += 1,
            TransferOutcome::CouldNotLoad => result.could_not_load += 1,
            TransferOutcome::Skipped => {}
        }
        result.entries.push(ExtensionResult {
            id: entry.id.clone(),
            outcome,
            reason,
        });
        progress(Progress {
            done: index + 1,
            total,
            current: entry.id,
        });
    }
    // Publish all dependencies before validating any extension. The app can use
    // a shared synthetic-library checker rather than a one-extension realm.
    for (index, entry) in result.entries.iter_mut().enumerate() {
        if matches!(
            entry.outcome,
            TransferOutcome::BroughtIn | TransferOutcome::AlreadyPresent
        ) {
            let dir = native.join("extensions").join(&entry.id);
            let checked = disk::json::<Value>(&dir.join("manifest.json"))
                .map_err(|e| e.to_string())
                .and_then(|manifest| check(&dir, &manifest));
            match checked {
                Ok(summary) => entry.reason = format!("{} {}", entry.reason.trim(), summary),
                Err(error) => {
                    if entry.outcome == TransferOutcome::BroughtIn {
                        result.brought_in -= 1;
                    } else {
                        result.already_present -= 1;
                    }
                    result.could_not_load += 1;
                    entry.outcome = TransferOutcome::CouldNotLoad;
                    entry.reason = format!(
                        "It was copied but couldn’t load: {}",
                        error.lines().next().unwrap_or("Unknown loading error")
                    );
                }
            }
        }
        progress(Progress {
            done: count + index + 1,
            total,
            current: entry.id.clone(),
        });
    }
    state.extensions_imported = true;
    disk::atomic(&receipt, &state)?;
    disk::atomic(&private.join("last-result.json"), &result)?;
    Ok(result)
}

/// Maintenance wrapper: copies only. Startup UI should call `run` with the app's
/// isolated compiler/activation validator to get actual load-failure reasons.
pub fn import_extensions(config: &Config) -> Result<State> {
    run(
        config,
        |_| {},
        |_, _| Ok("Copied; runtime activation has not been checked".into()),
    )?;
    state(
        &config.native_profile.canonicalize()?,
        &config.electron_profile.canonicalize()?,
    )
}
