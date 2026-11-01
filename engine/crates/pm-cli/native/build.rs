fn main() {
    if let Some(dir) = std::env::var_os("DEP_PM_MEDIA_NATIVE_FFMPEG_LIB") {
        println!(
            "cargo:rustc-link-arg-bins=-Wl,-rpath,{}",
            std::path::Path::new(&dir).display()
        );
    }
}
