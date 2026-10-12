//! glTF 2.0 decode into the geometry `pm-geom3d` needs: the node hierarchy, meshes
//! (triangles, points, lines), morph targets, skins, animations, materials (side and
//! opacity, which picks respect), EXT_mesh_gpu_instancing, KHR_mesh_quantization, and
//! images (PNG through the host, WebP everywhere, AVIF natively).
//!
//! The `gltf` crate parses the document and the GLB container. Accessor data is read here
//! (normalized integers for quantization, sparse accessors, strides), and extensions the
//! crate does not model are read from the raw JSON, as three.js r180's GLTFLoader reads
//! them.

use glam::{DMat4, DQuat, DVec3};
use serde_json::Value;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

/// Primitive topology after three.js's conversion (strips and fans become lists).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topology {
    Triangles,
    Points,
    /// `LineSegments` (mode 1), `LineLoop` (2), `Line` (3).
    LineSegments,
    LineLoop,
    LineStrip,
}

#[derive(Clone, Debug)]
pub struct Primitive {
    pub topology: Topology,
    pub positions: Vec<[f32; 3]>,
    /// Triangle (or point / line) indices; `None` is non-indexed.
    pub indices: Option<Vec<u32>>,
    pub joints: Option<Vec<[u16; 4]>>,
    pub weights: Option<Vec<[f32; 4]>>,
    /// Morph target position deltas (relative, as glTF and three.js's GLTFLoader use).
    pub morph_positions: Vec<Vec<[f32; 3]>>,
    pub material: Option<usize>,
    /// GLTFLoader's `computeBounds`: the POSITION accessor's min/max (normalized
    /// components scaled), grown by the largest absolute morph displacement. three.js
    /// uses it for raycast pre-tests instead of the exact bounds.
    pub accessor_bounds: Option<([f64; 3], [f64; 3])>,
}

#[derive(Clone, Debug)]
pub struct Mesh {
    pub name: String,
    pub primitives: Vec<Primitive>,
    pub weights: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Side {
    Front,
    Double,
}

#[derive(Clone, Debug)]
pub struct Material {
    pub name: String,
    pub side: Side,
    /// `baseColorFactor[3]`; three.js sets `opacity` from it for every alpha mode.
    pub opacity: f32,
    pub alpha_mode: gltf::material::AlphaMode,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub translation: DVec3,
    pub rotation: DQuat,
    pub scale: DVec3,
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
    /// Node-level morph weights (override the mesh's).
    pub weights: Option<Vec<f32>>,
    /// EXT_mesh_gpu_instancing: per-instance matrices, stored as f32 like three.js's
    /// `InstancedMesh.instanceMatrix`.
    pub instances: Option<Vec<[f32; 16]>>,
}

#[derive(Clone, Debug)]
pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<DMat4>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
    CubicSpline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    Translation,
    Rotation,
    Scale,
    Weights,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub node: usize,
    pub path: Path,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    /// Flat output values (for CUBICSPLINE: in-tangent, value, out-tangent per key).
    pub values: Vec<f32>,
    /// Components per value (3, 4, or the morph target count).
    pub size: usize,
}

#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub channels: Vec<Channel>,
    /// three.js `AnimationClip.resetDuration()`: the largest last key time.
    pub duration: f32,
}

#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA8 as decoded (no color conversion).
    pub rgba: Vec<u8>,
    pub mime: String,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub skins: Vec<Skin>,
    pub clips: Vec<Clip>,
    /// Decoded images by glTF image index (`None` when not decoded or the codec belongs
    /// to the host, e.g. AVIF in the wasm build).
    pub images: Vec<Option<Image>>,
    /// Texture index to image index, honoring EXT_texture_webp and EXT_texture_avif.
    pub texture_sources: Vec<Option<usize>>,
    pub extensions_used: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct LoadOptions {
    pub decode_images: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self { decode_images: true }
    }
}

/// Supported required extensions (three.js r180 loads these; the geometry needs the
/// mesh ones, the rest only affect shading).
const SUPPORTED: &[&str] = &[
    "KHR_mesh_quantization",
    "EXT_mesh_gpu_instancing",
    "EXT_texture_webp",
    "EXT_texture_avif",
    "KHR_texture_transform",
    "KHR_materials_unlit",
    "KHR_materials_emissive_strength",
    "KHR_materials_clearcoat",
    "KHR_materials_sheen",
    "KHR_materials_iridescence",
    "KHR_materials_specular",
    "KHR_materials_ior",
    "KHR_materials_transmission",
    "KHR_materials_volume",
    "KHR_materials_dispersion",
    "KHR_materials_anisotropy",
    "KHR_materials_bump",
    "EXT_materials_bump",
    "KHR_lights_punctual",
];

struct Data<'a> {
    json: &'a Value,
    buffers: Vec<&'a [u8]>,
}

impl<'a> Data<'a> {
    fn accessor(&self, index: usize) -> Result<&'a Value> {
        self.json["accessors"].get(index).ok_or_else(|| Error(format!("accessor {index} missing")))
    }

    /// Reads an accessor as f64 components (normalized integers mapped as glTF defines,
    /// which is what three.js's `BufferAttribute` getters return).
    fn read(&self, index: usize) -> Result<(Vec<f64>, usize)> {
        let a = self.accessor(index)?;
        let count = a["count"].as_u64().ok_or_else(|| Error("accessor count".into()))? as usize;
        let comps = match a["type"].as_str().unwrap_or("") {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT2" => 4,
            "MAT3" => 9,
            "MAT4" => 16,
            t => return err(format!("accessor type {t}")),
        };
        let ctype = a["componentType"].as_u64().unwrap_or(0) as u32;
        let normalized = a["normalized"].as_bool().unwrap_or(false);
        let csize = match ctype {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            c => return err(format!("component type {c}")),
        };
        let decode = |b: &[u8]| -> f64 {
            match ctype {
                5120 => {
                    let v = b[0] as i8 as f64;
                    if normalized { (v / 127.0).max(-1.0) } else { v }
                }
                5121 => {
                    let v = b[0] as f64;
                    if normalized { v / 255.0 } else { v }
                }
                5122 => {
                    let v = i16::from_le_bytes([b[0], b[1]]) as f64;
                    if normalized { (v / 32767.0).max(-1.0) } else { v }
                }
                5123 => {
                    let v = u16::from_le_bytes([b[0], b[1]]) as f64;
                    if normalized { v / 65535.0 } else { v }
                }
                5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            }
        };
        let mut out = vec![0.0; count * comps];
        if let Some(view_index) = a["bufferView"].as_u64() {
            let view = &self.json["bufferViews"][view_index as usize];
            let buffer = self.buffers.get(view["buffer"].as_u64().unwrap_or(0) as usize).ok_or_else(|| Error("buffer missing".into()))?;
            let start = view["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
            // Matrix columns of 1- and 2-byte components are 4-byte aligned in glTF.
            let column_pad = |rows: usize| -> usize { (rows * csize).div_ceil(4) * 4 };
            let element = match (comps, csize) {
                (4, 1) if a["type"] == "MAT2" => 2 * column_pad(2),
                (9, 1 | 2) => 3 * column_pad(3),
                (16, 1 | 2) => 4 * column_pad(4),
                _ => comps * csize,
            };
            let stride = view["byteStride"].as_u64().map(|s| s as usize).unwrap_or(element);
            let needed = if count == 0 { 0 } else { start + stride * (count - 1) + element };
            if needed > buffer.len() || needed > view["byteOffset"].as_u64().unwrap_or(0) as usize + view["byteLength"].as_u64().unwrap_or(0) as usize {
                return err(format!("accessor {index} reads past its buffer view"));
            }
            for i in 0..count {
                let base = start + i * stride;
                for c in 0..comps {
                    let offset = match (comps, a["type"].as_str()) {
                        (9, _) if csize < 4 => (c / 3) * column_pad(3) + (c % 3) * csize,
                        (16, _) if csize < 4 => (c / 4) * column_pad(4) + (c % 4) * csize,
                        (4, Some("MAT2")) if csize < 4 => (c / 2) * column_pad(2) + (c % 2) * csize,
                        _ => c * csize,
                    };
                    out[i * comps + c] = decode(&buffer[base + offset..]);
                }
            }
        }
        if let Some(sparse) = a.get("sparse") {
            let n = sparse["count"].as_u64().unwrap_or(0) as usize;
            let idx = &sparse["indices"];
            let val = &sparse["values"];
            let read_view = |v: &Value| -> Result<&'a [u8]> {
                let view = &self.json["bufferViews"][v["bufferView"].as_u64().ok_or_else(|| Error("sparse view".into()))? as usize];
                let buffer = self.buffers.get(view["buffer"].as_u64().unwrap_or(0) as usize).ok_or_else(|| Error("buffer missing".into()))?;
                let start = view["byteOffset"].as_u64().unwrap_or(0) as usize + v["byteOffset"].as_u64().unwrap_or(0) as usize;
                buffer.get(start..).ok_or_else(|| Error("sparse range".into()))
            };
            let ib = read_view(idx)?;
            let vb = read_view(val)?;
            let isize = match idx["componentType"].as_u64().unwrap_or(0) {
                5121 => 1,
                5123 => 2,
                5125 => 4,
                c => return err(format!("sparse index type {c}")),
            };
            for k in 0..n {
                let target = match isize {
                    1 => ib[k] as usize,
                    2 => u16::from_le_bytes([ib[2 * k], ib[2 * k + 1]]) as usize,
                    _ => u32::from_le_bytes([ib[4 * k], ib[4 * k + 1], ib[4 * k + 2], ib[4 * k + 3]]) as usize,
                };
                if target >= count {
                    return err("sparse index out of range");
                }
                for c in 0..comps {
                    out[target * comps + c] = decode(&vb[(k * comps + c) * csize..]);
                }
            }
        }
        Ok((out, comps))
    }

    fn read_vec3_f32(&self, index: usize) -> Result<Vec<[f32; 3]>> {
        let (v, comps) = self.read(index)?;
        if comps != 3 {
            return err("expected VEC3");
        }
        Ok(v.chunks_exact(3).map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]).collect())
    }
}

fn mat4_from_cols(m: &[f64]) -> DMat4 {
    DMat4::from_cols_slice(m)
}

/// Decodes a GLB (or a self-contained glTF JSON with data URIs) into a [`Model`].
pub fn load(bytes: &[u8], options: LoadOptions) -> Result<Model> {
    let gltf::Gltf { document, blob } = gltf::Gltf::from_slice_without_validation(bytes).map_err(|e| Error(format!("glTF: {e}")))?;
    let json: Value = serde_json::to_value(document.as_json()).map_err(|e| Error(format!("glTF JSON: {e}")))?;
    // Raw JSON for extensions (the `gltf` crate keeps unknown ones only as `others`).
    let raw: Value = if bytes.starts_with(b"glTF") {
        let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        serde_json::from_slice(&bytes[20..20 + len]).map_err(|e| Error(format!("GLB JSON: {e}")))?
    } else {
        serde_json::from_slice(bytes).map_err(|e| Error(format!("glTF JSON: {e}")))?
    };
    let _ = json;
    let used: Vec<String> = raw["extensionsUsed"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    for required in raw["extensionsRequired"].as_array().into_iter().flatten() {
        let name = required.as_str().unwrap_or("");
        if !SUPPORTED.contains(&name) {
            return err(format!("required extension {name} is not supported"));
        }
    }
    // Buffers: GLB BIN chunk or base64 data URIs.
    let mut owned: Vec<Vec<u8>> = Vec::new();
    for (i, b) in raw["buffers"].as_array().into_iter().flatten().enumerate() {
        match b["uri"].as_str() {
            None if i == 0 => owned.push(blob.clone().ok_or_else(|| Error("GLB has no BIN chunk".into()))?),
            Some(uri) if uri.starts_with("data:") => owned.push(decode_data_uri(uri)?),
            _ => return err("external buffers are not loaded"),
        }
    }
    let data = Data { json: &raw, buffers: owned.iter().map(|b| b.as_slice()).collect() };

    let mut model = Model { extensions_used: used, ..Default::default() };

    // Materials.
    for m in raw["materials"].as_array().into_iter().flatten() {
        let pbr = &m["pbrMetallicRoughness"];
        model.materials.push(Material {
            name: m["name"].as_str().unwrap_or("").to_string(),
            side: if m["doubleSided"].as_bool().unwrap_or(false) { Side::Double } else { Side::Front },
            opacity: pbr["baseColorFactor"].get(3).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
            alpha_mode: match m["alphaMode"].as_str().unwrap_or("OPAQUE") {
                "BLEND" => gltf::material::AlphaMode::Blend,
                "MASK" => gltf::material::AlphaMode::Mask,
                _ => gltf::material::AlphaMode::Opaque,
            },
        });
    }

    // Meshes.
    for (mi, m) in raw["meshes"].as_array().into_iter().flatten().enumerate() {
        let mut primitives = Vec::new();
        for p in m["primitives"].as_array().into_iter().flatten() {
            let attrs = &p["attributes"];
            let Some(pos_index) = attrs["POSITION"].as_u64() else {
                return err(format!("mesh {mi}: primitive without POSITION"));
            };
            let positions = data.read_vec3_f32(pos_index as usize)?;
            let mut indices = match p["indices"].as_u64() {
                Some(i) => Some(data.read(i as usize)?.0.into_iter().map(|v| v as u32).collect::<Vec<u32>>()),
                None => None,
            };
            let mode = p["mode"].as_u64().unwrap_or(4);
            let topology = match mode {
                0 => Topology::Points,
                1 => Topology::LineSegments,
                2 => Topology::LineLoop,
                3 => Topology::LineStrip,
                4 => Topology::Triangles,
                5 | 6 => {
                    // three.js toTrianglesDrawMode: strips alternate winding; fans pivot on 0.
                    let src: Vec<u32> = indices.clone().unwrap_or_else(|| (0..positions.len() as u32).collect());
                    let mut tris = Vec::new();
                    if mode == 6 {
                        for i in 1..src.len().saturating_sub(1) {
                            tris.extend_from_slice(&[src[0], src[i], src[i + 1]]);
                        }
                    } else {
                        for i in 0..src.len().saturating_sub(2) {
                            if i % 2 == 0 {
                                tris.extend_from_slice(&[src[i], src[i + 1], src[i + 2]]);
                            } else {
                                tris.extend_from_slice(&[src[i + 2], src[i + 1], src[i]]);
                            }
                        }
                    }
                    indices = Some(tris);
                    Topology::Triangles
                }
                m => return err(format!("primitive mode {m}")),
            };
            if let Some(ix) = &indices {
                if ix.iter().any(|&i| i as usize >= positions.len()) {
                    return err(format!("mesh {mi}: index out of range"));
                }
            }
            let joints = match attrs["JOINTS_0"].as_u64() {
                Some(i) => {
                    let (v, c) = data.read(i as usize)?;
                    if c != 4 {
                        return err("JOINTS_0 must be VEC4");
                    }
                    Some(v.chunks_exact(4).map(|j| [j[0] as u16, j[1] as u16, j[2] as u16, j[3] as u16]).collect())
                }
                None => None,
            };
            let weights = match attrs["WEIGHTS_0"].as_u64() {
                Some(i) => {
                    let (v, c) = data.read(i as usize)?;
                    if c != 4 {
                        return err("WEIGHTS_0 must be VEC4");
                    }
                    Some(v.chunks_exact(4).map(|w| [w[0] as f32, w[1] as f32, w[2] as f32, w[3] as f32]).collect())
                }
                None => None,
            };
            let mut morph_positions = Vec::new();
            for t in p["targets"].as_array().into_iter().flatten() {
                match t["POSITION"].as_u64() {
                    Some(i) => morph_positions.push(data.read_vec3_f32(i as usize)?),
                    // three.js fills a missing target attribute with zeros (relative).
                    None => morph_positions.push(vec![[0.0; 3]; positions.len()]),
                }
            }
            let accessor_bounds = accessor_bounds(&raw, attrs["POSITION"].as_u64().unwrap() as usize, p["targets"].as_array());
            primitives.push(Primitive { topology, positions, indices, joints, weights, morph_positions, material: p["material"].as_u64().map(|v| v as usize), accessor_bounds });
        }
        let weights = m["weights"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
        model.meshes.push(Mesh { name: m["name"].as_str().unwrap_or("").to_string(), primitives, weights });
    }

    // Nodes.
    let nodes = raw["nodes"].as_array().cloned().unwrap_or_default();
    for (ni, n) in nodes.iter().enumerate() {
        let (translation, rotation, scale) = if let Some(m) = n["matrix"].as_array() {
            let cols: Vec<f64> = m.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
            let (s, r, t) = mat4_from_cols(&cols).to_scale_rotation_translation();
            (t, r, s)
        } else {
            let t = n["translation"].as_array().map(|a| DVec3::new(f(a, 0), f(a, 1), f(a, 2))).unwrap_or(DVec3::ZERO);
            let r = n["rotation"].as_array().map(|a| DQuat::from_xyzw(f(a, 0), f(a, 1), f(a, 2), f(a, 3))).unwrap_or(DQuat::IDENTITY);
            let s = n["scale"].as_array().map(|a| DVec3::new(f(a, 0), f(a, 1), f(a, 2))).unwrap_or(DVec3::ONE);
            (t, r, s)
        };
        let instances = match n["extensions"].get("EXT_mesh_gpu_instancing") {
            Some(ext) => {
                let attrs = &ext["attributes"];
                let read = |name: &str, comps: usize| -> Result<Option<Vec<f64>>> {
                    match attrs[name].as_u64() {
                        Some(i) => {
                            let (v, c) = data.read(i as usize)?;
                            if c != comps {
                                return err(format!("instancing {name} components"));
                            }
                            Ok(Some(v))
                        }
                        None => Ok(None),
                    }
                };
                let t = read("TRANSLATION", 3)?;
                let r = read("ROTATION", 4)?;
                let s = read("SCALE", 3)?;
                let count = t.as_ref().map(|v| v.len() / 3).or(r.as_ref().map(|v| v.len() / 4)).or(s.as_ref().map(|v| v.len() / 3)).unwrap_or(0);
                let mut list = Vec::with_capacity(count);
                for k in 0..count {
                    // three.js reads each attribute into f32 vectors, composes in f64,
                    // and stores the matrix in a Float32Array.
                    let tr = t.as_ref().map(|v| DVec3::new(v[3 * k] as f32 as f64, v[3 * k + 1] as f32 as f64, v[3 * k + 2] as f32 as f64)).unwrap_or(DVec3::ZERO);
                    let ro = r.as_ref().map(|v| DQuat::from_xyzw(v[4 * k] as f32 as f64, v[4 * k + 1] as f32 as f64, v[4 * k + 2] as f32 as f64, v[4 * k + 3] as f32 as f64)).unwrap_or(DQuat::IDENTITY);
                    let sc = s.as_ref().map(|v| DVec3::new(v[3 * k] as f32 as f64, v[3 * k + 1] as f32 as f64, v[3 * k + 2] as f32 as f64)).unwrap_or(DVec3::ONE);
                    let m = compose(tr, ro, sc).to_cols_array();
                    let mut m32 = [0f32; 16];
                    for (o, v) in m32.iter_mut().zip(m) {
                        *o = v as f32;
                    }
                    list.push(m32);
                }
                Some(list)
            }
            None => None,
        };
        let children: Vec<usize> = n["children"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64().map(|v| v as usize)).collect()).unwrap_or_default();
        if children.iter().any(|&c| c >= nodes.len() || c == ni) {
            return err(format!("node {ni}: bad child"));
        }
        model.nodes.push(Node {
            name: n["name"].as_str().unwrap_or("").to_string(),
            parent: None,
            children,
            translation,
            rotation,
            scale,
            mesh: n["mesh"].as_u64().map(|v| v as usize),
            skin: n["skin"].as_u64().map(|v| v as usize),
            weights: n["weights"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()),
            instances,
        });
    }
    for i in 0..model.nodes.len() {
        for c in model.nodes[i].children.clone() {
            if model.nodes[c].parent.is_some() {
                return err(format!("node {c} has two parents"));
            }
            model.nodes[c].parent = Some(i);
        }
    }
    let scene = raw["scene"].as_u64().unwrap_or(0) as usize;
    model.roots = raw["scenes"].get(scene).and_then(|s| s["nodes"].as_array()).map(|a| a.iter().filter_map(|v| v.as_u64().map(|v| v as usize)).collect()).unwrap_or_default();

    // Skins.
    for s in raw["skins"].as_array().into_iter().flatten() {
        let joints: Vec<usize> = s["joints"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64().map(|v| v as usize)).collect()).unwrap_or_default();
        let inverse_bind = match s["inverseBindMatrices"].as_u64() {
            Some(i) => {
                let (v, c) = data.read(i as usize)?;
                if c != 16 {
                    return err("inverseBindMatrices must be MAT4");
                }
                // three.js copies them through a Float32Array-backed Matrix4.fromArray.
                v.chunks_exact(16).map(|m| DMat4::from_cols_slice(&m.iter().map(|x| *x as f32 as f64).collect::<Vec<_>>())).collect()
            }
            None => vec![DMat4::IDENTITY; joints.len()],
        };
        if inverse_bind.len() < joints.len() {
            return err("too few inverse bind matrices");
        }
        model.skins.push(Skin { joints, inverse_bind });
    }

    // Animations.
    for a in raw["animations"].as_array().into_iter().flatten() {
        let mut channels = Vec::new();
        let samplers = a["samplers"].as_array().cloned().unwrap_or_default();
        for c in a["channels"].as_array().into_iter().flatten() {
            let target = &c["target"];
            let Some(node) = target["node"].as_u64() else { continue };
            let path = match target["path"].as_str().unwrap_or("") {
                "translation" => Path::Translation,
                "rotation" => Path::Rotation,
                "scale" => Path::Scale,
                "weights" => Path::Weights,
                _ => continue,
            };
            let sampler = samplers.get(c["sampler"].as_u64().unwrap_or(u64::MAX) as usize).ok_or_else(|| Error("animation sampler".into()))?;
            let interpolation = match sampler["interpolation"].as_str().unwrap_or("LINEAR") {
                "STEP" => Interpolation::Step,
                "CUBICSPLINE" => Interpolation::CubicSpline,
                _ => Interpolation::Linear,
            };
            let (times, _) = data.read(sampler["input"].as_u64().ok_or_else(|| Error("sampler input".into()))? as usize)?;
            let (values, _) = data.read(sampler["output"].as_u64().ok_or_else(|| Error("sampler output".into()))? as usize)?;
            let keys = times.len().max(1);
            let per_key = if interpolation == Interpolation::CubicSpline { values.len() / keys / 3 } else { values.len() / keys };
            channels.push(Channel {
                node: node as usize,
                path,
                interpolation,
                times: times.iter().map(|t| *t as f32).collect(),
                values: values.iter().map(|v| *v as f32).collect(),
                size: per_key,
            });
        }
        let duration = channels.iter().filter_map(|c| c.times.last().copied()).fold(0.0f32, f32::max);
        model.clips.push(Clip { name: a["name"].as_str().unwrap_or("").to_string(), channels, duration });
    }

    // Textures and images.
    for t in raw["textures"].as_array().into_iter().flatten() {
        let ext = &t["extensions"];
        let source = ext["EXT_texture_avif"]["source"].as_u64().or(ext["EXT_texture_webp"]["source"].as_u64()).or(t["source"].as_u64());
        model.texture_sources.push(source.map(|v| v as usize));
    }
    for img in raw["images"].as_array().into_iter().flatten() {
        if !options.decode_images {
            model.images.push(None);
            continue;
        }
        let (bytes, mime): (Vec<u8>, String) = if let Some(view) = img["bufferView"].as_u64() {
            let v = &raw["bufferViews"][view as usize];
            let buffer = data.buffers.get(v["buffer"].as_u64().unwrap_or(0) as usize).ok_or_else(|| Error("image buffer".into()))?;
            let start = v["byteOffset"].as_u64().unwrap_or(0) as usize;
            let len = v["byteLength"].as_u64().unwrap_or(0) as usize;
            (buffer.get(start..start + len).ok_or_else(|| Error("image range".into()))?.to_vec(), img["mimeType"].as_str().unwrap_or("").to_string())
        } else if let Some(uri) = img["uri"].as_str() {
            let mime = uri.strip_prefix("data:").and_then(|s| s.split(';').next()).unwrap_or("").to_string();
            (decode_data_uri(uri)?, mime)
        } else {
            return err("image without data");
        };
        model.images.push(crate::image::decode(&bytes, &mime)?);
    }
    Ok(model)
}

fn accessor_bounds(raw: &Value, position: usize, targets: Option<&Vec<Value>>) -> Option<([f64; 3], [f64; 3])> {
    let scale = |a: &Value| -> f64 {
        if !a["normalized"].as_bool().unwrap_or(false) {
            return 1.0;
        }
        match a["componentType"].as_u64().unwrap_or(0) {
            5120 => 1.0 / 127.0,
            5121 => 1.0 / 255.0,
            5122 => 1.0 / 32767.0,
            5123 => 1.0 / 65535.0,
            _ => 1.0,
        }
    };
    let a = &raw["accessors"][position];
    let (min, max) = (a["min"].as_array()?, a["max"].as_array()?);
    let k = scale(a);
    let mut lo = [f(min, 0) * k, f(min, 1) * k, f(min, 2) * k];
    let mut hi = [f(max, 0) * k, f(max, 1) * k, f(max, 2) * k];
    let mut disp = [0f64; 3];
    for t in targets.into_iter().flatten() {
        let Some(i) = t["POSITION"].as_u64() else { continue };
        let ta = &raw["accessors"][i as usize];
        let (Some(tmin), Some(tmax)) = (ta["min"].as_array(), ta["max"].as_array()) else { continue };
        let k = scale(ta);
        for c in 0..3 {
            disp[c] = disp[c].max(f(tmin, c).abs().max(f(tmax, c).abs()) * k);
        }
    }
    for c in 0..3 {
        lo[c] -= disp[c];
        hi[c] += disp[c];
    }
    Some((lo, hi))
}

fn f(a: &[Value], i: usize) -> f64 {
    a.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

/// three.js `Matrix4.compose`.
pub fn compose(t: DVec3, r: DQuat, s: DVec3) -> DMat4 {
    let (x, y, z, w) = (r.x, r.y, r.z, r.w);
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xx, xy, xz) = (x * x2, x * y2, x * z2);
    let (yy, yz, zz) = (y * y2, y * z2, z * z2);
    let (wx, wy, wz) = (w * x2, w * y2, w * z2);
    DMat4::from_cols_array(&[
        (1.0 - (yy + zz)) * s.x,
        (xy + wz) * s.x,
        (xz - wy) * s.x,
        0.0,
        (xy - wz) * s.y,
        (1.0 - (xx + zz)) * s.y,
        (yz + wx) * s.y,
        0.0,
        (xz + wy) * s.z,
        (yz - wx) * s.z,
        (1.0 - (xx + yy)) * s.z,
        0.0,
        t.x,
        t.y,
        t.z,
        1.0,
    ])
}

fn decode_data_uri(uri: &str) -> Result<Vec<u8>> {
    let (_, b64) = uri.split_once(";base64,").ok_or_else(|| Error("only base64 data URIs are supported".into()))?;
    base64_decode(b64)
}

fn base64_decode(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return err("bad base64"),
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}
