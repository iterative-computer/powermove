//! Prints the native geometry digest: `cargo run -p pm-vector --example digest --features conformance [cases]`.
fn main() {
    let cases = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let (hash, values) = pm_vector::conformance::digest(cases);
    println!("{hash:016x} {values}");
}
