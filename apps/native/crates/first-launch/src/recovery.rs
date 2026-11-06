//! Restore a sealed migration checkpoint into a NEW private directory for
//! inspection/resume. This never replaces a profile or converts native work back
//! to Electron. No Restore UI or reverse migration is exposed.
use crate::{Checkpoint, disk};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

/// The hook runs after each durable file copy for interruption proof. An error
/// leaves the entire partial stage; the next attempt archives it before retry.
pub fn restore_checkpoint(
    source: &Path,
    destination: &Path,
    mut hook: impl FnMut() -> Result<()>,
) -> Result<Checkpoint> {
    ensure!(
        source.is_absolute() && destination.is_absolute(),
        "checkpoint paths must be absolute"
    );
    let resolved_source = disk::resolved(source)?;
    let resolved_destination = disk::resolved(destination)?;
    ensure!(
        resolved_source == source && resolved_destination == destination,
        "checkpoint paths must be canonical and unlinked"
    );
    let source = resolved_source;
    let destination = resolved_destination;
    ensure!(
        !destination.starts_with(&source) && !source.starts_with(&destination),
        "checkpoint restore overlaps source"
    );
    let cp = Checkpoint::load(&source)?;
    ensure!(
        !destination.starts_with(&cp.source) && !cp.source.starts_with(&destination),
        "checkpoint restore overlaps original profile"
    );
    let parent = destination.parent().context("restore needs a parent")?;
    let parent_meta = fs::symlink_metadata(parent)?;
    ensure!(
        parent_meta.is_dir()
            && parent_meta.uid() == unsafe { libc::geteuid() }
            && parent_meta.mode() & 0o077 == 0,
        "restore parent must be an existing private directory owned by this user"
    );
    let name = destination
        .file_name()
        .context("restore needs a name")?
        .to_str()
        .context("invalid restore name")?;
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(parent.join(format!(".{name}.checkpoint-restore.lock")))?;
    lease
        .try_lock_exclusive()
        .context("checkpoint restore already running")?;
    ensure!(
        fs::symlink_metadata(&destination).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "restore destination already exists"
    );
    let stage = parent.join(format!(".{name}.checkpoint-restoring"));
    let marker = parent.join(format!(".{name}.checkpoint-stage.json"));
    let identity =
        serde_json::json!({"schema":1,"checkpoint":disk::hash(&source.join("checkpoint.json"))?.0});
    if marker.exists() {
        ensure!(
            disk::json::<serde_json::Value>(&marker)? == identity,
            "checkpoint stage belongs to another input"
        );
    } else {
        ensure!(!stage.exists(), "unowned checkpoint stage retained");
        disk::atomic(&marker, &identity)?;
    }
    if stage.exists() {
        ensure!(
            fs::symlink_metadata(&stage)?.is_dir(),
            "unsafe checkpoint stage"
        );
        disk::archive(&stage)?;
    }
    disk::space(parent, disk::tree_bytes(&source)?)?;
    disk::private_dir(&stage)?;
    let before = disk::walk(&source)?;
    let mut hashes = std::collections::BTreeMap::new();
    for relative in before.keys() {
        let input = source.join(relative);
        let output = stage.join(relative);
        disk::private_dir(output.parent().context("invalid checkpoint path")?)?;
        let copied = disk::copy(&input, &output)?;
        hashes.insert(relative.clone(), (copied.hash, copied.bytes));
        hook()?;
    }
    ensure!(
        disk::walk(&source)? == before,
        "checkpoint changed during restore"
    );
    for (relative, expected) in hashes {
        ensure!(
            disk::hash(&source.join(&relative))? == expected
                && disk::hash(&stage.join(relative))? == expected,
            "checkpoint restore verification failed"
        );
    }
    let restored = Checkpoint::load(&stage)?;
    // Check again under the sibling producer lease, never replace an old profile.
    ensure!(
        !destination.exists(),
        "restore destination appeared during copy"
    );
    publish_directory(&stage, &destination)?;
    File::open(parent)?.sync_all()?;
    Ok(restored)
}

/// Exclusive directory publication; a final-name race must never replace even
/// an empty foreign directory. Unsupported filesystems fail and retain staging.
fn publish_directory(source: &Path, destination: &Path) -> Result<()> {
    let source = std::ffi::CString::new(source.as_os_str().as_encoded_bytes())?;
    let destination = std::ffi::CString::new(destination.as_os_str().as_encoded_bytes())?;
    #[cfg(target_os = "macos")]
    let result = {
        unsafe extern "C" {
            fn renamex_np(
                from: *const libc::c_char,
                to: *const libc::c_char,
                flags: u32,
            ) -> libc::c_int;
        }
        // SAFETY: valid path strings, RENAME_EXCL refuses an existing name.
        unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), 4) }
    };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    anyhow::bail!("exclusive checkpoint directory publication unavailable");
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        ensure!(result == 0, std::io::Error::last_os_error());
        Ok(())
    }
}
