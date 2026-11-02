//! Portable verification runner for this app-owned module. No production crate
//! or workspace dependency changes. The app itself unconditionally links AppKit
//! and Metal, so Linux cannot run `cargo test -p pm-native-app`.
//!
//! From the repository root (after sourcing /workspace/.tools/activate.sh):
//! ```text
//! mkdir -p work
//! rustc --edition=2024 apps/native/crates/app/src/surfaces/authority_check.rs -o work/authority-check-runner
//! work/authority-check-runner
//! work/authority-check-runner --macos
//! ```
//! --macos adds compile/clippy checks for aarch64-apple-darwin. Install that
//! standard-library target first. It does not run or visually certify AppKit.
use std::{error::Error, fs, path::Path, process::Command};

fn cargo(manifest: &Path, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let status = Command::new("cargo")
        .args(args)
        .arg("--manifest-path")
        .arg(manifest)
        .status()?;
    if !status.success() {
        return Err(format!("cargo {args:?} failed: {status}").into());
    }
    Ok(())
}
fn clippy(manifest: &Path, target: Option<&str>) -> Result<(), Box<dyn Error>> {
    let mut command = Command::new("cargo");
    command.args(["clippy", "--locked", "--all-targets", "--manifest-path"]);
    command.arg(manifest);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let status = command.args(["--", "-D", "warnings"]).status()?;
    if !status.success() {
        return Err(format!("clippy failed: {status}").into());
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    // Canonicalize instead of rejecting system-owned macOS /var or /tmp links.
    let source = Path::new(file!()).canonicalize()?;
    let root = std::env::current_dir()?.canonicalize()?;
    let work = root.join("work/authority-check");
    fs::create_dir_all(&work)?;
    let library = source
        .parent()
        .ok_or("source directory")?
        .join("authority.rs");
    let manifest = work.join("Cargo.toml");
    // These are the app's exact current objc2 pins. Only this transient package
    // is resolved; never cargo update the shared native/engine workspace.
    let text = format!(
        "[package]\nname = \"pm-authority-check\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
         [workspace]\n[lib]\npath = {:?}\n\
         [target.'cfg(target_os = \"macos\")'.dependencies]\n\
         objc2 = \"=0.6.5\"\nobjc2-app-kit = \"=0.3.2\"\n\
         objc2-foundation = \"=0.3.2\"\nblock2 = \"=0.6.2\"\n",
        library.to_str().ok_or("UTF-8 source path")?
    );
    fs::write(&manifest, text)?;
    if !work.join("Cargo.lock").exists() {
        cargo(&manifest, &["generate-lockfile"])?;
    }
    cargo(&manifest, &["fmt", "--check"])?;
    cargo(&manifest, &["test", "--locked"])?;
    clippy(&manifest, None)?;
    if std::env::args().any(|a| a == "--macos") {
        cargo(
            &manifest,
            &[
                "check",
                "--locked",
                "--all-targets",
                "--target",
                "aarch64-apple-darwin",
            ],
        )?;
        clippy(&manifest, Some("aarch64-apple-darwin"))?;
    }
    Ok(())
}
