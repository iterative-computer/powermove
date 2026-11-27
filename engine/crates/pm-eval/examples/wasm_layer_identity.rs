//! A WASM module exporting the layer replay digest
//! ([`support::layer_identity::digest`]) for the native/WASM bit-identity
//! check of the layer queries.
//!
//!   cargo build --release -p pm-eval --example wasm_layer_identity --target wasm32-unknown-unknown
//!   node crates/pm-eval/examples/support/run-wasm-identity.mjs \
//!     target/wasm32-unknown-unknown/release/examples/wasm_layer_identity.wasm \
//!     crates/pm-eval/tests/vectors/layer-vectors.json.gz
//!   cargo run --release -p pm-eval --example layer_identity_native -- \
//!     crates/pm-eval/tests/vectors/layer-vectors.json.gz
//!
//! Both print the same digest when the layer queries are bit-identical.

#[path = "support/layer_identity.rs"]
mod layer_identity;

/// Allocates `len` bytes for the caller to write the vectors into.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

fn run(ptr: *mut u8, len: usize) -> layer_identity::Digest {
    // SAFETY: the caller passes a buffer from `alloc` holding `len` bytes.
    let bytes = unsafe { Vec::from_raw_parts(ptr, len, len) };
    layer_identity::digest(&String::from_utf8(bytes).expect("UTF-8"))
}

/// The digest of the vectors at `ptr..ptr + len` (UTF-8 JSON).
///
/// # Safety
///
/// `ptr` must come from [`alloc`] with at least `len` initialized bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn digest(ptr: *mut u8, len: usize) -> u64 {
    run(ptr, len).hash()
}

/// The number of numbers the digest covers (for the report).
///
/// # Safety
///
/// As for [`digest`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn count(ptr: *mut u8, len: usize) -> u64 {
    run(ptr, len).numbers()
}
