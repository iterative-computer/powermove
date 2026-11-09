// Links Open Image Denoise when the `oidn` feature is on.
fn main() {
    println!("cargo:rerun-if-env-changed=OIDN_DIR");
    if std::env::var_os("CARGO_FEATURE_METALFX").is_some() && std::env::var("CARGO_CFG_TARGET_OS").as_deref()==Ok("macos") {
        let out=std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        println!("cargo:rerun-if-changed=native/metalfx.mm");
        println!("cargo:rerun-if-changed=native/surface-message.h");
        assert!(std::process::Command::new("xcrun").args(["clang++","-std=c++17","-fobjc-arc","-mmacosx-version-min=13.0","-O2","-c","native/metalfx.mm","-o"]).arg(out.join("metalfx.o")).status().unwrap().success(),"MetalFX bridge compile failed");
        assert!(std::process::Command::new("ar").arg("rcs").arg(out.join("libstudio_metalfx.a")).arg(out.join("metalfx.o")).status().unwrap().success());
        println!("cargo:rustc-link-search=native={}",out.display());println!("cargo:rustc-link-lib=static=studio_metalfx");
        for framework in ["Foundation","Metal","MetalFX","IOSurface"] {println!("cargo:rustc-link-lib=framework={framework}");}
        println!("cargo:rustc-link-lib=c++");
    }
    if std::env::var_os("CARGO_FEATURE_OIDN").is_none() {
        return;
    }
    let dir = std::env::var("OIDN_DIR").unwrap_or_else(|_| "/opt/homebrew/opt/open-image-denoise".into());
    println!("cargo:rustc-link-search=native={dir}/lib");
    println!("cargo:rustc-link-lib=dylib=OpenImageDenoise");
    // Find the library at run time without DYLD_LIBRARY_PATH.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}/lib");
}
