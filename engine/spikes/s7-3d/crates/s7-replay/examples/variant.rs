//! Prints the hand-written WGSL variant for a three.js program (or why there is none).
//!
//!   cargo run --example variant -- <vs.glsl> <fs.glsl>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vs = std::fs::read_to_string(&a[0]).unwrap();
    let fs = std::fs::read_to_string(&a[1]).unwrap();
    println!("{:?}", s7_replay::hand::variant(&vs, &fs));
    for line in s7_replay::hand::explain(&vs, &fs) {
        println!("  {line}");
    }
}
