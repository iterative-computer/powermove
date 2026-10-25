fn main() {
    // Standalone probes need the same dylibs as pm-node's examples. Link
    // against the directory pm-media actually used, including custom builds.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && let Some(dir) = std::env::var_os("DEP_PM_MEDIA_NATIVE_FFMPEG_LIB")
    {
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            std::path::Path::new(&dir).display()
        );
    }
}
