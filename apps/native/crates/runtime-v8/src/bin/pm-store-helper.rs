fn main() {
    if let Err(error) = pm_runtime_v8::run_store_helper(&pm_runtime_v8::PlatformStoreSandbox) {
        eprintln!("Store helper: {error}");
        std::process::exit(1);
    }
}
