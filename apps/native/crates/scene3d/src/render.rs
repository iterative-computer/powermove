//! Worker-side imported surface extraction. The renderer consumes the same
//! normalized, posed resource as geometry operations; no placeholder meshes.
use crate::{Error, Result, fail, import::Model};
use gltf::{ObjectKind, math, pose};
pub use pm_geom3d::{Geometry, gltf};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Debug, Default)]
pub struct VertexAttributes {
    pub colors: Arc<Vec<[f32; 4]>>,
    pub secondary_uvs: Arc<Vec<[f32; 2]>>,
}
/// A legacy image material uses the same 1024² RGBA8 array as imported
/// Studio maps. Generated geometry uses Three.js' default flipped image UVs;
/// imported glTF geometry keeps the image's original row order.
pub fn material_texture(path: &std::path::Path, flip_y: bool) -> Result<Vec<u8>> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| Error(format!("Material image: {e}")))?
        .with_guessed_format()
        .map_err(|e| Error(format!("Material image: {e}")))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 << 20);
    reader.limits(limits);
    let source = reader
        .decode()
        .map_err(|e| Error(format!("Material image: {e}")))?
        .into_rgba8();
    let mut image =
        image::imageops::resize(&source, 1024, 1024, image::imageops::FilterType::Triangle);
    if flip_y {
        image::imageops::flip_vertical_in_place(&mut image);
    }
    Ok(image.into_raw())
}

pub struct Part {
    pub geometry: Arc<Geometry>,
    pub world: [f64; 16],
    pub material: Option<gltf::Material>,
    /// File material identity; final geometry variants can have other indices.
    pub source_material: Option<usize>,
    pub attributes: Arc<VertexAttributes>,
}
/// Resource lifetime owns decoded bytes and derived immutable geometry. Static
/// parts share geometry across times, instances and renderer snapshot changes.
pub struct Resource {
    pub model: Model,
    pub key: String,
    meshes: std::sync::Mutex<HashMap<(usize, u64), Arc<Geometry>>>,
    attributes: std::sync::Mutex<HashMap<usize, Arc<VertexAttributes>>>,
}
impl Resource {
    pub fn new(model: Model) -> Self {
        Self {
            model,
            key: String::new(),
            meshes: Default::default(),
            attributes: Default::default(),
        }
    }
    pub fn load(name: &str, bytes: &[u8]) -> Result<Arc<Self>> {
        use sha2::{Digest, Sha256};
        let key = format!(
            "{}:{:x}",
            name.rsplit('.').next().unwrap_or("").to_ascii_lowercase(),
            Sha256::digest(bytes)
        );
        // One slot per source: the same model decodes once while different
        // models decode in parallel.
        type Slot = Arc<std::sync::Mutex<std::sync::Weak<Resource>>>;
        static SHARED: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Slot>>> =
            std::sync::OnceLock::new();
        fn poisoned<T>(_: T) -> Error {
            Error("Model resource cache poisoned".into())
        }
        let slot = {
            let mut slots = SHARED
                .get_or_init(Default::default)
                .lock()
                .map_err(poisoned)?;
            slots.retain(|k, slot| {
                *k == key
                    || Arc::strong_count(slot) > 1
                    || slot.try_lock().map_or(true, |held| held.strong_count() > 0)
            });
            slots.entry(key.clone()).or_default().clone()
        };
        let mut held = slot.lock().map_err(poisoned)?;
        if let Some(resource) = held.upgrade() {
            return Ok(resource);
        }
        let mut resource = Self::new(crate::import::load(name, bytes)?);
        resource.key = key;
        let resource = Arc::new(resource);
        *held = Arc::downgrade(&resource);
        Ok(resource)
    }
    /// Source Studio uses a raw RGBA8 1024² map array; colour maps are
    /// decoded exactly once by the tracing shader, data maps remain raw.
    pub fn texture(&self, texture: usize) -> Result<Vec<u8>> {
        let Model::Gltf(model) = &self.model else {
            return fail("No imported model maps");
        };
        let texture = model
            .textures
            .get(texture)
            .ok_or_else(|| Error("Missing model texture".into()))?;
        let image = model
            .images
            .get(texture.image)
            .ok_or_else(|| Error("Missing model image".into()))?;
        Ok((*texture_rgba(&image.bytes)?).clone())
    }
    pub fn parts(&self, time: f64) -> Result<Vec<Part>> {
        if !time.is_finite() {
            return fail("Invalid model sample time");
        }
        let mut cache = self
            .meshes
            .lock()
            .map_err(|_| Error("Model geometry cache poisoned".into()))?;
        match &self.model {
            Model::Obj(m) => {
                let geometry = cache
                    .entry((0, 0))
                    .or_insert_with(|| {
                        Arc::new(Geometry {
                            positions: m.positions.clone(),
                            normals: m.normals.clone(),
                            uvs: m.texcoords.clone(),
                            indices: (0..m.vertex_count as u32).collect(),
                            indexed: false,
                            groups: vec![],
                        })
                    })
                    .clone();
                Ok(vec![Part {
                    geometry,
                    world: math::IDENTITY,
                    material: None,
                    source_material: None,
                    attributes: Arc::default(),
                }])
            }
            Model::Splats(_) => {
                fail("Gaussian splats require their rasterizer, not a surface substitute")
            }
            Model::Gltf(m) => {
                let pose = pose::Pose::at(m, time);
                let (parent, pose) = pose::placed(m, pose, &math::IDENTITY);
                let worlds = pose::world(m, &pose, Some(&parent));
                let mut parts = vec![];
                for object in m.traversal() {
                    let ObjectKind::Drawable(d) = &m.objects[object].kind else {
                        continue;
                    };
                    let g = &m.geometries[d.geometry];
                    if g.mode != gltf::Mode::Triangles {
                        return fail("Studio imported primitive is not a surface");
                    }
                    // Node motion changes only world; deformation changes geometry.
                    let deformed = d.skin.is_some() || g.morph.count() != 0;
                    let key = if deformed {
                        (object, time.to_bits())
                    } else {
                        (d.geometry, u64::MAX)
                    };
                    let geometry = if let Some(geometry) = cache.get(&key) {
                        geometry.clone()
                    } else {
                        let poser =
                            pose::Poser::new(m, object, d, &pose.influences[object], &worlds);
                        let mut positions = Vec::with_capacity(g.vertex_count() * 3);
                        let mut normals = Vec::with_capacity(g.vertex_count() * 3);
                        for i in 0..g.vertex_count() {
                            let p = poser.vertex(i);
                            positions.extend(p.map(|v| v as f32));
                            if let Some(n) = &g.normal {
                                let mut normal: [f64; 3] =
                                    std::array::from_fn(|c| n.data[i * 3 + c] as f64);
                                if let Some(targets) = &g.morph.normal {
                                    for (k, target) in targets.iter().enumerate() {
                                        let weight =
                                            pose.influences[object].get(k).copied().unwrap_or(0.);
                                        if weight != 0. {
                                            if i >= target.count() {
                                                return fail("Incomplete morph normals");
                                            }
                                            for (c, value) in normal.iter_mut().enumerate() {
                                                *value += target.data[i * 3 + c] as f64 * weight;
                                            }
                                        }
                                    }
                                }
                                // applyBoneTransform is affine: subtract its origin
                                // to transform a normal direction, not a point.
                                let end = poser.apply_bone(i, normal);
                                let origin = poser.apply_bone(i, [0.; 3]);
                                let normal: [f64; 3] = std::array::from_fn(|c| end[c] - origin[c]);
                                let length = normal.iter().map(|v| v * v).sum::<f64>().sqrt();
                                let normal =
                                    normal.map(|v| if length == 0. { 0. } else { v / length });
                                normals.extend(normal.map(|v| v as f32));
                            }
                        }
                        let indices = g.index.as_ref().map_or_else(
                            || (0..g.vertex_count() as u32).collect(),
                            |a| a.as_ref().clone(),
                        );
                        if !indices.len().is_multiple_of(3)
                            || indices.iter().any(|&i| i as usize >= g.vertex_count())
                            || positions.iter().chain(&normals).any(|x| !x.is_finite())
                        {
                            return fail("Invalid posed imported geometry");
                        }
                        let geometry = Arc::new(Geometry {
                            positions,
                            normals,
                            uvs: g.uv[0]
                                .as_ref()
                                .map_or_else(Vec::new, |a| a.data.as_ref().clone()),
                            indices,
                            indexed: g.index.is_some(),
                            groups: vec![],
                        });
                        // Bound worker-derived pose retention without changing pixels.
                        if cache.len() >= 512 {
                            cache.retain(|(_, t), _| *t == u64::MAX || *t == 0);
                        }
                        cache.insert(key, geometry.clone());
                        geometry
                    };
                    let attributes = {
                        let mut cache = self
                            .attributes
                            .lock()
                            .map_err(|_| Error("Model vertex cache poisoned".into()))?;
                        if let Some(attributes) = cache.get(&d.geometry) {
                            attributes.clone()
                        } else {
                            let mut attributes = VertexAttributes::default();
                            if let Some(color) = &g.color {
                                if ![3, 4].contains(&color.size)
                                    || color.count() != g.vertex_count()
                                {
                                    return fail("Invalid imported vertex colors");
                                }
                                attributes.colors = Arc::new(
                                    color
                                        .data
                                        .chunks_exact(color.size)
                                        .map(|c| {
                                            [
                                                c[0],
                                                c[1],
                                                c[2],
                                                if color.size == 4 { c[3] } else { 1. },
                                            ]
                                        })
                                        .collect(),
                                );
                            }
                            if let Some(uv) = &g.uv[1] {
                                if uv.size != 2 || uv.count() != g.vertex_count() {
                                    return fail("Invalid imported secondary UVs");
                                }
                                attributes.secondary_uvs = Arc::new(
                                    uv.data.chunks_exact(2).map(|p| [p[0], p[1]]).collect(),
                                );
                            }
                            let attributes = Arc::new(attributes);
                            cache.insert(d.geometry, attributes.clone());
                            attributes
                        }
                    };
                    let material = Some(m.materials[d.material].clone());
                    if let Some(instances) = &d.instances {
                        for instance in &instances.matrices {
                            parts.push(Part {
                                geometry: geometry.clone(),
                                world: math::multiply(&worlds[object], &instance.map(f64::from)),
                                material: material.clone(),
                                source_material: m.material_sources[d.material],
                                attributes: attributes.clone(),
                            });
                        }
                    } else {
                        parts.push(Part {
                            geometry,
                            world: worlds[object],
                            material,
                            source_material: m.material_sources[d.material],
                            attributes,
                        });
                    }
                }
                if parts.is_empty() {
                    return fail("Imported model has no surfaces");
                }
                Ok(parts)
            }
        }
    }
}

/// Complete immutable mesh identity, independent of address or path reuse.
pub fn geometry_key(g: &Geometry) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for data in [&g.positions, &g.normals, &g.uvs] {
        h.update((data.len() as u64).to_le_bytes());
        for value in data {
            h.update(value.to_bits().to_le_bytes());
        }
    }
    h.update((g.indices.len() as u64).to_le_bytes());
    for value in &g.indices {
        h.update(value.to_le_bytes());
    }
    format!("{:x}", h.finalize())
}

/// The shared native map array uses raw RGBA8 1024² images. Key cached pixels
/// by content, so relink/replacement under the same asset ID cannot go stale.
pub fn texture_rgba(bytes: &[u8]) -> Result<Arc<Vec<u8>>> {
    use sha2::{Digest, Sha256};
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Vec<([u8; 32], Arc<Vec<u8>>)>>> =
        std::sync::OnceLock::new();
    let key: [u8; 32] = Sha256::digest(bytes).into();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| Error("Map image cache poisoned".into()))?;
    if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
        let entry = cache.remove(i);
        let pixels = entry.1.clone();
        cache.push(entry);
        return Ok(pixels);
    }
    let image = image::load_from_memory(bytes)
        .map_err(|e| Error(format!("Material image: {e}")))?
        .into_rgba8();
    let pixels = Arc::new(
        image::imageops::resize(&image, 1024, 1024, image::imageops::FilterType::Triangle)
            .into_raw(),
    );
    cache.push((key, pixels.clone()));
    if cache.len() > 16 {
        cache.remove(0);
    }
    Ok(pixels)
}
