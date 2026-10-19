fn main() {
    if std::env::var_os("CARGO_FEATURE_NODE").is_some() {
        // `-undefined dynamic_lookup` for the cdylib: Node's symbols
        // resolve when Node loads the addon.
        napi_build::setup();
    } else {
        println!(
            "cargo:warning=pm-bridge-addon built without `--features node`: the dylib has no Node-API exports (napi_register_module_v1). The app's addon is built by apps/desktop/scripts/build-engine-addon.mjs."
        );
    }
}
