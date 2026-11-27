//! Inspect an isolated migrated profile without opening an app window.
use anyhow::{Context, Result, ensure};

fn main() -> Result<()> {
    let profile = std::path::PathBuf::from(
        std::env::var_os("PM_NATIVE_PROFILE").context("scratch profile required")?,
    );
    let found = pm_compat_runner::discovery::scan(&[
        profile.join("extensions-shared"),
        profile.join("extensions"),
    ]);
    let enabled = pm_kernel::LibraryStore::open(&profile)
        .read("extensions")?
        .context("extension settings missing")?;
    for id in std::env::args().skip(1) {
        let extension = found
            .iter()
            .find(|entry| entry.id == id)
            .with_context(|| format!("{id}: not discovered"))?;
        ensure!(extension.manifest.is_ok(), "{id}: invalid manifest");
        ensure!(
            extension.directory == profile.join("extensions").join(&id),
            "{id}: active folder missing"
        );
        ensure!(
            enabled.get(&id).and_then(|value| value.as_bool()) == Some(true),
            "{id}: not enabled"
        );
        println!(
            "{id}: landed=true enabled=true discovered=true store={}",
            extension.store.is_some()
        );
    }
    Ok(())
}
