fn main() {
    println!("cargo:rerun-if-env-changed=PM_FFMPEG_DIR");
    if let Some(root) = std::env::var_os("PM_FFMPEG_DIR") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", std::path::Path::new(&root).join("lib").display());
    }
}
