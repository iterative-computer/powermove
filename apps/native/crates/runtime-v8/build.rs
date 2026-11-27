fn main() {
    println!("cargo:rerun-if-env-changed=V8_FROM_SOURCE");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        assert!(
            std::env::var_os("V8_FROM_SOURCE").is_none(),
            "Linux must use upstream V8 prebuilt archives, never a source build"
        );
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        assert!(
            std::env::var_os("CARGO_FEATURE_MACOS_SANDBOX").is_some(),
            "macOS builds require --features macos-sandbox and the pinned S1 archive/bindings"
        );
    }
}
