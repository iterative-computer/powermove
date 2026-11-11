use std::sync::Arc;

use pm_studio3d_engine::extract::{Extractor, MeshInstance, SceneInput, Transform};
use pm_studio3d_engine::math::{Vec3, v3};
use pm_studio3d_engine::mesh::Mesh;
use pm_studio3d_engine::scene::preset;

fn input() -> SceneInput {
    let mut scene = preset("hero", "space-gray", 0.).unwrap();
    scene.objects.clear();
    SceneInput {
        scene,
        instances: Vec::new(),
        materials: None,
        maps: None,
        view: Default::default(),
    }
}

fn triangle() -> Arc<Mesh> {
    Arc::new(Mesh {
        positions: vec![v3(-1., 0., 0.), v3(1., 0., 0.), v3(0., 1., 0.)],
        indices: vec![[0, 1, 2]],
        ..Default::default()
    })
}

fn instance(mesh: Arc<Mesh>, layer: &str) -> MeshInstance {
    MeshInstance {
        mesh,
        layer: layer.into(),
        material: 0,
        world: Transform::default(),
    }
}

#[test]
fn motion_reuses_blas_and_rebuilds_world_bounds() {
    let mut input = input();
    input.instances.push(instance(triangle(), "product"));
    let mut extractor = Extractor::default();
    let first = extractor.extract(&input).unwrap();
    input.instances[0].world.0[12] = 12.;
    input.instances[0].world.0[0] = -2.;
    let second = extractor.extract(&input).unwrap();
    assert_eq!(extractor.builds, 1);
    assert_eq!(first.layers, ["product"]);
    assert_eq!(second.bounds.0.x, 10.);
    assert_eq!(second.bounds.1.x, 14.);
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&first.prepared.positions),
        bytemuck::cast_slice::<_, u8>(&second.prepared.positions)
    );
    // Inverse x = -.5*world_x + 6; normal matrix is inverse transpose.
    let o = second.prepared.lights.len() * 4;
    assert_eq!(second.prepared.sdata[o], [-0.5, 0., 0., 6.]);
    assert_eq!(second.prepared.sdata[o + 3][0], -0.5);
}

#[test]
fn material_and_layer_assignment_do_not_rebuild_geometry() {
    let mesh = triangle();
    let mut input = input();
    input.instances = vec![instance(mesh.clone(), "a"), instance(mesh, "b")];
    input.instances[1].material = 1;
    let mut extractor = Extractor::default();
    let e = extractor.extract(&input).unwrap();
    assert_eq!(extractor.builds, 1);
    assert_eq!(e.layers, ["a", "b"]);
    assert_eq!(
        e.prepared.tris.len(),
        2,
        "distinct material bindings retain their own triangles"
    );
    let offset = e.prepared.lights.len() * 4;
    let mut assignments: Vec<_> = e.prepared.sdata[offset..offset + input.instances.len() * 9]
        .chunks_exact(9)
        .map(|r| (r[4][3] as u32, r[5][3] as u32))
        .collect();
    assignments.sort();
    assert_eq!(assignments, [(2, 0), (4, 0)]);
    input.instances[1].layer = "a".into();
    let e = extractor.extract(&input).unwrap();
    assert_eq!(extractor.builds, 1);
    assert_eq!(e.layers, ["a"]);
    assert!(
        e.prepared.sdata[offset..offset + input.instances.len() * 9]
            .chunks_exact(9)
            .all(|r| r[4][3] == 2.)
    );
}

#[test]
fn tlas_leaves_index_instances_in_reordered_scene_data() {
    let mut input = input();
    let mesh = triangle();
    input.instances = (0..36)
        .map(|i| {
            let mut instance = instance(mesh.clone(), &format!("layer-{i}"));
            instance.world.0[12] = ((i * 17) % 36) as f32 * 4.;
            instance
        })
        .collect();
    let mut extractor = Extractor::default();
    let e = extractor.extract(&input).unwrap();
    assert_eq!(extractor.builds, 1);
    let light_end = e.prepared.lights.len() * 4;
    let mut stack = vec![0];
    let mut seen = [false; 36];
    while let Some(i) = stack.pop() {
        let n = e.prepared.nodes[i];
        for (start, count, min, max) in [
            (n.left, n.lcount, n.lmin, n.lmax),
            (n.right, n.rcount, n.rmin, n.rmax),
        ] {
            match count {
                u32::MAX => {}
                0 => stack.push(start as usize),
                _ => {
                    for k in start..start + count {
                        assert!(!seen[k as usize]);
                        seen[k as usize] = true;
                        let inv = &e.prepared.sdata[light_end + k as usize * 9..];
                        let x = -inv[0][3];
                        assert!(x - 1. >= min[0] && x + 1. <= max[0]);
                        let root = inv[3][3] as usize;
                        assert!(root > input.instances.len() * 2);
                        let b = e.prepared.nodes[root];
                        assert_eq!(b.lcount, 1);
                        let id = (inv[4][3] as usize >> 1) - 1;
                        assert_eq!(input.instances[id].world.0[12], x);
                    }
                }
            }
        }
    }
    assert!(seen.iter().all(|s| *s));
}

#[test]
fn empty_scene_and_empty_mesh_are_real_empty_trees() {
    let mut input = input();
    input
        .instances
        .push(instance(Arc::new(Mesh::default()), "empty"));
    let e = Extractor::default().extract(&input).unwrap();
    assert!(e.layers.is_empty());
    assert!(e.prepared.tris.is_empty());
    assert_eq!(e.prepared.nodes[0].lcount, u32::MAX);
    assert_eq!(e.prepared.nodes[0].rcount, u32::MAX);
    assert_eq!(e.bounds.0.x, 0.);
}

#[test]
fn invalid_authored_data_is_refused_without_panicking() {
    let mut input = input();
    input.instances.push(instance(triangle(), "a"));
    let mut extractor = Extractor::default();
    input.instances[0].material = usize::MAX;
    assert!(extractor.extract(&input).is_err());
    input.instances[0].material = 0;
    input.instances[0].world.0[0] = 0.;
    assert!(extractor.extract(&input).is_err());
    input.instances[0].world = Transform::default();
    input.instances[0].world.0[12] = f32::NAN;
    assert!(extractor.extract(&input).is_err());
    input.instances[0].world = Transform::default();
    input.instances[0].mesh = Arc::new(Mesh {
        positions: vec![Vec3::ZERO],
        indices: vec![[0, 1, 0]],
        ..Default::default()
    });
    assert!(extractor.extract(&input).is_err());
}

#[test]
fn removed_geometry_releases_the_cache() {
    let mesh = triangle();
    let mut input = input();
    input.instances.push(instance(mesh.clone(), "a"));
    let mut extractor = Extractor::default();
    extractor.extract(&input).unwrap();
    input.instances.clear();
    extractor.extract(&input).unwrap();
    assert_eq!(Arc::strong_count(&mesh), 1);
}
