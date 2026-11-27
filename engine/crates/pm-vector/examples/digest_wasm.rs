//! The geometry digest as a wasm32 export, for comparison with the native
//! digest (see tests/wasm/digest.ts).

/// Digest over `cases` cases per family.
#[unsafe(no_mangle)]
pub extern "C" fn pm_vector_digest(cases: u32) -> u64 {
    pm_vector::conformance::digest(cases).0
}

/// Number of values the digest covers.
#[unsafe(no_mangle)]
pub extern "C" fn pm_vector_digest_values(cases: u32) -> u64 {
    pm_vector::conformance::digest(cases).1
}
