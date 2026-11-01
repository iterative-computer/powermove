fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-mmacosx-version-min=14.0");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        println!("cargo:rerun-if-env-changed=PM_FFMPEG_DIR");
        if let Some(dir) = std::env::var_os("PM_FFMPEG_DIR") {
            println!(
                "cargo:rustc-link-arg=-Wl,-rpath,{}",
                std::path::PathBuf::from(dir).join("lib").display()
            );
        }
    }
}
