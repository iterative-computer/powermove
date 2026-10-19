fn main() {
    // pm-node's tests and examples link Powermove's FFmpeg (pm-media) and
    // find its dylibs in the build pm-media linked against. The addon
    // (pm-node-addon) sets its own rpaths.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && let Some(dir) = std::env::var_os("DEP_PM_MEDIA_NATIVE_FFMPEG_LIB")
    {
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            std::path::Path::new(&dir).display()
        );
    }
}
