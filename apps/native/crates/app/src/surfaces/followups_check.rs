//! Reproducible verification for all eight design-review follow-ups.
//! Run from repo root (Rust 1.96.1):
//! rustc --edition=2024 apps/native/crates/app/src/surfaces/followups_check.rs -o work/followups-check
//! work/followups-check prepare
//! work/followups-check test
//! work/followups-check clippy
//! work/followups-check check-macos
//! On a Mac: work/followups-check macos (real main-thread keyboard/control probe).
//! All tests/checks use --locked. Preparation only prunes a copy of the native
//! lock and adds this disposable root; it never updates a repository lock.
use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    path::Path,
    process::{Command, Stdio},
};
fn pins(lock: &str) -> BTreeSet<Vec<&str>> {
    lock.split("[[package]]")
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
            (!pin.contains(&"name = \"pm-native-surface-followups\"")).then_some(pin)
        })
        .collect()
}
fn main() -> Result<(), Box<dyn Error>> {
    let action = std::env::args()
        .nth(1)
        .ok_or("expected prepare/test/clippy/check-macos/macos/fmt")?;
    let root = std::env::current_dir()?.canonicalize()?;
    let sources = root.join("apps/native/crates/app/src/surfaces");
    if !sources.join("followups_check.rs").is_file() {
        return Err("run from the Powermove repository root".into());
    }
    let dir = root.join("work/followups");
    let manifest = dir.join("Cargo.toml");
    if action == "prepare" {
        fs::create_dir_all(&dir)?;
        let mut lib = String::new();
        for module in [
            "authority",
            "dialogs",
            "documents",
            "editing",
            "kit_native",
            "library",
            "media",
            "media_appkit",
            "media_kernel",
            "media_wire",
            "recovery",
            "recovery_kernel",
            "recovery_native",
            "recovery_wire",
        ] {
            if module == "media_appkit" {
                lib.push_str("#[cfg(target_os = \"macos\")]\n");
            }
            lib.push_str(&format!(
                "#[path = {:?}]\npub mod {module};\n",
                sources.join(format!("{module}.rs"))
            ));
        }
        fs::write(dir.join("lib.rs"), lib)?;
        fs::write(
            &manifest,
            r#"[workspace]
[package]
name = "pm-native-surface-followups"
version = "0.1.0"
edition = "2024"
[lib]
path = "lib.rs"
[dependencies]
pm-kernel = { path = "../../apps/native/crates/kernel" }
pm-design = { path = "../../apps/native/crates/design" }
pm-surface = { path = "../../apps/native/crates/surface" }
pm-model = { path = "../../engine/crates/pm-model" }
serde_json = "1"
[dev-dependencies]
tempfile = "3"
pm-services = { path = "../../apps/native/crates/services" }
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-app-kit = "0.3"
objc2-foundation = "0.3"
block2 = "0.6"
async-channel = "2"
[[test]]
name = "native-main-thread"
path = "../../apps/native/crates/app/src/surfaces/native_main_thread_check.rs"
harness = false
[profile.dev]
debug = 0
incremental = false
"#,
        )?;
        let base = fs::read_to_string(root.join("apps/native/Cargo.lock"))?;
        fs::write(dir.join("Cargo.lock"), &base)?;
        run(&manifest, &["metadata", "--format-version", "1"], true)?;
        let prepared = fs::read_to_string(dir.join("Cargo.lock"))?;
        if !pins(&prepared).is_subset(&pins(&base)) {
            return Err("scratch dependency pins differ from apps/native/Cargo.lock".into());
        }
        return Ok(());
    }
    if !manifest.is_file() {
        return Err("run prepare first".into());
    }
    let args: &[&str] = match action.as_str() {
        "test" => &["test", "--locked", "--lib"],
        "clippy" => &[
            "clippy",
            "--locked",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        "check-macos" => &[
            "clippy",
            "--locked",
            "--all-targets",
            "--target",
            "aarch64-apple-darwin",
            "--",
            "-D",
            "warnings",
        ],
        "macos" if cfg!(target_os = "macos") => {
            &["test", "--locked", "--test", "native-main-thread"]
        }
        "macos" => {
            return Err("macos requires a real Mac; cross compilation is check-macos".into());
        }
        "fmt" => &["fmt", "--check"],
        _ => return Err("unknown check command".into()),
    };
    run(&manifest, args, false)
}
fn run(manifest: &Path, args: &[&str], quiet: bool) -> Result<(), Box<dyn Error>> {
    let mut command = Command::new("cargo");
    // Cargo flags go before rustc/clippy's -- separator.
    command
        .arg(args[0])
        .arg("--manifest-path")
        .arg(manifest)
        .args(&args[1..]);
    if quiet {
        command.stdout(Stdio::null());
    }
    if !command.status()?.success() {
        return Err(format!("cargo {args:?} failed").into());
    }
    Ok(())
}
