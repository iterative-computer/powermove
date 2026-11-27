//! Compiles the C shim over the vendored Signalsmith Stretch headers.
//!
//! The library's portable FFT is used on every platform (no Accelerate,
//! IPP, or PFFFT backend), so the same input stretches to the same bytes on
//! every machine. Fast math stays off: the stretch must be deterministic.

fn main() {
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=vendor");
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include("vendor")
        .file("src/shim.cpp")
        .opt_level(3)
        .flag_if_supported("-fno-fast-math")
        .flag_if_supported("-ffp-contract=off")
        .warnings(false)
        .compile("pm_signalsmith_stretch");
}
