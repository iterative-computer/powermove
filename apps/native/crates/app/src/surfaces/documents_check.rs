//! Portable verification driver for the app's owned document modules.
//!
//! From the repository root:
//! `mkdir -p work`
//! `rustc --edition=2024 apps/native/crates/app/src/surfaces/documents_check.rs -o work/documents-check-tool`
//! `work/documents-check-tool` (tests, Clippy, formatting)
//! `work/documents-check-tool --check-macos` (macOS-target type check only)
//!
//! The app binary unconditionally links Metal/GPUI. This disposable harness
//! compiles the actual owned modules with the real kernel, not copied models.
//! It seeds its lock from apps/native/Cargo.lock and adds only the harness root.
//! Repository manifests/lockfiles are never written. No spike/vendor fixtures.
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};
fn run(root: &Path, args: &[&str], quiet: bool) -> io::Result<()> {
    let mut command = Command::new("cargo");
    command
        .current_dir(root.join("apps/native"))
        .env("CARGO_TARGET_DIR", root.join("apps/native/target"))
        .args(args);
    if quiet {
        command.stdout(std::process::Stdio::null());
    }
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "cargo {} failed: {status}",
            args.join(" ")
        )))
    }
}
fn check() -> io::Result<()> {
    let mut root = env::current_dir()?.canonicalize()?;
    while !root
        .join("apps/native/crates/app/src/surfaces/documents.rs")
        .is_file()
    {
        if !root.pop() {
            return Err(io::Error::other("run inside the Powermove repository"));
        }
    }
    let scratch = root.join("work/documents-check");
    fs::create_dir_all(&scratch)?;
    let manifest = r#"[package]
name = "pm-documents-check"
version = "0.1.0"
edition = "2024"
[workspace]
[lib]
path = "../../apps/native/crates/app/src/surfaces/documents.rs"
[dependencies]
pm-kernel = { path = "../../apps/native/crates/kernel" }
pm-model = { path = "../../engine/crates/pm-model" }
[[test]]
name = "service_contract"
path = "../../apps/native/crates/app/src/surfaces/documents_services_tests.rs"
[dev-dependencies]
pm-services = { path = "../../apps/native/crates/services" }
tempfile = "3"
serde_json = "1"
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-app-kit = "0.3"
objc2-foundation = "0.3"
block2 = "0.6"
pm-design = { path = "../../apps/native/crates/design" }
[profile.dev]
debug = 0
incremental = false
[profile.test]
debug = 0
incremental = false
"#;
    let path = scratch.join("Cargo.toml");
    if fs::read_to_string(&path).ok().as_deref() != Some(manifest)
        || !scratch.join("Cargo.lock").is_file()
    {
        fs::write(&path, manifest)?;
        fs::copy(
            root.join("apps/native/Cargo.lock"),
            scratch.join("Cargo.lock"),
        )?;
        run(
            &root,
            &[
                "metadata",
                "--manifest-path",
                path.to_str()
                    .ok_or_else(|| io::Error::other("non-UTF8 checkout path"))?,
                "--format-version",
                "1",
            ],
            true,
        )?;
    }
    let path = path
        .to_str()
        .ok_or_else(|| io::Error::other("non-UTF8 checkout path"))?;
    if env::args().any(|a| a == "--check-macos") {
        run(
            &root,
            &[
                "check",
                "--locked",
                "--manifest-path",
                path,
                "--target",
                "x86_64-apple-darwin",
            ],
            false,
        )?;
        run(
            &root,
            &[
                "clippy",
                "--locked",
                "--manifest-path",
                path,
                "--target",
                "x86_64-apple-darwin",
                "--",
                "-D",
                "warnings",
            ],
            false,
        )?;
    } else {
        run(&root, &["test", "--locked", "--manifest-path", path], false)?;
        run(
            &root,
            &[
                "clippy",
                "--locked",
                "--manifest-path",
                path,
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
            false,
        )?;
        run(&root, &["fmt", "--manifest-path", path, "--check"], false)?;
        // The driver is a binary source rather than a module of the app.
        let driver: PathBuf = root.join("apps/native/crates/app/src/surfaces/documents_check.rs");
        let status = Command::new("rustfmt")
            .current_dir(&root)
            .args(["--edition", "2024", "--check"])
            .arg(driver)
            .status()?;
        if !status.success() {
            return Err(io::Error::other("driver formatting failed"));
        }
    }
    Ok(())
}
fn main() -> ExitCode {
    match check() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
