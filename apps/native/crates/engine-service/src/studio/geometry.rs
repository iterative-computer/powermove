//! Shared Studio terrain and nature preparation on the render worker.
use super::*;
use pm_studio3d_engine::{
    nature::Nature,
    terrain::{Terrain, TerrainLayer},
};

impl DocumentSource {
    pub(super) fn nature_mesh(
        &self,
        source: &Value,
        lod: usize,
    ) -> Result<(Arc<Mesh>, Vec<Material>), String> {
        let nature = Nature {
            species: serde_json::from_value(source["nature"].clone()).map_err(|e| e.to_string())?,
            seed: source["seed"].as_u64().unwrap_or(1) as u32,
        };
        let key = format!("nature:{source}:{lod}");
        Ok((
            self.cached_mesh(key, || Ok(nature.mesh(lod)))?,
            nature.materials().to_vec(),
        ))
    }
    pub(super) fn terrain_mesh(
        &self,
        object: &Value,
        base: Material,
        camera: &Camera,
    ) -> Result<(Arc<Mesh>, Vec<Material>), String> {
        let node = &object["terrain"];
        let p = &node["p"];
        let regions = node["regions"]
            .as_array()
            .ok_or("Missing terrain regions")?;
        let mut palette = vec![base];
        for region in regions {
            let p = &region["p"];
            palette.push(Material {
                metallic: n(p, "metalness", 0.),
                ..Material::dielectric(color(p, "color", "#737B47"), n(p, "roughness", 0.8))
            });
        }
        let mut terrain = Terrain {
            size: [n(p, "width", 20.), n(p, "depth", 20.)],
            resolution: n(p, "resolution", 64.).round() as u32,
            amplitude: n(p, "amplitude", 2.),
            frequency: n(p, "frequency", 0.1),
            octaves: n(p, "octaves", 5.).round() as u32,
            seed: val(&p["seed"]).as_f64().unwrap_or(1.).round() as u32,
            ridged: val(&p["ridged"]) == true,
            detail: n(p, "detail", 0.),
            erosion: n(p, "erosion", 0.).round() as u32,
            adaptive: val(&p["adaptive"]) == true,
            blend: n(p, "blend", 0.),
            natural: val(&p["natural"]) == true,
            water_level: n(p, "waterLevel", -100000.),
            snow_level: n(p, "snowLevel", 100000.),
            heights: node["heights"]
                .as_array()
                .ok_or("Missing terrain heights")?
                .iter()
                .map(|v| {
                    v.as_f64()
                        .map(|v| v as f32)
                        .ok_or("Invalid terrain height".to_string())
                })
                .collect::<Result<_, _>>()?,
            layers: regions
                .iter()
                .enumerate()
                .map(|(i, r)| TerrainLayer {
                    material: i + 1,
                    min_height: n(&r["p"], "minHeight", -100000.),
                    max_height: n(&r["p"], "maxHeight", 100000.),
                    min_slope: n(&r["p"], "minSlope", 0.),
                    max_slope: n(&r["p"], "maxSlope", 90.),
                })
                .collect(),
            ..Default::default()
        };
        if !node["heightSource"].is_null() {
            let id = node["heightSource"]
                .as_str()
                .ok_or("Live terrain height-source capture is not connected yet")?;
            let path = self
                .media
                .paths
                .get(id)
                .ok_or_else(|| format!("Terrain height image is not loaded: {id}"))?;
            let key = (id.to_owned(), terrain.resolution);
            let mut cache = self
                .height_maps
                .lock()
                .map_err(|_| "Terrain image cache poisoned")?;
            if let Some(index) = cache.iter().position(|(k, _)| *k == key) {
                let entry = cache.remove(index);
                terrain.heights = (*entry.1).clone();
                cache.push(entry);
            } else {
                terrain.load_height_image(path)?;
                cache.push((key, Arc::new(terrain.heights.clone())));
                if cache.len() > 8 {
                    cache.remove(0);
                }
            }
        }
        terrain.validate(palette.len())?;
        let resolution = terrain.resolution_for_camera(camera);
        // Include decoded heights, palette and chosen grid in the shared key;
        // replacing an asset under the same ID cannot reuse stale geometry.
        let key = format!(
            "terrain:{}:{palette:?}:{resolution}",
            serde_json::to_string(&terrain).map_err(|e| e.to_string())?
        );
        let colored = terrain.natural || terrain.blend > 0.;
        let mesh = self.cached_mesh(key, || {
            let terrain = if terrain.adaptive || terrain.erosion > 0 {
                terrain.baked(resolution)
            } else {
                terrain
            };
            if colored {
                return Ok(terrain.blended_mesh(&palette));
            }
            let mut groups = terrain.meshes().into_iter();
            let (material, mut mesh) = groups.next().ok_or("Terrain has no triangles")?;
            mesh.triangle_materials = vec![material; mesh.indices.len()];
            for (material, group) in groups {
                mesh.triangle_materials
                    .extend(std::iter::repeat_n(material, group.indices.len()));
                mesh.indices.extend(group.indices);
            }
            Ok(mesh)
        })?;
        if colored {
            palette = vec![Material {
                procedural: 1,
                ..Material::dielectric(Vec3::splat(1.), 0.88)
            }];
        }
        Ok((mesh, palette))
    }
}
