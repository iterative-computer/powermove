use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/import/modelio.mm");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = out.join("modelio.o");
    let library = out.join("libpm_scene3d_modelio.a");
    let target = env::var("TARGET").unwrap();
    let deployment = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "13.0".into());
    let status = Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "clang++",
            "-target",
            &target,
            "-std=c++17",
            "-fobjc-arc",
            "-O2",
        ])
        .arg(format!("-mmacosx-version-min={deployment}"))
        .args(["-c", "src/import/modelio.mm", "-o"])
        .arg(&object)
        .status()
        .expect("start Model I/O bridge compiler");
    assert!(status.success(), "Model I/O bridge compilation failed");
    let status = Command::new("xcrun")
        .args(["ar", "rcs"])
        .arg(&library)
        .arg(&object)
        .status()
        .expect("start Model I/O bridge archiver");
    assert!(status.success(), "Model I/O bridge archive failed");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=pm_scene3d_modelio");
    for framework in ["ModelIO", "Foundation", "CoreGraphics"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rustc-link-lib=c++");
}
