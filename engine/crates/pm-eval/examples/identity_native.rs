//! Prints the native digest of [`support::identity::digest`]; see
//! `wasm_identity.rs`.

#[path = "support/identity.rs"]
mod identity;

fn main() {
    let path = std::env::args().nth(1).expect("path to eval-vectors.json");
    let text = std::fs::read_to_string(path).expect("read vectors");
    let d = identity::digest(&text);
    println!("native digest {:016x} over {} values", d.value(), d.count());
}
