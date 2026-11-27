//! Bundle the realm at build time. The application embeds the result; there is
//! no package manager, Node loader or source-directory lookup at runtime.
use std::{env, path::PathBuf, process::Command};
fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../js");
    // Everything build-app.ts reads, including the theme modules native-theme.ts imports.
    for input in [
        "compat/src",
        "compat/scripts",
        "compat/package.json",
        "compat/bun.lock",
        "remote-dom/src",
        "remote-dom/package.json",
        "remote-dom/bun.lock",
        "themes",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(input).display());
    }
    let output = env::var_os("OUT_DIR").unwrap();
    let status = Command::new("bun")
        .current_dir(root.join("compat"))
        .args(["scripts/build-app.ts"])
        .arg(output)
        .status()
        .expect("bun is required to build the embedded compatibility realm");
    assert!(
        status.success(),
        "compat realm bundle failed; install js/compat and js/remote-dom dependencies first"
    );
}
