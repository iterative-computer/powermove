//! Shared GLB output for the mesh importers.
use super::{MAX_BYTES, MAX_TRIANGLES, texture};
use crate::{Error, Result, fail};
use image::RgbaImage;
use pm_geom3d::{gltf::math::Mat4, model_import::glb::GlbWriter};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const MAX_VERTICES: usize = MAX_TRIANGLES * 3;
pub const MAX_NODES: usize = 65_536;
pub const MAX_FACE_VERTICES: usize = 4096;

#[derive(Default)]
pub struct Scene {
    writer: GlbWriter,
    meshes: Vec<Value>,
    nodes: Vec<Value>,
    materials: Vec<Value>,
    material_keys: HashMap<String, usize>,
    images: Vec<Value>,
    image_keys: HashMap<[u8; 32], usize>,
    // Cache input pixels too, so formats that repeat a decoded map across
    // material objects do not repeatedly compress it.
    texture_keys: HashMap<([u8; 32], bool, Sampler), usize>,
    textures: Vec<Value>,
    texture_slots: HashMap<(usize, usize), usize>,
    samplers: Vec<Value>,
    sampler_keys: HashMap<Sampler, usize>,
    vertices: usize,
    triangles: usize,
    bytes: usize,
}

#[derive(Default)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub uv: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct Sampler {
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub mag_filter: u32,
    pub min_filter: u32,
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            wrap_s: 10497,
            wrap_t: 10497,
            mag_filter: 9729,
            min_filter: 9987,
        }
    }
}

#[derive(Default)]
pub enum Alpha {
    #[default]
    Opaque,
    Blend,
    Mask(f32),
}

pub struct Material {
    pub base_color: [f32; 4],
    pub base_map: Option<usize>,
    pub metallic: f32,
    pub roughness: f32,
    pub metallic_roughness_map: Option<usize>,
    pub normal_map: Option<usize>,
    pub occlusion_map: Option<usize>,
    pub occlusion_strength: f32,
    pub emissive: [f32; 3],
    pub emissive_map: Option<usize>,
    pub alpha: Alpha,
    pub double_sided: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: [1.; 4],
            base_map: None,
            metallic: 0.,
            roughness: 1.,
            metallic_roughness_map: None,
            normal_map: None,
            occlusion_map: None,
            occlusion_strength: 1.,
            emissive: [0.; 3],
            emissive_map: None,
            alpha: Alpha::Opaque,
            double_sided: false,
        }
    }
}

pub struct Part {
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

impl Scene {
    pub fn material(&mut self, name: &str, rgba: [f32; 4]) -> usize {
        self.pbr_material(
            name,
            Material {
                base_color: rgba,
                alpha: if rgba[3] < 1. {
                    Alpha::Blend
                } else {
                    Alpha::Opaque
                },
                ..Material::default()
            },
        )
    }

    pub fn pbr_material(&mut self, name: &str, m: Material) -> usize {
        let mode = match m.alpha {
            Alpha::Opaque => "OPAQUE",
            Alpha::Blend => "BLEND",
            Alpha::Mask(_) => "MASK",
        };
        let mut material = json!({
            "name": name,
            "pbrMetallicRoughness": {
                "baseColorFactor": m.base_color,
                "metallicFactor": m.metallic,
                "roughnessFactor": m.roughness,
            },
            "emissiveFactor": m.emissive,
            "alphaMode": mode,
            "doubleSided": m.double_sided,
        });
        if let Alpha::Mask(cutoff) = m.alpha {
            material["alphaCutoff"] = json!(cutoff);
        }
        let pbr = &mut material["pbrMetallicRoughness"];
        for (key, map) in [
            ("baseColorTexture", m.base_map),
            ("metallicRoughnessTexture", m.metallic_roughness_map),
        ] {
            if let Some(index) = map {
                pbr[key] = json!({"index": index});
            }
        }
        for (key, map) in [
            ("normalTexture", m.normal_map),
            ("emissiveTexture", m.emissive_map),
        ] {
            if let Some(index) = map {
                material[key] = json!({"index": index});
            }
        }
        if let Some(index) = m.occlusion_map {
            material["occlusionTexture"] =
                json!({"index": index, "strength": m.occlusion_strength});
        }
        let key = material.to_string();
        *self.material_keys.entry(key).or_insert_with(|| {
            let index = self.materials.len();
            self.materials.push(material);
            index
        })
    }

    pub fn texture(&mut self, image: RgbaImage, color: bool, sampler: Sampler) -> Result<usize> {
        let image = texture::capped(image);
        let mut hash = Sha256::new();
        hash.update(image.width().to_le_bytes());
        hash.update(image.height().to_le_bytes());
        hash.update(image.as_raw());
        let key = (hash.finalize().into(), color, sampler);
        if let Some(&index) = self.texture_keys.get(&key) {
            return Ok(index);
        }
        let (mime, bytes) = texture::encode(image, color)?;
        let hash = Sha256::digest(&bytes).into();
        let source = if let Some(&index) = self.image_keys.get(&hash) {
            index
        } else {
            self.bytes += bytes.len();
            if self.bytes > MAX_BYTES {
                return fail("Model textures exceed import limits");
            }
            let index = self.images.len();
            let view = self.writer.view(&bytes, None);
            self.images
                .push(json!({"bufferView": view, "mimeType": mime}));
            self.image_keys.insert(hash, index);
            index
        };
        let sampler = *self.sampler_keys.entry(sampler).or_insert_with(|| {
            let index = self.samplers.len();
            self.samplers
                .push(json!({"wrapS": sampler.wrap_s, "wrapT": sampler.wrap_t,
                "magFilter": sampler.mag_filter, "minFilter": sampler.min_filter}));
            index
        });
        let index = *self
            .texture_slots
            .entry((source, sampler))
            .or_insert_with(|| {
                let index = self.textures.len();
                self.textures
                    .push(json!({"source": source, "sampler": sampler}));
                index
            });
        self.texture_keys.insert(key, index);
        Ok(index)
    }

    /// Compact each material's vertex stream. The renderer decodes a vertex
    /// stream per primitive, so sharing an untrimmed accessor in the file
    /// would still copy and upload all parent vertices for every material.
    pub fn partitioned_mesh(
        &mut self,
        name: &str,
        vertices: Mesh,
        parts: Vec<Part>,
    ) -> Result<usize> {
        let count = vertices.positions.len();
        if [
            &vertices.normals.len(),
            &vertices.colors.len(),
            &vertices.uv.len(),
        ]
        .into_iter()
        .any(|&len| len != 0 && len != count)
        {
            return fail("Invalid mesh vertex attributes");
        }
        let mut groups = Vec::<Part>::new();
        let mut slots = HashMap::new();
        for part in parts {
            if !part.indices.len().is_multiple_of(3) {
                return fail("Invalid mesh triangle indices");
            }
            let slot = *slots.entry(part.material).or_insert_with(|| {
                groups.push(Part {
                    material: part.material,
                    indices: Vec::new(),
                });
                groups.len() - 1
            });
            groups[slot].indices.extend(part.indices);
        }
        let mut parts = Vec::with_capacity(groups.len());
        for Part { material, indices } in groups {
            let mut part = Mesh {
                material,
                ..Mesh::default()
            };
            let mut remap = HashMap::new();
            for index in indices {
                let i = index as usize;
                if i >= count {
                    return fail("Invalid mesh triangle index");
                }
                let local = *remap.entry(index).or_insert_with(|| {
                    let local = part.positions.len() as u32;
                    part.positions.push(vertices.positions[i]);
                    if !vertices.normals.is_empty() {
                        part.normals.push(vertices.normals[i]);
                    }
                    if !vertices.colors.is_empty() {
                        part.colors.push(vertices.colors[i]);
                    }
                    if !vertices.uv.is_empty() {
                        part.uv.push(vertices.uv[i]);
                    }
                    local
                });
                part.indices.push(local);
            }
            parts.push(part);
        }
        self.mesh(name, parts)
    }

    pub fn mesh(&mut self, name: &str, parts: impl IntoIterator<Item = Mesh>) -> Result<usize> {
        let mut primitives = Vec::new();
        for part in parts {
            if part.positions.is_empty() || part.indices.is_empty() {
                continue;
            }
            let count = part.positions.len();
            self.vertices += count;
            self.triangles += part.indices.len() / 3;
            self.bytes += count * 12
                + part.normals.len() * 12
                + part.colors.len() * 16
                + part.uv.len() * 8
                + part.indices.len() * 4;
            if self.vertices > MAX_VERTICES
                || self.triangles > MAX_TRIANGLES
                || self.bytes > MAX_BYTES
            {
                return fail("Model geometry exceeds import limits");
            }
            if !part.indices.len().is_multiple_of(3)
                || part.indices.iter().any(|&i| i as usize >= count)
                || part.positions.iter().flatten().any(|v| !v.is_finite())
            {
                return fail("Invalid mesh positions or triangle indices");
            }
            let mut attributes = json!({"POSITION": self.writer.floats(&part.positions, true)});
            for (length, valid) in [
                (
                    part.normals.len(),
                    part.normals.iter().flatten().all(|v| v.is_finite()),
                ),
                (
                    part.colors.len(),
                    part.colors.iter().flatten().all(|v| v.is_finite()),
                ),
                (
                    part.uv.len(),
                    part.uv.iter().flatten().all(|v| v.is_finite()),
                ),
            ] {
                if length != 0 && (length != count || !valid) {
                    return fail("Invalid mesh vertex attributes");
                }
            }
            if !part.normals.is_empty() {
                attributes["NORMAL"] = json!(self.writer.floats(&part.normals, false));
            }
            if !part.colors.is_empty() {
                attributes["COLOR_0"] = json!(self.writer.floats(&part.colors, false));
            }
            if !part.uv.is_empty() {
                attributes["TEXCOORD_0"] = json!(self.writer.floats(&part.uv, false));
            }
            let mut primitive =
                json!({"attributes": attributes, "indices": self.writer.indices(&part.indices)});
            if let Some(material) = part.material {
                primitive["material"] = json!(material);
            }
            primitives.push(primitive);
        }
        if primitives.is_empty() {
            return fail("Model has no mesh triangles");
        }
        let index = self.meshes.len();
        self.meshes
            .push(json!({"name": name, "primitives": primitives}));
        Ok(index)
    }

    pub fn node(
        &mut self,
        name: &str,
        mesh: Option<usize>,
        matrix: Option<Mat4>,
        children: Vec<usize>,
    ) -> Result<usize> {
        let index = self.nodes.len();
        if index >= MAX_NODES
            || matrix
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || !(*v as f32).is_finite())
        {
            return fail("Invalid or oversized model hierarchy");
        }
        let mut node = json!({"name": name});
        if let Some(mesh) = mesh {
            node["mesh"] = json!(mesh);
        }
        if let Some(matrix) = matrix {
            node["matrix"] = json!(matrix);
        }
        if !children.is_empty() {
            node["children"] = json!(children);
        }
        self.nodes.push(node);
        Ok(index)
    }

    pub fn finish(self, roots: Vec<usize>) -> Result<Vec<u8>> {
        if self.meshes.is_empty() || roots.is_empty() {
            return fail("Model has no mesh triangles");
        }
        let bytes = self.writer.finish(
            json!({
                "asset": {"version": "2.0"},
                "scene": 0,
                "scenes": [{"nodes": roots}],
                "nodes": self.nodes,
                "meshes": self.meshes,
                "materials": self.materials,
                "textures": self.textures,
                "images": self.images,
                "samplers": self.samplers,
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        if bytes.len() > MAX_BYTES {
            return fail("Converted model exceeds 512 MB");
        }
        Ok(bytes)
    }
}

pub fn single(name: &str, mesh: Mesh) -> Result<Vec<u8>> {
    let mut scene = Scene::default();
    let mesh = scene.mesh(name, [mesh])?;
    let root = scene.node(name, Some(mesh), None, vec![])?;
    scene.finish(vec![root])
}

pub fn number<T: std::str::FromStr>(value: Option<&str>) -> Result<T> {
    value
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error("Missing or invalid mesh number".into()))
}

pub fn position(words: &mut std::str::SplitWhitespace<'_>) -> Result<[f32; 3]> {
    Ok([
        number(words.next())?,
        number(words.next())?,
        number(words.next())?,
    ])
}

pub fn srgb(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// Project onto the dominant face plane so concave polygons triangulate
/// correctly. Keep the source winding after earcut normalizes it.
pub fn polygon(positions: &[[f32; 3]], face: &[u32], out: &mut Vec<u32>) -> Result<()> {
    if !(3..=MAX_FACE_VERTICES).contains(&face.len())
        || out.len() / 3 + face.len() - 2 > MAX_TRIANGLES
        || face.iter().any(|&i| i as usize >= positions.len())
    {
        return fail("Invalid or oversized polygon");
    }
    if face.len() == 3 {
        out.extend_from_slice(face);
        return Ok(());
    }
    let point = |i: u32| positions[i as usize].map(f64::from);
    let mut normal = [0.; 3];
    for (&a, &b) in face.iter().zip(face.iter().cycle().skip(1)) {
        let (a, b) = (point(a), point(b));
        for k in 0..3 {
            let j = (k + 1) % 3;
            let l = (k + 2) % 3;
            normal[k] += a[j] * b[l] - a[l] * b[j];
        }
    }
    let axis = (0..3)
        .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .unwrap();
    let flat: Vec<_> = face
        .iter()
        .flat_map(|&i| {
            let p = point(i);
            [p[(axis + 1) % 3], p[(axis + 2) % 3]]
        })
        .collect();
    if flat.iter().any(|v| !v.is_finite()) {
        return fail("Invalid polygon positions");
    }
    for triangle in pm_geom3d::earcut::earcut(&flat, &[]).chunks_exact(3) {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
        let area = (flat[2 * b] - flat[2 * a]) * (flat[2 * c + 1] - flat[2 * a + 1])
            - (flat[2 * b + 1] - flat[2 * a + 1]) * (flat[2 * c] - flat[2 * a]);
        if area * normal[axis] < 0. {
            out.extend([face[a], face[c], face[b]]);
        } else {
            out.extend([face[a], face[b], face[c]]);
        }
    }
    Ok(())
}
