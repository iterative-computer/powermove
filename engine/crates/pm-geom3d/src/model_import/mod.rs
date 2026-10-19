//! glTF 2.0 model import (plan sections 8 and 13 "3D models"): GLB and
//! self-contained glTF decoded as three.js r180's GLTFLoader decodes them,
//! the import checks of today's `loadGltf` (`core/scene3d/import-model.ts`),
//! and the object graph three.js builds, with the facts the importer reports
//! (vertices, triangles, and the normalization that centers the model and
//! scales it to unit radius).
//!
//! The `gltf` crate parses the GLB container and the document; accessor data
//! is read here (strides, normalized integers, sparse accessors) and
//! extensions the crate does not model are read from the raw JSON, as
//! three.js reads them. Images stay encoded (PNG, JPEG, WebP, AVIF bytes and
//! their MIME type); the renderer decodes them. Draco and meshopt are not
//! supported (section 13: no Draco or meshopt).
//!
//! - [`load`]: a [`Model`] from GLB or glTF bytes.
//! - [`import`]: `loadGltf`'s checks, then the [`Scene`] three.js builds and
//!   its [`ImportFacts`].
//! - [`obj`] (feature `import`): OBJ with MTL and textures packed to a GLB,
//!   as `prepareImport` packs them today.

mod accessor;
pub mod math;
mod scene;

pub mod glb;

pub use scene::{
    ImportFacts, MaterialFacts, Object, ObjectKind, Scene, geometry_box, import, material_facts,
};

use accessor::Data;
use math::Mat4;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Primitive topology after three.js's conversion (strips and fans become
/// triangle lists).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topology {
    Triangles,
    Points,
    /// `LineSegments` (mode 1).
    LineSegments,
    /// `LineLoop` (mode 2).
    LineLoop,
    /// `Line` (mode 3).
    LineStrip,
}

/// A morph target's deltas (relative, as GLTFLoader keeps them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MorphTarget {
    pub position: Option<Vec<[f32; 3]>>,
    pub normal: Option<Vec<[f32; 3]>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Primitive {
    pub topology: Topology,
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    /// TEXCOORD_0 and TEXCOORD_1 (three.js `uv`, `uv1`).
    pub uv: Option<Vec<[f32; 2]>>,
    pub uv1: Option<Vec<[f32; 2]>>,
    /// COLOR_0 as RGBA (RGB colors get alpha 1); `color_components` says
    /// which the file stored (three.js keeps the item size).
    pub colors: Option<Vec<[f32; 4]>>,
    pub color_components: usize,
    pub tangents: Option<Vec<[f32; 4]>>,
    pub joints: Option<Vec<[u16; 4]>>,
    pub weights: Option<Vec<[f32; 4]>>,
    /// WEIGHTS_0's component type (5126 float, 5121 or 5123 normalized).
    pub weights_type: u32,
    /// Indices (converted to lists for strips and fans); `None` is non-indexed.
    pub indices: Option<Vec<u32>>,
    pub targets: Vec<MorphTarget>,
    pub material: Option<usize>,
    /// The POSITION accessor's min and max (normalized components scaled),
    /// grown by the largest morph displacement (GLTFLoader's `computeBounds`).
    pub accessor_bounds: Option<([f64; 3], [f64; 3])>,
    /// Why WebGL refuses to draw this primitive, when it does: an index past
    /// the last vertex or a vertex attribute shorter than POSITION. three.js
    /// loads such a primitive (it counts toward the import facts and
    /// bounds) and draws nothing for it, so the renderer skips it.
    pub undrawable: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub primitives: Vec<Primitive>,
    pub weights: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}

/// KHR_texture_transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureTransform {
    pub offset: [f32; 2],
    pub rotation: f32,
    pub scale: [f32; 2],
    pub tex_coord: Option<u32>,
}

/// A material's reference to a texture.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureRef {
    pub texture: usize,
    pub tex_coord: u32,
    pub transform: Option<TextureTransform>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    pub base_color: [f32; 4],
    pub base_color_texture: Option<TextureRef>,
    pub metallic: f32,
    pub roughness: f32,
    pub metallic_roughness_texture: Option<TextureRef>,
    pub normal_texture: Option<TextureRef>,
    pub normal_scale: f32,
    pub occlusion_texture: Option<TextureRef>,
    pub occlusion_strength: f32,
    pub emissive: [f32; 3],
    pub emissive_texture: Option<TextureRef>,
    /// KHR_materials_emissive_strength.
    pub emissive_strength: f32,
    /// KHR_materials_unlit.
    pub unlit: bool,
    /// The material's `extensions` object (every KHR_materials_* the
    /// renderer reads as three.js does).
    pub extensions: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sampler {
    pub mag_filter: Option<u32>,
    pub min_filter: Option<u32>,
    pub wrap_s: u32,
    pub wrap_t: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    /// The image, honoring EXT_texture_webp and EXT_texture_avif.
    pub source: Option<usize>,
    pub sampler: Sampler,
}

/// An embedded image, still encoded.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LightKind {
    Directional,
    Point,
    Spot { inner_cone: f64, outer_cone: f64 },
}

/// KHR_lights_punctual.
#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub name: String,
    pub kind: LightKind,
    pub color: [f64; 3],
    pub intensity: f64,
    pub range: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Camera {
    Perspective {
        yfov: f64,
        aspect: Option<f64>,
        znear: f64,
        zfar: Option<f64>,
    },
    Orthographic {
        xmag: f64,
        ymag: f64,
        znear: f64,
        zfar: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// The local matrix (TRS composed, or `matrix` decomposed and
    /// recomposed as three.js's `applyMatrix4` does).
    pub translation: [f64; 3],
    pub rotation: [f64; 4],
    pub scale: [f64; 3],
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
    pub camera: Option<usize>,
    pub light: Option<usize>,
    /// Node-level morph weights.
    pub weights: Option<Vec<f32>>,
    /// EXT_mesh_gpu_instancing matrices, stored as f32 like three.js's
    /// `instanceMatrix`.
    pub instances: Option<Vec<[f32; 16]>>,
    /// Referenced as a skin joint (three.js makes it a `Bone`).
    pub is_bone: bool,
}

impl Node {
    pub fn matrix(&self) -> Mat4 {
        math::compose(self.translation, self.rotation, self.scale)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skin {
    pub joints: Vec<usize>,
    /// Inverse bind matrices (f32 values, as three.js reads them).
    pub inverse_bind: Vec<Mat4>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub node: usize,
    pub path: Path,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    /// Flat output values (for CUBICSPLINE: in-tangent, value, out-tangent).
    pub values: Vec<f32>,
    /// Components per value (3, 4, or the morph target count).
    pub size: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    pub channels: Vec<Channel>,
    /// `AnimationClip.resetDuration()`: the largest last key time.
    pub duration: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub nodes: Vec<Node>,
    /// The default scene's name and root nodes.
    pub scene_name: String,
    pub roots: Vec<usize>,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub images: Vec<Image>,
    pub skins: Vec<Skin>,
    pub clips: Vec<Clip>,
    pub lights: Vec<Light>,
    pub cameras: Vec<Camera>,
    /// Each camera's `name` (empty when it has none).
    pub camera_names: Vec<String>,
    pub extensions_used: Vec<String>,
}

/// Extensions three.js r180 loads with the app's configuration (no Draco,
/// meshopt, or KTX2 decoder).
pub const SUPPORTED: &[&str] = &[
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
    "EXT_materials_bump",
    "KHR_lights_punctual",
];

fn f(a: &[Value], i: usize) -> f64 {
    a.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn ids(v: &Value) -> Vec<usize> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_u64().map(|v| v as usize))
                .collect()
        })
        .unwrap_or_default()
}

/// The document's JSON (the GLB JSON chunk, or the file).
pub fn json_of(bytes: &[u8]) -> Result<Value> {
    if bytes.starts_with(b"glTF") {
        if bytes.len() < 20 {
            return err("Invalid GLB container");
        }
        let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let json = bytes
            .get(20..20 + len)
            .ok_or_else(|| Error("Invalid GLB scene chunk".into()))?;
        serde_json::from_slice(json).map_err(|e| Error(format!("GLB JSON: {e}")))
    } else {
        serde_json::from_slice(bytes).map_err(|e| Error(format!("glTF JSON: {e}")))
    }
}

/// Decodes a GLB, or a self-contained glTF with data URIs.
pub fn load(bytes: &[u8]) -> Result<Model> {
    // The document as raw JSON, as GLTFLoader reads it (a typed schema
    // refuses files GLTFLoader loads: a 15-value matrix, an animation
    // channel without a node).
    let (raw, blob) = crate::gltf::container(bytes).map_err(|e| Error(e.0))?;
    let used: Vec<String> = raw["extensionsUsed"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    // GLTFLoader warns about an unknown required extension and loads the
    // file; the ones it refuses (Draco without a decoder, KTX2, meshopt)
    // are refused by the acceptance check (`scene::import`).
    let mut owned: Vec<Vec<u8>> = Vec::new();
    for (i, b) in raw["buffers"].as_array().into_iter().flatten().enumerate() {
        match b["uri"].as_str() {
            None if i == 0 => owned.push(
                blob.clone()
                    .ok_or_else(|| Error("GLB has no BIN chunk".into()))?,
            ),
            Some(uri) if uri.starts_with("data:") => owned.push(decode_data_uri(uri)?),
            _ => {
                return err(
                    "Use a self-contained GLB with embedded textures and geometry. External glTF files are not loaded.",
                );
            }
        }
    }
    let data = Data {
        json: &raw,
        buffers: owned.iter().map(|b| b.as_slice()).collect(),
    };
    let mut model = Model {
        extensions_used: used,
        ..Default::default()
    };
    load_materials(&raw, &mut model);
    load_meshes(&raw, &data, &mut model)?;
    load_nodes(&raw, &data, &mut model)?;
    load_skins(&raw, &data, &mut model)?;
    load_clips(&raw, &data, &mut model)?;
    load_textures(&raw, &data, &mut model)?;
    for l in raw["extensions"]["KHR_lights_punctual"]["lights"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let color = l["color"]
            .as_array()
            .map_or([1.0; 3], |c| [f(c, 0), f(c, 1), f(c, 2)]);
        let kind = match l["type"].as_str().unwrap_or("") {
            "directional" => LightKind::Directional,
            "point" => LightKind::Point,
            "spot" => LightKind::Spot {
                inner_cone: l["spot"]["innerConeAngle"].as_f64().unwrap_or(0.0),
                outer_cone: l["spot"]["outerConeAngle"]
                    .as_f64()
                    .unwrap_or(std::f64::consts::FRAC_PI_4),
            },
            t => return err(format!("unknown light type {t}")),
        };
        model.lights.push(Light {
            name: l["name"].as_str().unwrap_or("").into(),
            kind,
            color,
            intensity: l["intensity"].as_f64().unwrap_or(1.0),
            range: l["range"].as_f64(),
        });
    }
    for c in raw["cameras"].as_array().into_iter().flatten() {
        model
            .camera_names
            .push(c["name"].as_str().unwrap_or("").into());
        model.cameras.push(match c["type"].as_str().unwrap_or("") {
            "orthographic" => {
                let o = &c["orthographic"];
                Camera::Orthographic {
                    xmag: o["xmag"].as_f64().unwrap_or(1.0),
                    ymag: o["ymag"].as_f64().unwrap_or(1.0),
                    znear: o["znear"].as_f64().unwrap_or(0.01),
                    zfar: o["zfar"].as_f64().unwrap_or(100.0),
                }
            }
            _ => {
                let p = &c["perspective"];
                Camera::Perspective {
                    yfov: p["yfov"].as_f64().unwrap_or(0.8),
                    aspect: p["aspectRatio"].as_f64(),
                    znear: p["znear"].as_f64().unwrap_or(0.01),
                    zfar: p["zfar"].as_f64(),
                }
            }
        });
    }
    let scene = raw["scene"].as_u64().unwrap_or(0) as usize;
    model.roots = raw["scenes"]
        .get(scene)
        .map(|s| ids(&s["nodes"]))
        .unwrap_or_default();
    model.scene_name = raw["scenes"]
        .get(scene)
        .and_then(|s| s["name"].as_str())
        .unwrap_or("")
        .into();
    if model.roots.iter().any(|&r| r >= model.nodes.len()) {
        return err("scene root out of range");
    }
    Ok(model)
}

fn texture_ref(v: &Value) -> Option<TextureRef> {
    let texture = v["index"].as_u64()? as usize;
    let t = &v["extensions"]["KHR_texture_transform"];
    let transform = t.is_object().then(|| TextureTransform {
        offset: t["offset"]
            .as_array()
            .map_or([0.0; 2], |a| [f(a, 0) as f32, f(a, 1) as f32]),
        rotation: t["rotation"].as_f64().unwrap_or(0.0) as f32,
        scale: t["scale"]
            .as_array()
            .map_or([1.0; 2], |a| [f(a, 0) as f32, f(a, 1) as f32]),
        tex_coord: t["texCoord"].as_u64().map(|c| c as u32),
    });
    Some(TextureRef {
        texture,
        tex_coord: v["texCoord"].as_u64().unwrap_or(0) as u32,
        transform,
    })
}

fn load_materials(raw: &Value, model: &mut Model) {
    for m in raw["materials"].as_array().into_iter().flatten() {
        let pbr = &m["pbrMetallicRoughness"];
        let base = pbr["baseColorFactor"].as_array().map_or([1.0; 4], |a| {
            [
                f(a, 0) as f32,
                f(a, 1) as f32,
                f(a, 2) as f32,
                f(a, 3) as f32,
            ]
        });
        let ext = &m["extensions"];
        model.materials.push(Material {
            name: m["name"].as_str().unwrap_or("").into(),
            alpha_mode: match m["alphaMode"].as_str().unwrap_or("OPAQUE") {
                "BLEND" => AlphaMode::Blend,
                "MASK" => AlphaMode::Mask,
                _ => AlphaMode::Opaque,
            },
            alpha_cutoff: m["alphaCutoff"].as_f64().unwrap_or(0.5) as f32,
            double_sided: m["doubleSided"].as_bool().unwrap_or(false),
            base_color: base,
            base_color_texture: texture_ref(&pbr["baseColorTexture"]),
            metallic: pbr["metallicFactor"].as_f64().unwrap_or(1.0) as f32,
            roughness: pbr["roughnessFactor"].as_f64().unwrap_or(1.0) as f32,
            metallic_roughness_texture: texture_ref(&pbr["metallicRoughnessTexture"]),
            normal_texture: texture_ref(&m["normalTexture"]),
            normal_scale: m["normalTexture"]["scale"].as_f64().unwrap_or(1.0) as f32,
            occlusion_texture: texture_ref(&m["occlusionTexture"]),
            occlusion_strength: m["occlusionTexture"]["strength"].as_f64().unwrap_or(1.0) as f32,
            emissive: m["emissiveFactor"].as_array().map_or([0.0; 3], |a| {
                [f(a, 0) as f32, f(a, 1) as f32, f(a, 2) as f32]
            }),
            emissive_texture: texture_ref(&m["emissiveTexture"]),
            emissive_strength: ext["KHR_materials_emissive_strength"]["emissiveStrength"]
                .as_f64()
                .unwrap_or(1.0) as f32,
            unlit: ext.get("KHR_materials_unlit").is_some(),
            extensions: ext.clone(),
        });
    }
}

fn load_meshes(raw: &Value, data: &Data<'_>, model: &mut Model) -> Result<()> {
    for (mi, m) in raw["meshes"].as_array().into_iter().flatten().enumerate() {
        let mut primitives = Vec::new();
        for p in m["primitives"].as_array().into_iter().flatten() {
            if p["extensions"].get("KHR_draco_mesh_compression").is_some() {
                return err("Draco-compressed meshes are not supported");
            }
            let attrs = &p["attributes"];
            // An attribute accessor of a type GLTFLoader has no size for has
            // no elements: the attribute is absent (`crate::gltf`'s reader).
            let attr = |name: &str| -> Option<u64> {
                attrs[name].as_u64().filter(|&i| {
                    !matches!(data.layout(i as usize), Err(e) if e.0.starts_with("accessor type"))
                })
            };
            let Some(pos_index) = attrs["POSITION"].as_u64() else {
                return err(format!("mesh {mi}: primitive without POSITION"));
            };
            let positions = data.read_n::<3>(pos_index as usize)?;
            let opt3 = |name: &str| -> Result<Option<Vec<[f32; 3]>>> {
                attr(name).map(|i| data.read_n::<3>(i as usize)).transpose()
            };
            let opt2 = |name: &str| -> Result<Option<Vec<[f32; 2]>>> {
                attr(name).map(|i| data.read_n::<2>(i as usize)).transpose()
            };
            let normals = opt3("NORMAL")?;
            let uv = opt2("TEXCOORD_0")?;
            let uv1 = opt2("TEXCOORD_1")?;
            let (colors, color_components) = match attr("COLOR_0") {
                Some(i) => {
                    let (v, c) = data.read(i as usize)?;
                    let list = match c {
                        3 => v
                            .chunks_exact(3)
                            .map(|x| [x[0] as f32, x[1] as f32, x[2] as f32, 1.0])
                            .collect(),
                        4 => v
                            .chunks_exact(4)
                            .map(|x| [x[0] as f32, x[1] as f32, x[2] as f32, x[3] as f32])
                            .collect(),
                        _ => return err("COLOR_0 must be VEC3 or VEC4"),
                    };
                    (Some(list), c)
                }
                None => (None, 0),
            };
            let tangents = attr("TANGENT")
                .map(|i| data.read_n::<4>(i as usize))
                .transpose()?;
            let mut indices = match p["indices"].as_u64() {
                Some(i) => Some(
                    data.read(i as usize)?
                        .0
                        .into_iter()
                        .map(|v| v as u32)
                        .collect::<Vec<u32>>(),
                ),
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
                    // toTrianglesDrawMode: strips alternate winding; fans pivot on 0.
                    let src: Vec<u32> = indices
                        .clone()
                        .unwrap_or_else(|| (0..positions.len() as u32).collect());
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
            let mut undrawable = None;
            if let Some(ix) = &indices
                && ix.iter().any(|&i| i as usize >= positions.len())
            {
                undrawable = Some(format!("mesh {mi}: index out of range"));
            }
            for (name, len) in [
                ("NORMAL", normals.as_ref().map(Vec::len)),
                ("TEXCOORD_0", uv.as_ref().map(Vec::len)),
                ("TEXCOORD_1", uv1.as_ref().map(Vec::len)),
                ("COLOR_0", colors.as_ref().map(Vec::len)),
                ("TANGENT", tangents.as_ref().map(Vec::len)),
            ] {
                if len.is_some_and(|l| l < positions.len()) {
                    undrawable.get_or_insert(format!("mesh {mi}: {name} is shorter than POSITION"));
                }
            }
            let joints = match attr("JOINTS_0") {
                Some(i) => {
                    let (v, c) = data.read(i as usize)?;
                    if c != 4 {
                        return err("JOINTS_0 must be VEC4");
                    }
                    Some(
                        v.chunks_exact(4)
                            .map(|j| [j[0] as u16, j[1] as u16, j[2] as u16, j[3] as u16])
                            .collect(),
                    )
                }
                None => None,
            };
            let weights = attr("WEIGHTS_0")
                .map(|i| data.read_n::<4>(i as usize))
                .transpose()?;
            let weights_type = match attr("WEIGHTS_0") {
                Some(i) => data.layout(i as usize)?.component_type,
                None => 5126,
            };
            let mut targets = Vec::new();
            for t in p["targets"].as_array().into_iter().flatten() {
                targets.push(MorphTarget {
                    position: t["POSITION"]
                        .as_u64()
                        .map(|i| data.read_n::<3>(i as usize))
                        .transpose()?,
                    normal: t["NORMAL"]
                        .as_u64()
                        .map(|i| data.read_n::<3>(i as usize))
                        .transpose()?,
                });
            }
            if [
                joints.as_ref().map(Vec::len),
                weights.as_ref().map(Vec::len),
            ]
            .iter()
            .any(|l| l.is_some_and(|l| l < positions.len()))
            {
                // three.js reads past the skin attributes and throws.
                return err(format!(
                    "mesh {mi}: skin attributes are shorter than POSITION"
                ));
            }
            if targets.iter().any(|t| {
                [
                    t.position.as_ref().map(Vec::len),
                    t.normal.as_ref().map(Vec::len),
                ]
                .iter()
                .any(|l| l.is_some_and(|l| l < positions.len()))
            }) {
                undrawable.get_or_insert(format!(
                    "mesh {mi}: a morph target is shorter than POSITION"
                ));
            }
            let accessor_bounds = accessor_bounds(raw, pos_index as usize, p["targets"].as_array());
            primitives.push(Primitive {
                topology,
                positions,
                normals,
                uv,
                uv1,
                colors,
                color_components,
                tangents,
                joints,
                weights,
                weights_type,
                indices,
                targets,
                material: p["material"].as_u64().map(|v| v as usize),
                accessor_bounds,
                undrawable,
            });
        }
        let weights = m["weights"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
            .unwrap_or_default();
        model.meshes.push(Mesh {
            name: m["name"].as_str().unwrap_or("").into(),
            primitives,
            weights,
        });
    }
    Ok(())
}

fn load_nodes(raw: &Value, data: &Data<'_>, model: &mut Model) -> Result<()> {
    let nodes = raw["nodes"].as_array().cloned().unwrap_or_default();
    let mut bones = vec![false; nodes.len()];
    for s in raw["skins"].as_array().into_iter().flatten() {
        for j in ids(&s["joints"]) {
            if let Some(b) = bones.get_mut(j) {
                *b = true;
            }
        }
    }
    for (ni, n) in nodes.iter().enumerate() {
        let (translation, rotation, scale) = if let Some(m) = n["matrix"].as_array() {
            // `Matrix4.fromArray`: a missing entry is `undefined` (NaN).
            let mut cols = [f64::NAN; 16];
            for (c, v) in cols.iter_mut().zip(m) {
                *c = crate::gltf::js_number(v);
            }
            math::decompose(&cols)
        } else {
            let t = n["translation"]
                .as_array()
                .map_or([0.0; 3], |a| [f(a, 0), f(a, 1), f(a, 2)]);
            let r = n["rotation"].as_array().map_or([0.0, 0.0, 0.0, 1.0], |a| {
                [f(a, 0), f(a, 1), f(a, 2), f(a, 3)]
            });
            let s = n["scale"]
                .as_array()
                .map_or([1.0; 3], |a| [f(a, 0), f(a, 1), f(a, 2)]);
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
                let count = t
                    .as_ref()
                    .map(|v| v.len() / 3)
                    .or(r.as_ref().map(|v| v.len() / 4))
                    .or(s.as_ref().map(|v| v.len() / 3))
                    .unwrap_or(0);
                let q = |v: f64| v as f32 as f64;
                let mut list = Vec::with_capacity(count);
                for k in 0..count {
                    let tr = t.as_ref().map_or([0.0; 3], |v| {
                        [q(v[3 * k]), q(v[3 * k + 1]), q(v[3 * k + 2])]
                    });
                    let ro = r.as_ref().map_or([0.0, 0.0, 0.0, 1.0], |v| {
                        [
                            q(v[4 * k]),
                            q(v[4 * k + 1]),
                            q(v[4 * k + 2]),
                            q(v[4 * k + 3]),
                        ]
                    });
                    let sc = s.as_ref().map_or([1.0; 3], |v| {
                        [q(v[3 * k]), q(v[3 * k + 1]), q(v[3 * k + 2])]
                    });
                    let m = math::compose(tr, ro, sc);
                    list.push(std::array::from_fn(|i| m[i] as f32));
                }
                Some(list)
            }
            None => None,
        };
        let children = ids(&n["children"]);
        if children.iter().any(|&c| c >= nodes.len() || c == ni) {
            return err(format!("node {ni}: bad child"));
        }
        model.nodes.push(Node {
            name: n["name"].as_str().unwrap_or("").into(),
            parent: None,
            children,
            translation,
            rotation,
            scale,
            mesh: n["mesh"].as_u64().map(|v| v as usize),
            skin: n["skin"].as_u64().map(|v| v as usize),
            camera: n["camera"].as_u64().map(|v| v as usize),
            light: n["extensions"]["KHR_lights_punctual"]["light"]
                .as_u64()
                .map(|v| v as usize),
            weights: n["weights"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()),
            instances,
            is_bone: bones[ni],
        });
    }
    for i in 0..model.nodes.len() {
        for c in model.nodes[i].children.clone() {
            if model.nodes[c].parent.is_some() {
                return err(format!("node {c} has two parents"));
            }
            model.nodes[c].parent = Some(i);
        }
        let n = &model.nodes[i];
        if n.mesh.is_some_and(|m| m >= model.meshes.len())
            || n.skin
                .is_some_and(|s| s >= raw["skins"].as_array().map_or(0, Vec::len))
        {
            return err(format!("node {i}: reference out of range"));
        }
    }
    Ok(())
}

fn load_skins(raw: &Value, data: &Data<'_>, model: &mut Model) -> Result<()> {
    for s in raw["skins"].as_array().into_iter().flatten() {
        let joints = ids(&s["joints"]);
        if joints.iter().any(|&j| j >= model.nodes.len()) {
            return err("skin joint out of range");
        }
        let inverse_bind = match s["inverseBindMatrices"].as_u64() {
            Some(i) => {
                let (v, c) = data.read(i as usize)?;
                if c != 16 {
                    return err("inverseBindMatrices must be MAT4");
                }
                // `mat.fromArray(array, i * 16)` per joint: entries past the
                // array are `undefined` (NaN).
                (0..joints.len())
                    .map(|j| {
                        std::array::from_fn(|k| {
                            v.get(j * 16 + k).map_or(f64::NAN, |x| *x as f32 as f64)
                        })
                    })
                    .collect()
            }
            None => vec![math::IDENTITY; joints.len()],
        };
        model.skins.push(Skin {
            joints,
            inverse_bind,
        });
    }
    Ok(())
}

fn load_clips(raw: &Value, data: &Data<'_>, model: &mut Model) -> Result<()> {
    for a in raw["animations"].as_array().into_iter().flatten() {
        let mut channels = Vec::new();
        let samplers = a["samplers"].as_array().cloned().unwrap_or_default();
        for c in a["channels"].as_array().into_iter().flatten() {
            let target = &c["target"];
            let Some(node) = target["node"].as_u64() else {
                continue;
            };
            if node as usize >= model.nodes.len() {
                return err("animation target out of range");
            }
            let path = match target["path"].as_str().unwrap_or("") {
                "translation" => Path::Translation,
                "rotation" => Path::Rotation,
                "scale" => Path::Scale,
                "weights" => Path::Weights,
                _ => continue,
            };
            let sampler = samplers
                .get(c["sampler"].as_u64().unwrap_or(u64::MAX) as usize)
                .ok_or_else(|| Error("animation sampler".into()))?;
            let interpolation = match sampler["interpolation"].as_str().unwrap_or("LINEAR") {
                "STEP" => Interpolation::Step,
                "CUBICSPLINE" => Interpolation::CubicSpline,
                _ => Interpolation::Linear,
            };
            let (times, _) = data.read(
                sampler["input"]
                    .as_u64()
                    .ok_or_else(|| Error("sampler input".into()))? as usize,
            )?;
            let (values, _) = data.read(
                sampler["output"]
                    .as_u64()
                    .ok_or_else(|| Error("sampler output".into()))? as usize,
            )?;
            let keys = times.len().max(1);
            let per_key = if interpolation == Interpolation::CubicSpline {
                values.len() / keys / 3
            } else {
                values.len() / keys
            };
            channels.push(Channel {
                node: node as usize,
                path,
                interpolation,
                times: times.iter().map(|t| *t as f32).collect(),
                values: values.iter().map(|v| *v as f32).collect(),
                size: per_key,
            });
        }
        let duration = channels
            .iter()
            .filter_map(|c| c.times.last().copied())
            .fold(0.0f32, f32::max);
        model.clips.push(Clip {
            name: a["name"].as_str().unwrap_or("").into(),
            channels,
            duration,
        });
    }
    Ok(())
}

fn load_textures(raw: &Value, data: &Data<'_>, model: &mut Model) -> Result<()> {
    let samplers = raw["samplers"].as_array().cloned().unwrap_or_default();
    for t in raw["textures"].as_array().into_iter().flatten() {
        let ext = &t["extensions"];
        let source = ext["EXT_texture_avif"]["source"]
            .as_u64()
            .or(ext["EXT_texture_webp"]["source"].as_u64())
            .or(t["source"].as_u64());
        let s = t["sampler"].as_u64().and_then(|i| samplers.get(i as usize));
        let wrap = |k: &str| s.and_then(|s| s[k].as_u64()).unwrap_or(10497) as u32;
        model.textures.push(Texture {
            source: source.map(|v| v as usize),
            sampler: Sampler {
                mag_filter: s.and_then(|s| s["magFilter"].as_u64()).map(|v| v as u32),
                min_filter: s.and_then(|s| s["minFilter"].as_u64()).map(|v| v as u32),
                wrap_s: wrap("wrapS"),
                wrap_t: wrap("wrapT"),
            },
        });
    }
    for img in raw["images"].as_array().into_iter().flatten() {
        let (bytes, mime) = if let Some(view) = img["bufferView"].as_u64() {
            let v = &raw["bufferViews"][view as usize];
            let buffer = data
                .buffers
                .get(v["buffer"].as_u64().unwrap_or(0) as usize)
                .ok_or_else(|| Error("image buffer".into()))?;
            let start = v["byteOffset"].as_u64().unwrap_or(0) as usize;
            let len = v["byteLength"].as_u64().unwrap_or(0) as usize;
            (
                buffer
                    .get(start..start + len)
                    .ok_or_else(|| Error("image range".into()))?
                    .to_vec(),
                img["mimeType"].as_str().unwrap_or("").to_string(),
            )
        } else if let Some(uri) = img["uri"].as_str() {
            let mime = uri
                .strip_prefix("data:")
                .and_then(|s| s.split(';').next())
                .unwrap_or("")
                .to_string();
            (decode_data_uri(uri)?, mime)
        } else {
            return err("image without data");
        };
        model.images.push(Image { mime, bytes });
    }
    Ok(())
}

fn accessor_bounds(
    raw: &Value,
    position: usize,
    targets: Option<&Vec<Value>>,
) -> Option<([f64; 3], [f64; 3])> {
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
        let Some(i) = t["POSITION"].as_u64() else {
            continue;
        };
        let ta = &raw["accessors"][i as usize];
        let (Some(tmin), Some(tmax)) = (ta["min"].as_array(), ta["max"].as_array()) else {
            continue;
        };
        let k = scale(ta);
        for (c, d) in disp.iter_mut().enumerate() {
            *d = d.max(f(tmin, c).abs().max(f(tmax, c).abs()) * k);
        }
    }
    for c in 0..3 {
        lo[c] -= disp[c];
        hi[c] += disp[c];
    }
    Some((lo, hi))
}

/// A base64 `data:` URI's bytes.
pub fn decode_data_uri(uri: &str) -> Result<Vec<u8>> {
    let (_, b64) = uri
        .split_once(";base64,")
        .ok_or_else(|| Error("only base64 data URIs are supported".into()))?;
    base64_decode(b64)
}

pub(crate) fn base64_decode(s: &str) -> Result<Vec<u8>> {
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
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
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

#[cfg(test)]
fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for s in [
            &b""[..],
            b"a",
            b"ab",
            b"abc",
            b"abcd",
            &[0u8, 255, 128, 7, 9][..],
        ] {
            assert_eq!(base64_decode(&base64_encode(s)).unwrap(), s);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
    }
}
