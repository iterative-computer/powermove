fn main() {
    println!("cargo:rerun-if-env-changed=PM_SPARKLE_FRAMEWORK");
    println!("cargo:rerun-if-changed=native/sparkle.m");
    if std::env::var_os("CARGO_FEATURE_SPARKLE_MACOS").is_none() {
        return;
    }
    assert_eq!(
        std::env::var("CARGO_CFG_TARGET_OS").unwrap(),
        "macos",
        "Sparkle binding requires macOS"
    );
    let framework = std::path::PathBuf::from(
        std::env::var_os("PM_SPARKLE_FRAMEWORK")
            .expect("set PM_SPARKLE_FRAMEWORK to verified Sparkle.framework"),
    );
    assert_eq!(
        framework.file_name().and_then(|x| x.to_str()),
        Some("Sparkle.framework")
    );
    let framework = framework
        .canonicalize()
        .expect("Sparkle framework is missing");
    let info = std::process::Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleShortVersionString"])
        .arg(framework.join("Resources/Info.plist"))
        .output()
        .expect("read Sparkle pin");
    assert!(
        info.status.success() && info.stdout == b"2.10.1\n",
        "Sparkle framework version differs from pin; verify upstream archive first"
    );
    let parent = framework.parent().unwrap();
    cc::Build::new()
        .file("native/sparkle.m")
        .flag("-fobjc-arc")
        .flag("-fblocks")
        .flag("-mmacosx-version-min=14.0")
        .flag("-F")
        .flag(parent.to_str().unwrap())
        .warnings_into_errors(true)
        .compile("pm_sparkle_bridge");
    println!("cargo:rustc-link-search=framework={}", parent.display());
    for framework in ["Foundation", "Sparkle", "Security"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
}
