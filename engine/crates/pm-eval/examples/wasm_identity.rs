//! A WASM module exporting [`support::identity::digest`] for the native/WASM
//! bit-identity check.
//!
//!   cargo build --release -p pm-eval --example wasm_identity --target wasm32-unknown-unknown
//!   node crates/pm-eval/examples/support/run-wasm-identity.mjs \
//!     target/wasm32-unknown-unknown/release/examples/wasm_identity.wasm \
//!     crates/pm-eval/tests/vectors/eval-vectors.json
//!   cargo run --release -p pm-eval --example identity_native -- crates/pm-eval/tests/vectors/eval-vectors.json
//!
//! Both print the same digest when evaluation is bit-identical.

#[path = "support/identity.rs"]
mod identity;

/// Allocates `len` bytes for the caller to write the vectors into.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// The digest of the vectors at `ptr..ptr + len` (UTF-8 JSON).
///
/// # Safety
///
/// `ptr` must come from [`alloc`] with at least `len` initialized bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn digest(ptr: *mut u8, len: usize) -> u64 {
    // SAFETY: the caller passes a buffer from `alloc` holding `len` bytes.
    let bytes = unsafe { Vec::from_raw_parts(ptr, len, len) };
    let text = String::from_utf8(bytes).expect("UTF-8");
    identity::digest(&text).value()
}

/// The number of values the digest covers (for the report).
///
/// # Safety
///
/// As for [`digest`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn count(ptr: *mut u8, len: usize) -> u64 {
    // SAFETY: as for `digest`.
    let bytes = unsafe { Vec::from_raw_parts(ptr, len, len) };
    identity::digest(&String::from_utf8(bytes).expect("UTF-8")).count()
}
