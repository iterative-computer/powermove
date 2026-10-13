//! The parity cases and the Math sweep, compiled to wasm32 and driven by
//! `parity/wasm.ts` in bun, so the wasm build is checked against V8 the same
//! way the native build is (`tests/parity.rs`).
//!
//! Build: `cargo build -p pm-expr --release --example wasm_parity --target
//! wasm32-unknown-unknown`. The host writes input into the buffer `input(len)`
//! returns, calls `run_cases()`, and reads `output_len()` bytes at `output()`.

#[path = "../tests/support/mod.rs"]
mod support;

use std::sync::Mutex;

use pm_expr::Dialect;
use pm_model::json::{Value as Json, parse, to_canonical_string};
use support::{JsonExt, MATH_FUNCTIONS, evaluate_case, math_apply};

static INPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static OUTPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// A buffer of `len` bytes for the host to fill.
#[unsafe(no_mangle)]
pub extern "C" fn input(len: usize) -> *mut u8 {
    let mut input = INPUT.lock().unwrap();
    input.clear();
    input.resize(len, 0);
    input.as_mut_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn output() -> *const u8 {
    OUTPUT.lock().unwrap().as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn output_len() -> usize {
    OUTPUT.lock().unwrap().len()
}

/// Evaluates the cases file in the input (a context header line, then one
/// case per line) and writes one result record per case, in the encoding of
/// `parity/reference.mjs`.
#[unsafe(no_mangle)]
pub extern "C" fn run_cases() -> usize {
    let text = String::from_utf8(INPUT.lock().unwrap().clone()).expect("UTF-8 input");
    let mut lines = text.lines();
    let header = parse(lines.next().expect("header")).expect("header json");
    let Some(Json::Array(contexts)) = header.field("contexts") else {
        panic!("no contexts")
    };
    let mut out = String::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let case = parse(line).expect("case json");
        let source: Vec<u16> = case
            .field("source")
            .and_then(Json::as_js_string)
            .expect("source")
            .utf16_units()
            .collect();
        let ctx = case.field("ctx").and_then(Json::as_f64).expect("ctx") as usize;
        let record = evaluate_case(&source, &contexts[ctx], Dialect::Legacy);
        out.push_str(&to_canonical_string(&record));
        out.push('\n');
    }
    let mut output = OUTPUT.lock().unwrap();
    *output = out.into_bytes();
    output.len()
}

/// One Math sweep entry: function `index` of the sweep at `(a, b)`.
#[unsafe(no_mangle)]
pub extern "C" fn math(index: usize, a: f64, b: f64) -> f64 {
    math_apply(MATH_FUNCTIONS[index], a, b)
}
