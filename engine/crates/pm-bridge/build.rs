fn main() {
    // Node's symbols resolve when Node loads the addon.
    if std::env::var_os("CARGO_FEATURE_NODE").is_some() {
        napi_build::setup();
    }
}
