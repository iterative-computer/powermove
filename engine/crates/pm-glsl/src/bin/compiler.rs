fn main() {
    if !std::env::args().any(|a| a == "--glsl-compiler") {
        eprintln!("pm-glsl-compiler is an internal worker; use --glsl-compiler");
        std::process::exit(2);
    }
    if let Err(e) = pm_glsl::compiler::worker_main() {
        eprintln!("compiler worker: {e}");
        std::process::exit(1);
    }
}
