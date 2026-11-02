//! Portable Linux verification for the app-owned recovery modules.
//!
//! From the repository root:
//! rustc --edition=2024 apps/native/crates/app/src/surfaces/recovery_check.rs -o work/recovery-check
//! work/recovery-check test
//! work/recovery-check clippy
//!
//! Uses the real kernel/design and this exact source, never spike fixtures. The
//! disposable workspace stays under work/. Shared manifests/locks are untouched;
//! Cargo metadata registers the harness in a copy of the shared lock.
use std::{error::Error, fs, path::PathBuf, process::Command};
fn run(command: &mut Command) -> Result<(), Box<dyn Error>> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("command failed: {status}").into());
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let operation = std::env::args().nth(1).unwrap_or_else(|| "test".into());
    if !matches!(operation.as_str(), "test" | "clippy") {
        return Err("use test or clippy".into());
    }
    let root = std::env::current_dir()?
        .ancestors()
        .find(|p| {
            p.join("apps/native/crates/app/src/surfaces/recovery.rs")
                .is_file()
        })
        .map(PathBuf::from)
        .ok_or("run inside the Powermove repository")?;
    let dir = root.join("work/recovery");
    fs::create_dir_all(&dir)?;
    let manifest = r#"[package]
name = "pm-recovery-surfaces"
version = "0.1.0"
edition = "2024"
publish = false
[lib]
path = "../../apps/native/crates/app/src/surfaces/mod.rs"
[dependencies]
pm-kernel = { path = "../../apps/native/crates/kernel" }
pm-design = { path = "../../apps/native/crates/design" }
serde_json = "1"
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-app-kit = "0.3"
objc2-foundation = "0.3"
async-channel = "2"
block2 = "0.6"
[dev-dependencies]
pm-services = { path = "../../apps/native/crates/services" }
pm-store-host = { path = "../../apps/native/crates/store-host" }
[[test]]
name = "recovery_source_contract"
path = "../../apps/native/crates/app/src/surfaces/recovery_source_tests.rs"
[workspace]
"#;
    let manifest_path = dir.join("Cargo.toml");
    let shared_lock = fs::read(root.join("apps/native/Cargo.lock"))?;
    if fs::read_to_string(&manifest_path).ok().as_deref() != Some(manifest)
        || !fs::read_to_string(dir.join("Cargo.lock"))
            .unwrap_or_default()
            .contains("name = \"pm-recovery-surfaces\"")
        || fs::read(dir.join("base.Cargo.lock")).ok().as_deref() != Some(shared_lock.as_slice())
    {
        fs::write(&manifest_path, manifest)?;
        fs::copy(root.join("apps/native/Cargo.lock"), dir.join("Cargo.lock"))?;
        run(Command::new("cargo")
            .current_dir(root.join("apps/native"))
            .arg("metadata")
            .arg("--manifest-path")
            .arg(&manifest_path)
            .args(["--format-version", "1"])
            .stdout(std::process::Stdio::null()))?;
        fs::write(dir.join("base.Cargo.lock"), shared_lock)?;
    }
    let mut command = Command::new("cargo");
    command
        .current_dir(root.join("apps/native"))
        .arg(&operation)
        .arg("--locked")
        .arg("--manifest-path")
        .arg(manifest_path);
    if operation == "clippy" {
        command.args(["--all-targets", "--no-deps", "--", "-D", "warnings"]);
    }
    run(&mut command)
}
