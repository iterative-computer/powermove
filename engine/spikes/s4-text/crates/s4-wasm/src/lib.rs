//! The level 1 layout as a plain `wasm32-unknown-unknown` module, for the
//! native vs WASM identity check (plan section 12: "native and WASM layout
//! identical"). It runs exactly the code `s4-compare --replay --subsets`
//! runs natively: recorded registry answers plus metrics-and-shaping
//! subsets, no platform font APIs.
//!
//! The ABI is raw pointers and lengths so the check needs no binding
//! generator (scripts/wasm-identity.ts drives it from Bun):
//! - `s4_alloc(len) -> ptr` reserves `len` bytes the host fills;
//! - `s4_add_font(name, name_len, data, data_len)` takes ownership of a
//!   font buffer filled through `s4_alloc`;
//! - `s4_run(log, log_len, corpus, corpus_len) -> status` lays the corpus
//!   out; the JSON result is at `s4_out_ptr()` / `s4_out_len()`;
//! - `s4_misses()` is the number of resolver queries the log lacked.

use s4_layout::font::FontBytes;
use s4_layout::replay::{LogFile, run_corpus};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

thread_local! {
    static FONTS: RefCell<HashMap<String, FontBytes>> = RefCell::new(HashMap::new());
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static MISSES: RefCell<usize> = const { RefCell::new(0) };
}

/// Reserves `len` bytes and leaks them to the host; `s4_add_font` and
/// `s4_run` take the buffers back.
#[unsafe(no_mangle)]
pub extern "C" fn s4_alloc(len: usize) -> *mut u8 {
    let mut v = vec![0u8; len].into_boxed_slice();
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` must come from one `s4_alloc` call; the buffer is freed.
unsafe fn take(ptr: *mut u8, len: usize) -> Vec<u8> {
    unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) }.into_vec()
}

/// # Safety
/// Both buffers must come from `s4_alloc` with these lengths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn s4_add_font(name: *mut u8, name_len: usize, data: *mut u8, data_len: usize) {
    let name = String::from_utf8(unsafe { take(name, name_len) }).expect("utf-8 font name");
    let data = unsafe { take(data, data_len) };
    FONTS.with(|f| f.borrow_mut().insert(name, FontBytes(Arc::new(data))));
}

/// Returns 0 on success, 1 when a font named by the log was not added, 2 on
/// malformed input.
///
/// # Safety
/// Both buffers must come from `s4_alloc` with these lengths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn s4_run(log: *mut u8, log_len: usize, corpus: *mut u8, corpus_len: usize) -> u32 {
    let log = unsafe { take(log, log_len) };
    let corpus = unsafe { take(corpus, corpus_len) };
    let Ok(log) = serde_json::from_slice::<LogFile>(&log) else { return 2 };
    let Ok(corpus) = String::from_utf8(corpus) else { return 2 };
    let fonts = FONTS.with(|f| f.borrow().clone());
    if log.faces.iter().any(|r| !fonts.contains_key(&r.blob)) {
        return 1;
    }
    let (outputs, misses) = run_corpus(&log, |name| fonts[name].clone(), &corpus);
    MISSES.with(|m| *m.borrow_mut() = misses.len());
    let json = serde_json::to_vec(&outputs).expect("layout output serializes");
    OUT.with(|o| *o.borrow_mut() = json);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn s4_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn s4_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

#[unsafe(no_mangle)]
pub extern "C" fn s4_misses() -> usize {
    MISSES.with(|m| *m.borrow())
}
