//! Standalone harness setup: run from the repository root (see surfaces.md).
//! Writes only disposable work/ manifests/lockfiles. No app dependencies change.
use std::{
    error::Error,
    fs,
    path::Path,
    process::{Command, Stdio},
};

fn metadata(manifest: &Path) -> Result<(), Box<dyn Error>> {
    let status = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(manifest)
        .stdout(Stdio::null())
        .status()?;
    if !status.success() {
        return Err("editing harness metadata failed".into());
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let root = std::env::current_dir()?.canonicalize()?;
    let sources = root.join("apps/native/crates/app/src/surfaces");
    if !sources.join("editing.rs").is_file() {
        return Err("run the setup from the Powermove repository root".into());
    }
    let native = std::env::args().any(|a| a == "--native");
    let name = if native {
        "pm-editing-native-check"
    } else {
        "pm-editing-check"
    };
    let dir = root.join(if native {
        "work/editing-native-check"
    } else {
        "work/editing-check"
    });
    fs::create_dir_all(&dir)?;
    let lib = if native {
        let mut lib = String::new();
        for (module, file) in [
            ("colour", "editing_colour.rs"),
            ("text", "editing_text.rs"),
            ("host", "editing_host.rs"),
            ("native", "editing_native.rs"),
        ] {
            lib.push_str(&format!(
                "#[path = {:?}]\npub mod {module};\n",
                sources.join(file)
            ));
        }
        lib.push_str("pub use host::{EditingHost, InspectHost};\n");
        fs::write(dir.join("lib.rs"), lib)?;
        "lib.rs"
    } else {
        "../../apps/native/crates/app/src/surfaces/editing.rs"
    };
    let mut manifest = format!(
        r#"[workspace]
[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
[lib]
path = "{lib}"
[dependencies]
pm-design = {{ path = "../../apps/native/crates/design" }}
"#
    );
    if !native {
        manifest.push_str("pm-kernel = { path = \"../../apps/native/crates/kernel\" }\npm-model = { path = \"../../engine/crates/pm-model\" }\n");
    }
    manifest.push_str(
        r#"[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-app-kit = "0.3"
objc2-foundation = "0.3"
block2 = "0.6"
[profile.dev]
debug = 0
incremental = false
"#,
    );
    fs::write(dir.join("Cargo.toml"), manifest)?;
    if !dir.join("Cargo.lock").exists() {
        fs::copy(root.join("apps/native/Cargo.lock"), dir.join("Cargo.lock"))?;
    }
    metadata(&dir.join("Cargo.toml"))?;
    println!("Prepared {}", dir.strip_prefix(root)?.display());
    Ok(())
}
