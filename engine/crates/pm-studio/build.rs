// Optional native OIDN 2.x. No download or package-manager execution at build time.
fn main() {
    println!("cargo:rerun-if-env-changed=OIDN_DIR");
    if std::env::var_os("CARGO_FEATURE_OIDN").is_none() {
        return;
    }
    let dir = std::env::var_os("OIDN_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
                "/opt/homebrew/opt/open-image-denoise".into()
            } else {
                panic!("oidn feature requires OIDN_DIR pointing to an OIDN 2.x installation");
            }
        });
    let lib = dir.join("lib");
    assert!(lib.is_dir(), "OIDN_DIR/lib does not exist");
    println!("cargo::metadata=oidn_lib={}", lib.display());
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=OpenImageDenoise");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
}
