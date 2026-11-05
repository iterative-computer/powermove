//! Linux verification runner for the app-owned dialog module. The production
//! app has unconditional GPUI/Metal dependencies and cannot be built on Linux.
//! This adds no workspace package/dependency and changes no checked-in lockfile.
//!
//! From the repository root:
//! `mkdir -p work && rustc --edition 2024 apps/native/crates/app/src/surfaces/dialogs_check.rs -o work/check-dialogs`
//! `work/check-dialogs test` (or clippy / fmt / bootstrap)
//!
//! The generated work/dialogs-check manifest points at the actual owned source,
//! kernel and design crates. Its lock starts with the checked-in native lock;
//! metadata prunes unused packages and adds only the harness root.
//! Every test/check uses --locked. No spikes, vendor fixtures, home paths,
//! symlink-ownership assumptions, platform tools or real user data are used.
use std::{error::Error, fs, path::PathBuf, process::Command};

fn run(command: &mut Command) -> Result<(), Box<dyn Error>> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("dialog verification failed: {status}").into());
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let action = std::env::args().nth(1).unwrap_or_else(|| "test".into());
    if !["test", "clippy", "fmt", "bootstrap"].contains(&action.as_str()) {
        return Err("usage: check-dialogs [test|clippy|fmt|bootstrap]".into());
    }
    let here = std::env::current_dir()?.canonicalize()?;
    let root = here
        .ancestors()
        .find(|p| {
            p.join("apps/native/crates/app/src/surfaces/dialogs.rs")
                .is_file()
        })
        .ok_or("run inside the Powermove checkout")?;
    let dir = root.join("work/dialogs-check");
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join("Cargo.toml"),
        r#"[package]
name = "pm-dialogs-check"
version = "0.1.0"
edition = "2024"
[workspace]
[lib]
path = "../../apps/native/crates/app/src/surfaces/dialogs.rs"
[[bin]]
name = "pm-dialogs-check-runner"
path = "../../apps/native/crates/app/src/surfaces/dialogs_check.rs"
[dependencies]
pm-kernel = { path = "../../apps/native/crates/kernel" }
pm-design = { path = "../../apps/native/crates/design" }
pm-model = { path = "../../engine/crates/pm-model" }
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-app-kit = "0.3"
objc2-foundation = "0.3"
block2 = "0.6"
"#,
    )?;
    fs::copy(root.join("apps/native/Cargo.lock"), dir.join("Cargo.lock"))?;
    let manifest: PathBuf = dir.join("Cargo.toml");
    let metadata = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&manifest)
        .output()?;
    if !metadata.status.success() {
        return Err(format!(
            "lock preparation failed: {}",
            String::from_utf8_lossy(&metadata.stderr)
        )
        .into());
    }
    match action.as_str() {
        "bootstrap" => {}
        "test" => run(Command::new("cargo")
            .args(["test", "--locked", "--manifest-path"])
            .arg(&manifest))?,
        "clippy" => run(Command::new("cargo")
            .args(["clippy", "--locked", "--manifest-path"])
            .arg(&manifest)
            .args(["--all-targets", "--", "-D", "warnings"]))?,
        "fmt" => {
            run(Command::new("cargo")
                .args(["fmt", "--manifest-path"])
                .arg(&manifest)
                .arg("--check"))?;
        }
        _ => unreachable!("validated"),
    }
    Ok(())
}
