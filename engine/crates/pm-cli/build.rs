fn main() {
    // pm-cli links Powermove's FFmpeg through pm-media: find the dylibs in the
    // build pm-media linked against.
    if let Some(dir) = std::env::var_os("DEP_PM_MEDIA_NATIVE_FFMPEG_LIB") {
        println!(
            "cargo:rustc-link-arg-bins=-Wl,-rpath,{}",
            std::path::Path::new(&dir).display()
        );
    }
}
