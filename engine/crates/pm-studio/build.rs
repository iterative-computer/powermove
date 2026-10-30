// Links Open Image Denoise when the `oidn` feature is on.
fn main() {
    println!("cargo:rerun-if-env-changed=OIDN_DIR");
    if std::env::var_os("CARGO_FEATURE_OIDN").is_none() {
        return;
    }
    let dir = std::env::var("OIDN_DIR").unwrap_or_else(|_| "/opt/homebrew/opt/open-image-denoise".into());
    println!("cargo:rustc-link-search=native={dir}/lib");
    println!("cargo:rustc-link-lib=dylib=OpenImageDenoise");
    // Find the library at run time without DYLD_LIBRARY_PATH.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}/lib");
}
