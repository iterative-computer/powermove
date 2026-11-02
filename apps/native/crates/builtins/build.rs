//! Bundle authored JSX at build time. The application embeds the result; there
//! is no package manager, Node loader or source-directory lookup at runtime.
use std::{env, path::PathBuf, process::Command};
fn main() {
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../js/v4");
    println!("cargo:rerun-if-changed={}", source.join("src").display());
    println!("cargo:rerun-if-changed={}", source.join("panels").display());
    println!("cargo:rerun-if-changed={}", source.join("app").display());
    println!("cargo:rerun-if-changed={}", source.join("bun.lock").display());
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("builtins.js");
    let status = Command::new("bun")
        .current_dir(&source)
        .args(["build", "app/entry.tsx", "--target=browser", "--format=iife"])
        .arg("--outfile").arg(output)
        .status().expect("bun is required to build the embedded native panels");
    assert!(status.success(), "native v4 panel bundle failed; install js/v4 dependencies first");
}
