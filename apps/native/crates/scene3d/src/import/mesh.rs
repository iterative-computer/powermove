//! Shared GLB output for the mesh importers.
use super::{MAX_BYTES, MAX_TRIANGLES};
use crate::{Error, Result, fail};
use pm_geom3d::{gltf::math::Mat4, model_import::glb::GlbWriter};
use serde_json::{Value, json};

pub const MAX_VERTICES: usize = MAX_TRIANGLES * 3;
pub const MAX_NODES: usize = 65_536;
pub const MAX_FACE_VERTICES: usize = 4096;

#[derive(Default)]
pub struct Scene {
    writer: GlbWriter,
    meshes: Vec<Value>,
    nodes: Vec<Value>,
    materials: Vec<Value>,
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

impl Scene {
    pub fn material(&mut self, name: &str, rgba: [f32; 4]) -> usize {
        let index = self.materials.len();
        self.materials.push(json!({
            "name": name,
            "pbrMetallicRoughness": {"baseColorFactor": rgba, "metallicFactor": 0},
            "alphaMode": if rgba[3] < 1. { "BLEND" } else { "OPAQUE" },
        }));
        index
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
