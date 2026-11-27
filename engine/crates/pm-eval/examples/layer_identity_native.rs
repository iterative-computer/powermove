//! Prints the native digest of the layer replay; see `wasm_layer_identity.rs`.
//!
//!   cargo run --release -p pm-eval --example layer_identity_native -- \
//!     crates/pm-eval/tests/vectors/layer-vectors.json.gz

use std::io::Read;

#[path = "support/layer_identity.rs"]
mod layer_identity;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("path to layer-vectors.json.gz");
    let bytes = std::fs::read(&path).expect("read vectors");
    let mut text = String::new();
    if path.ends_with(".gz") {
        flate2::read::GzDecoder::new(&bytes[..])
            .read_to_string(&mut text)
            .expect("gunzip vectors");
    } else {
        text = String::from_utf8(bytes).expect("UTF-8");
    }
    let d = layer_identity::digest(&text);
    println!(
        "native layer digest {:016x} over {} numbers",
        d.hash(),
        d.numbers()
    );
}
