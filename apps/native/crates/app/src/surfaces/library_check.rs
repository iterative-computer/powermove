//! Disposable Linux harness for the exact app module (no copied source).
//!
//! From the repository root:
//! rustc --edition=2024 apps/native/crates/app/src/surfaces/library_check.rs -o work/library-check
//! work/library-check prepare
//! work/library-check test --locked
//! work/library-check clippy --locked --all-targets -- -D warnings
//! work/library-check fmt --check
//!
//! The app binary has unconditional Metal/GPUI/AppKit dependencies. This runner
//! creates only scratch files under work/, seeds the workspace lock (retaining
//! its resolved versions), and tests the actual registered module. It is not a
//! replacement app target or a new workspace member. Tests always require
//! --locked; prepare is a separate dependency-resolution step.
use std::{
    collections::BTreeSet,
    env, fs,
    path::Path,
    process::{Command, Stdio},
};

fn pins(text: &str) -> BTreeSet<Vec<&str>> {
    text.split("[[package]]")
        .skip(1)
        .filter_map(|block| {
            let pin: Vec<_> = block
                .lines()
                .filter(|line| {
                    ["name = ", "version = ", "source = ", "checksum = "]
                        .iter()
                        .any(|key| line.starts_with(key))
                })
                .collect();
            (!pin.contains(&"name = \"pm-library-headless\"")).then_some(pin)
        })
        .collect()
}

fn path_string(path: &Path) -> String {
    format!(
        "\"{}\"",
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    )
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?.canonicalize()?;
    let source = root.join("apps/native/crates/app/src/surfaces/library.rs");
    if !source.is_file() {
        return Err("Run from the Powermove repository root".into());
    }
    let mut args = env::args().skip(1);
    let action = args
        .next()
        .ok_or("Expected prepare, test, clippy, fmt or check")?;
    let rest: Vec<_> = args.collect();
    let scratch = root.join("work/library-headless");
    let manifest = scratch.join("Cargo.toml");
    if action == "prepare" {
        fs::create_dir_all(&scratch)?;
        let dependency = |name: &str, path: &str| {
            format!("{name} = {{ path = {} }}\n", path_string(&root.join(path)))
        };
        let mut text = format!(
            "[workspace]\n[package]\nname = \"pm-library-headless\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\npath = {}\n[dependencies]\n",
            path_string(&source)
        );
        text += &dependency("pm-kernel", "apps/native/crates/kernel");
        text += &dependency("pm-design", "apps/native/crates/design");
        text += &dependency("pm-model", "engine/crates/pm-model");
        text += "serde_json = \"1\"\n[dev-dependencies]\ntempfile = \"3\"\n[target.'cfg(target_os = \"macos\")'.dependencies]\nobjc2 = \"0.6\"\nobjc2-app-kit = \"0.3\"\nobjc2-foundation = \"0.3\"\nblock2 = \"0.6\"\nasync-channel = \"2\"\n";
        fs::write(&manifest, text)?;
        fs::copy(
            root.join("apps/native/Cargo.lock"),
            scratch.join("Cargo.lock"),
        )?;
        // Resolve only this scratch root using the workspace's existing pins.
        // No workspace manifest or lockfile is changed and no bulk update runs.
        let status = Command::new("cargo")
            .args(["metadata", "--format-version", "1"])
            .arg("--manifest-path")
            .arg(&manifest)
            .stdout(Stdio::null())
            .status()?;
        if !status.success() {
            return Err("Harness dependency preparation failed".into());
        }
        let workspace_lock = fs::read_to_string(root.join("apps/native/Cargo.lock"))?;
        let harness_lock = fs::read_to_string(scratch.join("Cargo.lock"))?;
        if !pins(&harness_lock).is_subset(&pins(&workspace_lock)) {
            return Err("Harness dependency pins differ from the workspace lock; update only the required package before preparing again".into());
        }
        return Ok(());
    }
    if !["test", "clippy", "fmt", "check"].contains(&action.as_str()) {
        return Err("Unknown check command".into());
    }
    if !manifest.exists() {
        return Err("Run work/library-check prepare first".into());
    }
    if action != "fmt" && !rest.iter().any(|arg| arg == "--locked") {
        return Err("Checks must pass --locked".into());
    }
    let status = Command::new("cargo")
        .arg(&action)
        .arg("--manifest-path")
        .arg(manifest)
        .args(rest)
        .status()?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
