fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-mmacosx-version-min=14.0");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        // Link arguments on pm-studio itself do not propagate through every
        // intermediate Rust crate to this executable. Keep an explicit dev
        // OIDN install discoverable; packaging relocates it to Frameworks.
        println!("cargo:rerun-if-env-changed=OIDN_DIR");
        if let Some(dir) = std::env::var_os("OIDN_DIR") {
            println!(
                "cargo:rustc-link-arg=-Wl,-rpath,{}",
                std::path::PathBuf::from(dir).join("lib").display()
            );
        }
        println!("cargo:rerun-if-env-changed=PM_FFMPEG_DIR");
        if let Some(dir) = std::env::var_os("PM_FFMPEG_DIR") {
            println!(
                "cargo:rustc-link-arg=-Wl,-rpath,{}",
                std::path::PathBuf::from(dir).join("lib").display()
            );
        }
    }
}
