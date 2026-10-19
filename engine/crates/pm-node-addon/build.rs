fn main() {
    if std::env::var_os("CARGO_FEATURE_NODE").is_some() {
        // `-undefined dynamic_lookup` for the cdylib: Node's symbols
        // resolve when Node loads the addon.
        napi_build::setup();
    } else {
        println!(
            "cargo:warning=pm-node-addon built without `--features node`: the dylib has no Node-API exports (napi_register_module_v1). The app's addon is built by apps/desktop/scripts/build-engine-addon.mjs."
        );
    }
    // The renderer links Powermove's FFmpeg (pm-media). The addon finds the
    // dylibs beside the app (`Contents/Resources/ffmpeg/lib`, two levels up
    // from `app.asar.unpacked/native`) or, in development, in the build
    // pm-media linked against.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path/../../ffmpeg/lib");
        if let Some(dir) = std::env::var_os("DEP_PM_MEDIA_NATIVE_FFMPEG_LIB") {
            println!(
                "cargo:rustc-link-arg=-Wl,-rpath,{}",
                std::path::Path::new(&dir).display()
            );
        }
    }
}
