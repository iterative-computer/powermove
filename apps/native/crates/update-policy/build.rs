fn main() {
    println!("cargo:rerun-if-env-changed=PM_SPARKLE_FRAMEWORK_DIR");
    println!("cargo:rerun-if-changed=native/sparkle.m");
    #[cfg(target_os = "macos")]
    if std::env::var_os("CARGO_FEATURE_MACOS_SPARKLE").is_some() {
        let root = std::env::var("PM_SPARKLE_FRAMEWORK_DIR")
            .expect("macos-sparkle requires the verified Sparkle 2.10.1 framework directory");
        cc::Build::new()
            .file("native/sparkle.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .flag("-mmacosx-version-min=14.0")
            .flag(format!("-F{root}"))
            .warnings_into_errors(true)
            .compile("pm_sparkle_bridge");
        println!("cargo:rustc-link-search=framework={root}");
        println!("cargo:rustc-link-lib=framework=Sparkle");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }
}
