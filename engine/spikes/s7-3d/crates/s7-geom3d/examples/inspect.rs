//! Prints a model's nodes and primitives (debugging aid).
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let m = s7_geom3d::load(&std::fs::read(path).unwrap(), Default::default()).unwrap();
    for (i, n) in m.nodes.iter().enumerate() {
        if let Some(mi) = n.mesh {
            let prims: Vec<String> = m.meshes[mi].primitives.iter().map(|p| format!("{:?} v{} i{:?} mat{:?}", p.topology, p.positions.len(), p.indices.as_ref().map(|x| x.len()), p.material)).collect();
            println!("node {i} {} mesh {mi} scale {:?} inst {:?}: {}", n.name, n.scale, n.instances.as_ref().map(|v| v.len()), prims.join("; "));
        }
    }
    for (i, img) in m.images.iter().enumerate() {
        println!("image {i}: {:?}", img.as_ref().map(|im| (im.width, im.height, im.mime.clone())));
    }
}
