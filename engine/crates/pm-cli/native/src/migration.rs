//! Dry-run means the actual first-launch importer runs against a private copy.
//! No Keychain accessor is constructed; secret consent remains a reported step.
use crate::{Error, Result, error, options::Options, project::new_destination};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

type Inventory = BTreeMap<PathBuf, (u64, u32, blake3::Hash)>;
fn inventory(root: &Path) -> Result<Inventory> {
    fn walk(root: &Path, relative: &Path, out: &mut Inventory) -> Result<()> {
        let path = root.join(relative);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(Error::invalid(
                "Profile copy refuses symlinks; provide a closed, ordinary profile snapshot",
            ));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path)? {
                walk(root, &relative.join(entry?.file_name()), out)?;
            }
        } else if metadata.is_file() {
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)?;
            let mut hasher = blake3::Hasher::new();
            let mut bytes = [0; 64 * 1024];
            loop {
                let n = file.read(&mut bytes)?;
                if n == 0 {
                    break;
                }
                hasher.update(&bytes[..n]);
            }
            out.insert(
                relative.into(),
                (
                    metadata.len(),
                    metadata.permissions().mode(),
                    hasher.finalize(),
                ),
            );
        } else {
            return Err(Error::invalid(
                "Profile contains a socket, device or other nonregular file",
            ));
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, Path::new(""), &mut out)?;
    Ok(out)
}
fn private_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}
use std::os::unix::fs::DirBuilderExt;
fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    private_dir(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        let destination = target.join(entry.file_name());
        if metadata.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else if metadata.is_file() {
            let mut input = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(entry.path())?;
            if !input.metadata()?.is_file() {
                return Err(Error::invalid("Profile changed while copying"));
            }
            let mode = if metadata.permissions().mode() & 0o111 != 0 {
                0o700
            } else {
                0o600
            };
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(mode)
                .open(destination)?;
            std::io::copy(&mut input, &mut output)?;
            output.sync_all()?;
        } else {
            return Err(Error::invalid("Profile changed or contains a symlink"));
        }
    }
    File::open(target)?.sync_all()?;
    Ok(())
}
pub fn dry_run(options: &Options) -> Result<Value> {
    let source = options.project.canonicalize()?;
    if !source.is_dir() {
        return Err(Error::invalid("Expected a profile directory"));
    }
    if pm_kernel::locks::older_electron_running(&source) {
        return Err(error(
            "profile_busy",
            "Quit Electron before taking the dry-run snapshot",
        ));
    }
    let scratch = new_destination(options.scratch.as_deref().expect("validated scratch"))?;
    if scratch.starts_with(&source) || source.starts_with(&scratch) {
        return Err(Error::invalid("Scratch and source profile overlap"));
    }
    let before = inventory(&source)?;
    let bytes = before
        .values()
        .try_fold(0u64, |total, (n, _, _)| total.checked_add(*n))
        .ok_or_else(|| Error::invalid("Profile size overflow"))?;
    let required = bytes
        .saturating_mul(4)
        .saturating_add(6 * 1024 * 1024 * 1024);
    if fs2::available_space(scratch.parent().expect("scratch parent"))? < required {
        return Err(error(
            "io_error",
            "Migration needs space for copies plus the first-launch 6 GiB reserve",
        ));
    }
    private_dir(&scratch)?;
    let copied = scratch.join("electron-copy");
    copy_tree(&source, &copied)?;
    let after = inventory(&source)?;
    if before != after {
        return Err(error(
            "profile_changed",
            "Source profile changed during copy; scratch is preserved, importer was not run",
        ));
    }
    let copied_inventory = inventory(&copied)?;
    if before.len() != copied_inventory.len()
        || before.iter().any(|(p, (n, _, hash))| {
            copied_inventory
                .get(p)
                .is_none_or(|(cn, _, ch)| n != cn || hash != ch)
        })
    {
        return Err(error(
            "copy_failed",
            "Profile copy does not match source; scratch is preserved",
        ));
    }
    let native = scratch.join(pm_first_launch::BETA_BUNDLE_ID);
    let config = pm_first_launch::Config {
        electron_profile: copied.clone(),
        native_profile: native.clone(),
    };
    let mut published = vec![];
    let outcome = pm_first_launch::run(&config, |event| {
        if let pm_first_launch::Event::Published(step) = event {
            published.push(step);
        }
        Ok(())
    })
    .map_err(|e| {
        error(
            "migration_error",
            format!("{e:#}; copied profile retained at {}", scratch.display()),
        )
    })?;
    // The original remains a read-only source even when the importer pauses.
    if before != inventory(&source)? {
        return Err(error(
            "profile_changed",
            "Original profile changed during dry-run; scratch preserved",
        ));
    }
    let (status, counts) = match outcome {
        pm_first_launch::Outcome::Complete {
            projects,
            history_refusals,
            missing_projects,
            blocked_owners,
            proxy_only,
            already_offline,
            not_reached,
        } => (
            "complete",
            json!({"projects":projects,"historyRefusals":history_refusals,"missingProjects":missing_projects,"blockedOwners":blocked_owners,"proxyOnly":proxy_only,"alreadyOffline":already_offline,"notReached":not_reached}),
        ),
        pm_first_launch::Outcome::NeedsSecretConsent { interrupted } => {
            ("needs_secret_consent", json!({"interrupted":interrupted}))
        }
        pm_first_launch::Outcome::NeedsFolderAccess { folders } => (
            "needs_folder_access",
            json!({"folders":folders.len(),"originals":folders.iter().map(|f| f.1).sum::<usize>()}),
        ),
        pm_first_launch::Outcome::WaitingForElectron => ("waiting_for_electron", json!({})),
    };
    let result = json!({"dryRun":true,"status":status,"sourceUnchanged":true,"copiedProfile":copied,"nativeProfile":native,"sourceFiles":before.len(),"sourceBytes":bytes,"published":published,"counts":counts,"keychainAccessed":false});
    let mut report = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(scratch.join("cli-dry-run.json"))?;
    report.write_all(&serde_json::to_vec(&result).expect("report JSON"))?;
    report.sync_all()?;
    File::open(&scratch)?.sync_all()?;
    Ok(result)
}
