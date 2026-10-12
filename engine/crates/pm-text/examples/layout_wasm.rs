//! The level 1 layout as a plain `wasm32-unknown-unknown` module, for the
//! native vs WASM identity check (plan section 12: "native and WASM layout
//! identical"). It runs exactly the code the native subset replay runs
//! (`corpus::run_replay`): registry answers plus metrics-and-shaping
//! subsets, no platform font APIs.
//!
//! Build: `cargo build -p pm-text --release --example layout_wasm
//! --target wasm32-unknown-unknown --no-default-features --features level2`.
//!
//! The ABI is raw pointers and lengths so the check needs no binding
//! generator (scripts/wasm-identity.mjs drives it from Bun and Node):
//! - `pm_alloc(len) -> ptr` reserves `len` bytes the host fills;
//! - `pm_add_font(key, key_len, data, data_len)` takes a subset filled
//!   through `pm_alloc`, keyed by `answers::subset_key(blob, index)`;
//! - `pm_run(answers, answers_len, corpus, corpus_len) -> status` lays the
//!   corpus out; the JSON result is at `pm_out_ptr()` / `pm_out_len()`;
//! - `pm_misses()` and `pm_pending()` count resolver queries the answers
//!   lacked and faces whose subsets were not added;
//! - `pm_run_level2(inputs, inputs_len, corpus, corpus_len) -> status` lays
//!   the corpus out with level 2 over `corpus::Level2Inputs` (fonts added
//!   with `pm_add_font` under their keys).

use pm_text::answers::{Answers, subset_key};
use pm_text::corpus;
use pm_text::font::FontBytes;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static FONTS: RefCell<HashMap<String, FontBytes>> = RefCell::new(HashMap::new());
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static REPORT: RefCell<(usize, usize)> = const { RefCell::new((0, 0)) };
}

/// Reserves `len` bytes and leaks them to the host; `pm_add_font` and
/// `pm_run` take the buffers back.
#[unsafe(no_mangle)]
pub extern "C" fn pm_alloc(len: usize) -> *mut u8 {
    let mut v = vec![0u8; len].into_boxed_slice();
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` must come from one `pm_alloc` call; the buffer is freed.
unsafe fn take(ptr: *mut u8, len: usize) -> Vec<u8> {
    unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) }.into_vec()
}

/// # Safety
/// Both buffers must come from `pm_alloc` with these lengths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pm_add_font(key: *mut u8, key_len: usize, data: *mut u8, data_len: usize) {
    let key = String::from_utf8(unsafe { take(key, key_len) }).expect("utf-8 font key");
    let data = unsafe { take(data, data_len) };
    FONTS.with(|f| f.borrow_mut().insert(key, FontBytes::new(data)));
}

/// Returns 0 on success, 1 when a subset the answers name was not added,
/// 2 on malformed input.
///
/// # Safety
/// Both buffers must come from `pm_alloc` with these lengths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pm_run(answers: *mut u8, answers_len: usize, corpus_json: *mut u8, corpus_len: usize) -> u32 {
    let answers = unsafe { take(answers, answers_len) };
    let corpus_json = unsafe { take(corpus_json, corpus_len) };
    let Ok(answers) = serde_json::from_slice::<Answers>(&answers) else { return 2 };
    let Ok(corpus_json) = String::from_utf8(corpus_json) else { return 2 };
    let fonts = FONTS.with(|f| f.borrow().clone());
    if answers.sources().iter().any(|(b, i)| !fonts.contains_key(&subset_key(b, *i))) {
        return 1;
    }
    let cases = corpus::load_corpus(&corpus_json);
    let (outputs, misses, pending) = corpus::run_replay(&answers, |b, i| (fonts[&subset_key(b, i)].clone(), 0), &cases);
    REPORT.with(|r| *r.borrow_mut() = (misses, pending));
    let json = serde_json::to_vec(&outputs).expect("layout output serializes");
    OUT.with(|o| *o.borrow_mut() = json);
    0
}

/// Returns 0 on success, 1 when a font the inputs name was not added, 2 on
/// malformed input.
///
/// # Safety
/// Both buffers must come from `pm_alloc` with these lengths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pm_run_level2(inputs: *mut u8, inputs_len: usize, corpus_json: *mut u8, corpus_len: usize) -> u32 {
    let inputs = unsafe { take(inputs, inputs_len) };
    let corpus_json = unsafe { take(corpus_json, corpus_len) };
    let Ok(inputs) = serde_json::from_slice::<corpus::Level2Inputs>(&inputs) else { return 2 };
    let Ok(corpus_json) = String::from_utf8(corpus_json) else { return 2 };
    let fonts = FONTS.with(|f| f.borrow().clone());
    let cases = corpus::load_corpus(&corpus_json);
    let Ok(outputs) = corpus::run_level2(&inputs, |k| fonts.get(k).cloned(), &cases) else { return 1 };
    let json = serde_json::to_vec(&outputs).expect("layout output serializes");
    OUT.with(|o| *o.borrow_mut() = json);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_misses() -> usize {
    REPORT.with(|r| r.borrow().0)
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_pending() -> usize {
    REPORT.with(|r| r.borrow().1)
}
