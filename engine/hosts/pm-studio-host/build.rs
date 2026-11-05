// Final binaries, unlike dependency rlibs, own their runtime search paths.
fn main() {
    if let Some(dir) = std::env::var_os("DEP_PM_STUDIO_OIDN_LIB") {
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            std::path::Path::new(&dir).display()
        );
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        }
    }
}
