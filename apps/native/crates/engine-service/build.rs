fn main() {
    if let Some(dir) = std::env::var_os("DEP_PM_STUDIO_OIDN_LIB") {
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            std::path::Path::new(&dir).display()
        );
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    }
    if let Some(dir) = std::env::var_os("DEP_PM_MEDIA_NATIVE_FFMPEG_LIB") {
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            std::path::Path::new(&dir).display()
        );
    }
}
