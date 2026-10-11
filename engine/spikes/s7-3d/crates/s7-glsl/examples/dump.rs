fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vs = std::fs::read_to_string(&a[0]).unwrap();
    let fs = std::fs::read_to_string(&a[1]).unwrap();
    let p = s7_glsl::compile(&vs, &fs).unwrap();
    for (stage, m) in [("vs", &p.vertex), ("fs", &p.fragment)] {
        for (_, v) in m.global_variables.iter() {
            if let naga::TypeInner::Struct { members, span } = &m.types[v.ty].inner {
                println!("{stage} block {:?} span {span}", v.name);
                for mem in members {
                    let extra = match &m.types[mem.ty].inner {
                        naga::TypeInner::Array { base, stride, .. } => format!(" array stride {stride} base {:?}", match &m.types[*base].inner { naga::TypeInner::Struct { span, .. } => format!("struct span {span}"), o => format!("{o:?}") }),
                        _ => String::new(),
                    };
                    println!("  {} @{}{extra}", mem.name.clone().unwrap_or_default(), mem.offset);
                }
            } else { println!("{stage} global {:?} {:?} {:?}", v.name, v.space, v.binding); }
        }
    }
    if a.len() > 2 {
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all() ^ naga::valid::ValidationFlags::CONTROL_FLOW_UNIFORMITY, naga::valid::Capabilities::all()).validate(&p.fragment).unwrap();
        let wgsl = naga::back::wgsl::write_string(&p.fragment, &info, naga::back::wgsl::WriterFlags::empty()).unwrap();
        std::fs::write(&a[2], wgsl).unwrap();
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all() ^ naga::valid::ValidationFlags::CONTROL_FLOW_UNIFORMITY, naga::valid::Capabilities::all()).validate(&p.vertex).unwrap();
        let wgsl = naga::back::wgsl::write_string(&p.vertex, &info, naga::back::wgsl::WriterFlags::empty()).unwrap();
        std::fs::write(format!("{}.vs.wgsl", a[2]), wgsl).unwrap();
    }
}
