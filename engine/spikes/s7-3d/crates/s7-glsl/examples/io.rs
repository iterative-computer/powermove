//! Prints entry-point IO bindings (location, interpolation, sampling) of a compiled program.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let p = s7_glsl::compile(&std::fs::read_to_string(&a[0]).unwrap(), &std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    for (stage, m) in [("vs", &p.vertex), ("fs", &p.fragment)] {
        for ep in &m.entry_points {
            for arg in &ep.function.arguments {
                if let Some(b) = &arg.binding { println!("{stage} in  {:?} {:?}", arg.name, b); }
                if let naga::TypeInner::Struct { members, .. } = &m.types[arg.ty].inner { for mem in members { println!("{stage} in. {:?} {:?}", mem.name, mem.binding); } }
            }
            if let Some(r) = &ep.function.result { if let naga::TypeInner::Struct { members, .. } = &m.types[r.ty].inner { for mem in members { println!("{stage} out {:?} {:?}", mem.name, mem.binding); } } }
        }
    }
}
