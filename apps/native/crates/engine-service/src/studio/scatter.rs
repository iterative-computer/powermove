//! Seeded world-surface placements reuse resident prototype meshes and palettes.
use super::*;
use pm_studio3d_engine::{
    nature::select_lod,
    scatter::{Instance, Scatter},
    scene::Object,
};

pub(super) fn apply(
    source: &DocumentSource,
    r: &pm_scene3d::service::ResolvedScene,
    scene: &Scene,
    instances: &mut Vec<MeshInstance>,
) -> Result<(), String> {
    let objects = r.scene["objects"]
        .as_array()
        .ok_or("Missing Studio objects")?;
    let mut scattered = Vec::new();
    for object in objects
        .iter()
        .filter(|o| o.get("scatter").is_some() && val(&o["p"]["visible"]) != false)
    {
        let id = object["id"].as_str().ok_or("Scatter has no ID")?;
        let node = &object["scatter"];
        let model = node["model"].as_str().ok_or("Scatter has no model")?;
        let surface = node["surface"].as_str().ok_or("Scatter has no surface")?;
        let prototypes: Vec<_> = instances.iter().filter(|i| i.layer == model).collect();
        let surfaces: Vec<_> = instances.iter().filter(|i| i.layer == surface).collect();
        if prototypes.is_empty() || surfaces.is_empty() {
            return Err(format!(
                "Scatter {id}: model or surface geometry is unavailable"
            ));
        }
        let p = &node["p"];
        let settings = Scatter {
            objects: vec![0],
            surface: Some(0),
            count: n(p, "count", 100.).round() as u32,
            density: n(p, "density", 1.),
            seed: val(&p["seed"]).as_f64().unwrap_or(1.).round() as u32,
            scale: [n(p, "scaleMin", 0.8), n(p, "scaleMax", 1.2)],
            rotation: [n(p, "rotationMin", 0.), n(p, "rotationMax", 360.)],
            min_height: n(p, "minHeight", -100000.),
            max_height: n(p, "maxHeight", 100000.),
            min_slope: n(p, "minSlope", 0.),
            max_slope: n(p, "maxSlope", 60.),
            ..Default::default()
        };
        // Mesh Arcs are retained for this immutable source. Pointer identity plus
        // exact evaluated matrices identifies posed/deformed surface changes.
        // Camera motion is excluded: LOD must not reroll placements.
        let mut key = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
        for surface in &surfaces {
            key.push_str(&format!(
                "{:p}:{:?}",
                Arc::as_ptr(&surface.mesh),
                surface.world.0
            ));
        }
        let mut cache = source
            .scatter_placements
            .lock()
            .map_err(|_| "Scatter placement cache poisoned")?;
        let placements = if let Some(index) = cache.iter().position(|(k, _)| *k == key) {
            let entry = cache.remove(index);
            let placements = entry.1.clone();
            cache.push(entry);
            placements
        } else {
            let mut mesh = Mesh::default();
            for surface in &surfaces {
                let offset = u32::try_from(mesh.positions.len())
                    .map_err(|_| "Scatter surface exceeds index capacity")?;
                mesh.positions.extend(
                    surface
                        .mesh
                        .positions
                        .iter()
                        .map(|p| surface.world.point(*p)),
                );
                mesh.indices
                    .extend(surface.mesh.indices.iter().map(|t| t.map(|i| i + offset)));
            }
            let mut placement_scene = scene.clone();
            placement_scene.objects = vec![Object { mesh, material: 0 }];
            settings.validate(&placement_scene)?;
            let placements = Arc::new(settings.generate(&placement_scene));
            cache.push((key, placements.clone()));
            while cache.len() > 4
                || cache
                    .iter()
                    .map(|(_, p)| p.len() * std::mem::size_of::<Instance>())
                    .sum::<usize>()
                    > 64 * 1024 * 1024
            {
                cache.remove(0);
            }
            placements
        };
        drop(cache);
        let root = r
            .model_worlds
            .get(model)
            .ok_or("Scatter source world is unavailable")?;
        let nature = objects
            .iter()
            .find(|o| o["id"] == model && o["source"].get("nature").is_some());
        let diameter = if nature.is_some() {
            let (lo, hi) = prototypes[0].mesh.bounds();
            let size = hi - lo;
            let scale = [0, 4, 8]
                .map(|i| {
                    (root[i].powi(2) + root[i + 1].powi(2) + root[i + 2].powi(2)).sqrt() as f32
                })
                .into_iter()
                .fold(0., f32::max);
            size.x.max(size.y).max(size.z) * scale
        } else {
            0.
        };
        let nature_meshes = nature
            .map(|nature| {
                (0..3)
                    .map(|lod| {
                        source
                            .nature_mesh(&nature["source"], lod)
                            .map(|(mesh, _)| mesh)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        for placement in placements.iter() {
            let a = placement.matrix();
            let world = [
                a[0] as f64,
                a[4] as f64,
                a[8] as f64,
                0.,
                a[1] as f64,
                a[5] as f64,
                a[9] as f64,
                0.,
                a[2] as f64,
                a[6] as f64,
                a[10] as f64,
                0.,
                a[3] as f64,
                a[7] as f64,
                a[11] as f64,
                1.,
            ];
            for prototype in &prototypes {
                let mut instance = (*prototype).clone();
                instance.layer = id.into();
                if let Some(meshes) = &nature_meshes {
                    let lod = select_lod(
                        diameter * placement.scale.x,
                        (placement.position - scene.camera.position).length(),
                    );
                    instance.mesh = meshes[lod].clone();
                }
                // Preserve source rotation/scale and imported part offsets, but
                // discard its root translation, exactly as the scene contract.
                let mut basis = prototype.world.0.map(|v| v as f64);
                for i in 12..15 {
                    basis[i] -= root[i];
                }
                instance.world = Transform(
                    pm_scene3d::render::gltf::math::multiply(&world, &basis).map(|v| v as f32),
                );
                scattered.push(instance);
            }
        }
    }
    instances.retain(|i| {
        objects
            .iter()
            .find(|o| o["id"] == i.layer)
            .is_some_and(|o| val(&o["p"]["visible"]) != false)
    });
    instances.extend(scattered);
    Ok(())
}
