//! Headless migration probe for isolated profile copies. Never opens a window.
use anyhow::{Context, Result, ensure};
use pm_first_launch::{Config, Outcome};
fn main() -> Result<()> {
    let config = Config {
        electron_profile: std::env::var_os("PM_NATIVE_ELECTRON_PROFILE")
            .context("source copy required")?
            .into(),
        native_profile: std::env::var_os("PM_NATIVE_PROFILE")
            .context("scratch destination required")?
            .into(),
    };
    let outcome = pm_first_launch::run(&config, |_| Ok(()))?;
    ensure!(
        matches!(outcome, Outcome::Complete { .. }),
        "migration did not complete"
    );
    let store = pm_kernel::LibraryStore::open(&config.native_profile);
    let enabled = store.read("extensions")?;
    let count = std::fs::read_dir(config.native_profile.join("extensions"))?.count();
    println!("Migration complete; {count} extension folders; enabled map: {enabled:?}");
    Ok(())
}
