//! Diagnostic: dump the renderer compatibility DTO for a self-contained glTF.
//! `cargo run -p pm-wasm --example import_compat -- model.glb`
fn main() {
    let path = std::env::args().nth(1).expect("model path");
    let bytes = std::fs::read(path).expect("read model");
    let scene = pm_wasm::imported_model::import(&bytes).expect("import model");
    println!("{scene}");
}
