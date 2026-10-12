//! Prints the preprocessed, rewritten fragment (or vertex with `vs`) source of a program.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let p = s7_glsl::compile(&std::fs::read_to_string(&a[0]).unwrap(), &std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    print!("{}", p.preprocessed[if a.get(2).map(|s| s == "vs").unwrap_or(false) { 0 } else { 1 }]);
}
