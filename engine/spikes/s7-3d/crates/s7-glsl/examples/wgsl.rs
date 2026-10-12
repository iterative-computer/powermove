//! Writes a compiled program's fragment and vertex modules as WGSL (debugging aid).
//!
//!   cargo run --example wgsl -- <vs.glsl> <fs.glsl> <out-prefix>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let p = s7_glsl::compile(&std::fs::read_to_string(&a[0]).unwrap(), &std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    for (name, m, info) in [("vs", &p.vertex, &p.vertex_info), ("fs", &p.fragment, &p.fragment_info)] {
        let wgsl = naga::back::wgsl::write_string(m, info, naga::back::wgsl::WriterFlags::empty()).unwrap();
        std::fs::write(format!("{}.{name}.wgsl", a[2]), wgsl).unwrap();
    }
}
