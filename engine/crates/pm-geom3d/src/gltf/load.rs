//! `loadGltf` and three.js r180's `GLTFLoader.parse`, as data.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::math::{self, Mat4, Quat, Vec3};
use super::{
    Attribute, Clip, Drawable, Error, Filter, Fit, Geometry, Image, Instances, Interpolation,
    Light, LightKind, MapRef, Material, MaterialType, Mode, Model, Morph, Object, ObjectKind,
    Property, Result, Sampler, Skin, Texture, Track, Wrap, err,
};

/// The importer's ceilings (`loadGltf`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// File size (128 MB).
    pub bytes: usize,
    /// Elements per accessor (1,500,000).
    pub accessor_count: u64,
    /// `POSITION` elements over all primitives (1,000,000).
    pub vertices: u64,
    pub nodes: usize,
    /// Triangles over all meshes (500,000).
    pub triangles: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            bytes: 128 * 1024 * 1024,
            accessor_count: 1_500_000,
            vertices: 1_000_000,
            nodes: 4096,
            triangles: 500_000,
        }
    }
}

/// Loads a GLB or self-contained glTF with today's import ceilings.
pub fn load(bytes: &[u8]) -> Result<Model> {
    load_with(bytes, Limits::default())
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

const GLB_MAGIC: u32 = 0x4654_6c67;
const CHUNK_JSON: u32 = 0x4e4f_534a;

/// The JSON document and the GLB `BIN` chunk, with `loadGltf`'s container
/// checks.
pub(crate) fn container(bytes: &[u8]) -> Result<(Value, Option<Cow<'_, [u8]>>)> {
    if bytes.len() >= 20 && u32_at(bytes, 0) == GLB_MAGIC {
        if u32_at(bytes, 4) != 2 || u32_at(bytes, 8) as usize != bytes.len() {
            return err("Invalid GLB container");
        }
        let length = u32_at(bytes, 12) as usize;
        if u32_at(bytes, 16) != CHUNK_JSON || 20 + length > bytes.len() {
            return err("Invalid GLB scene chunk");
        }
        // The `gltf` crate reads the GLB container; the document is read
        // as raw JSON, as GLTFLoader reads it (the crate's typed schema
        // refuses valid files with extensions it does not model, such as
        // `KHR_animation_pointer` channels without a node).
        let glb = gltf::Glb::from_slice(bytes).map_err(|e| Error(format!("GLB: {e}")))?;
        let json: Value =
            serde_json::from_slice(&glb.json).map_err(|e| Error(format!("GLB JSON: {e}")))?;
        let body = glb.bin;
        Ok((json, body))
    } else {
        let text = std::str::from_utf8(bytes).map_err(|e| Error(format!("glTF JSON: {e}")))?;
        let json: Value =
            serde_json::from_str(text).map_err(|e| Error(format!("glTF JSON: {e}")))?;
        Ok((json, None))
    }
}

/// `loadGltf`'s embedded-resource rule: data URIs of these types only.
fn allowed_uri(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("data:") else {
        return false;
    };
    let Some((mime, data)) = rest.split_once(";base64,") else {
        return false;
    };
    let mime = mime.to_ascii_lowercase();
    matches!(
        mime.as_str(),
        "application/octet-stream"
            | "application/gltf-buffer"
            | "image/png"
            | "image/jpeg"
            | "image/webp"
    ) && !data.is_empty()
        && data.bytes().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=') || c.is_ascii_whitespace()
        })
}

fn decode_data_uri(uri: &str) -> Result<(String, Vec<u8>)> {
    let rest = uri
        .strip_prefix("data:")
        .ok_or_else(|| Error("not a data URI".into()))?;
    let (mime, b64) = rest
        .split_once(";base64,")
        .ok_or_else(|| Error("Invalid embedded model resource".into()))?;
    let mut out = Vec::with_capacity(b64.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in b64.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return err("Invalid embedded model resource"),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok((mime.to_owned(), out))
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn f(a: &Value, i: usize) -> f64 {
    a.get(i).and_then(Value::as_f64).unwrap_or(0.0)
}

fn index(v: &Value) -> Option<usize> {
    v.as_u64().map(|i| i as usize)
}

/// The prefix of the error for an accessor type GLTFLoader has no size for.
const UNKNOWN_TYPE: &str = "accessor type";

/// JavaScript's `ToNumber` of a JSON value, as typed-array and matrix
/// stores apply it.
pub(crate) fn js_number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::String(t) => {
            let t = t.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse().unwrap_or(f64::NAN)
            }
        }
        Value::Array(a) if a.is_empty() => 0.0,
        Value::Array(a) if a.len() == 1 => js_number(&a[0]),
        _ => f64::NAN,
    }
}

/// `PropertyBinding.sanitizeNodeName`.
fn sanitize(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '[' | ']' | '.' | ':' | '/'))
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect()
}

/// `parser.createUniqueName`.
#[derive(Default)]
struct Names(HashMap<String, u32>);

impl Names {
    fn unique(&mut self, original: &str) -> String {
        let s = sanitize(original);
        match self.0.get_mut(&s) {
            Some(n) => {
                *n += 1;
                format!("{s}_{n}")
            }
            None => {
                self.0.insert(s.clone(), 0);
                s
            }
        }
    }
}

struct Doc<'a> {
    json: &'a Value,
    buffers: Vec<Cow<'a, [u8]>>,
}

const TYPES: [(&str, usize); 7] = [
    ("SCALAR", 1),
    ("VEC2", 2),
    ("VEC3", 3),
    ("VEC4", 4),
    ("MAT2", 4),
    ("MAT3", 9),
    ("MAT4", 16),
];

/// An accessor with exact source components (normalized integers kept raw),
/// component type, and item size. `raw`/`get` expose three.js numbers.
enum Components {
    Float(Arc<Vec<f32>>),
    Integer(Vec<u32>),
}

impl Components {
    fn len(&self) -> usize {
        match self {
            Self::Float(values) => values.len(),
            Self::Integer(values) => values.len(),
        }
    }

    fn resize(&mut self, len: usize) {
        match self {
            Self::Float(values) => Arc::make_mut(values).resize(len, 0.0),
            Self::Integer(values) => values.resize(len, 0),
        }
    }
}

/// Accessor assembly is exclusively owned; wrap float storage in an Arc
/// only after base and sparse writes are complete.
enum DecodeBuffer {
    Float(Vec<f32>),
    Integer(Vec<u32>),
}

impl DecodeBuffer {
    fn set_bits(&mut self, index: usize, bits: u32) {
        match self {
            Self::Float(values) => {
                // The old path decoded to f64, then stored in a Float32Array.
                // Keep that round trip (including signaling-NaN quieting).
                values[index] = f64::from(f32::from_bits(bits)) as f32;
            }
            Self::Integer(values) => values[index] = bits,
        }
    }

    fn share(self) -> Components {
        match self {
            Self::Float(values) => Components::Float(Arc::new(values)),
            Self::Integer(values) => Components::Integer(values),
        }
    }
}

struct Raw {
    // A glTF component occupies at most four bytes. Keep its exact source
    // bits until a consumer asks for a JS number: f64 staging doubled the
    // peak size of large POSITION accessors, while f32 staging would lose
    // exact u32 indices above 2^24.
    values: Components,
    size: usize,
    component: u64,
    normalized: bool,
}

impl Raw {
    fn raw(&self, i: usize) -> f64 {
        match &self.values {
            Components::Float(values) => f64::from(values[i]),
            Components::Integer(values) => match self.component {
                5120 | 5122 => f64::from(values[i] as i32),
                _ => f64::from(values[i]),
            },
        }
    }

    /// The value `BufferAttribute.getX` returns (denormalized).
    fn get(&self, i: usize) -> f64 {
        let v = self.raw(i);
        if !self.normalized {
            return v;
        }
        match self.component {
            5120 => (v / 127.0).max(-1.0),
            5121 => v / 255.0,
            5122 => (v / 32767.0).max(-1.0),
            5123 => v / 65535.0,
            _ => v,
        }
    }

    /// The f32 value WebGL's vertex fetch produces.
    fn attribute(&self) -> Attribute {
        Attribute {
            data: match &self.values {
                Components::Float(values) => values.clone(),
                Components::Integer(_) => {
                    Arc::new((0..self.values.len()).map(|i| self.get(i) as f32).collect())
                }
            },
            size: self.size,
        }
    }

    fn count(&self) -> usize {
        self.values.len() / self.size.max(1)
    }

    fn bits(&self) -> Option<u32> {
        match (self.normalized, self.component) {
            (true, 5121) => Some(8),
            (true, 5123) => Some(16),
            _ => None,
        }
    }
}

impl Doc<'_> {
    fn view(&self, i: usize) -> Result<&[u8]> {
        let v = &self.json["bufferViews"][i];
        let b = index(&v["buffer"]).ok_or_else(|| Error("buffer view without buffer".into()))?;
        let buffer = self
            .buffers
            .get(b)
            .ok_or_else(|| Error(format!("buffer {b} missing")))?;
        // `loadBufferView`: `buffer.slice(byteOffset, byteOffset +
        // byteLength)`, which clamps both ends to the buffer; an accessor
        // reading past the shorter view fails below, as its typed array
        // constructor does.
        let start = (v["byteOffset"].as_u64().unwrap_or(0) as usize).min(buffer.len());
        let len = v["byteLength"].as_u64().unwrap_or(0) as usize;
        let end = start.saturating_add(len).min(buffer.len());
        Ok(&buffer[start..end])
    }

    /// `loadAccessor`.
    fn accessor(&self, i: usize) -> Result<Raw> {
        let a = self
            .json
            .get("accessors")
            .and_then(|a| a.get(i))
            .ok_or_else(|| Error(format!("accessor {i} missing")))?;
        let count = a["count"]
            .as_u64()
            .ok_or_else(|| Error("accessor count".into()))? as usize;
        let ty = a["type"].as_str().unwrap_or("");
        let size = TYPES
            .iter()
            .find(|(n, _)| *n == ty)
            .map(|(_, s)| *s)
            .ok_or_else(|| Error(format!("{UNKNOWN_TYPE} {ty}")))?;
        let component = a["componentType"].as_u64().unwrap_or(0);
        let normalized = a["normalized"].as_bool() == Some(true);
        let csize = match component {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            c => return err(format!("accessor component type {c}")),
        };
        let decode = |b: &[u8]| -> u32 {
            match component {
                5120 => i32::from(b[0] as i8) as u32,
                5121 => u32::from(b[0]),
                5122 => i32::from(i16::from_le_bytes([b[0], b[1]])) as u32,
                5123 => u32::from(u16::from_le_bytes([b[0], b[1]])),
                _ => u32_at(b, 0),
            }
        };
        let mut values = if component == 5126 {
            DecodeBuffer::Float(vec![0.0; count * size])
        } else {
            DecodeBuffer::Integer(vec![0; count * size])
        };
        if let Some(view_index) = index(&a["bufferView"]) {
            let view = self.view(view_index)?;
            let offset = a["byteOffset"].as_u64().unwrap_or(0) as usize;
            // Matrix columns of 1 and 2-byte components are 4-byte aligned.
            let column = |rows: usize| (rows * csize).div_ceil(4) * 4;
            let (columns, rows) = match ty {
                "MAT2" => (2, 2),
                "MAT3" => (3, 3),
                "MAT4" => (4, 4),
                _ => (1, size),
            };
            let element = if columns > 1 {
                columns * column(rows)
            } else {
                size * csize
            };
            let stride = self.json["bufferViews"][view_index]["byteStride"]
                .as_u64()
                .map_or(element, |s| s as usize);
            let needed = if count == 0 {
                0
            } else {
                offset + stride * (count - 1) + element
            };
            if needed > view.len() {
                return err(format!("accessor {i} reads past its buffer view"));
            }
            for e in 0..count {
                let base = offset + e * stride;
                for c in 0..size {
                    let at = if columns > 1 {
                        (c / rows) * column(rows) + (c % rows) * csize
                    } else {
                        c * csize
                    };
                    values.set_bits(e * size + c, decode(&view[base + at..]));
                }
            }
        }
        if let Some(sparse) = a.get("sparse") {
            let n = sparse["count"].as_u64().unwrap_or(0) as usize;
            let read = |v: &Value| -> Result<&[u8]> {
                let view =
                    self.view(index(&v["bufferView"]).ok_or_else(|| Error("sparse view".into()))?)?;
                let at = v["byteOffset"].as_u64().unwrap_or(0) as usize;
                view.get(at..).ok_or_else(|| Error("sparse range".into()))
            };
            let ib = read(&sparse["indices"])?;
            let vb = read(&sparse["values"])?;
            let isize = match sparse["indices"]["componentType"].as_u64().unwrap_or(0) {
                5121 => 1,
                5123 => 2,
                5125 => 4,
                c => return err(format!("sparse index type {c}")),
            };
            if ib.len() < n * isize || vb.len() < n * size * csize {
                return err("sparse accessor reads past its buffer view");
            }
            for k in 0..n {
                let target = match isize {
                    1 => ib[k] as usize,
                    2 => u16::from_le_bytes([ib[2 * k], ib[2 * k + 1]]) as usize,
                    _ => u32_at(ib, 4 * k) as usize,
                };
                // `setX` past the end of the typed array writes nothing.
                if target >= count {
                    continue;
                }
                for c in 0..size.min(4) {
                    values.set_bits(target * size + c, decode(&vb[(k * size + c) * csize..]));
                }
            }
        }
        Ok(Raw {
            values: values.share(),
            size,
            component,
            normalized,
        })
    }
}

/// What extensions the loader has (`GLTFLoader.parse`'s `extensions`).
struct Extensions {
    unlit: bool,
    texture_transform: bool,
}

/// `GLTFTextureTransformExtension.extendTexture` then `setUvTransform`
/// (`texture.matrix`; center 0).
fn uv_matrix(offset: [f64; 2], repeat: [f64; 2], rotation: f64) -> [f64; 9] {
    let c = libm::cos(rotation);
    let s = libm::sin(rotation);
    let (sx, sy) = (repeat[0], repeat[1]);
    let n11 = sx * c;
    let n12 = sx * s;
    let n13 = -sx * (c * 0.0 + s * 0.0) + 0.0 + offset[0];
    let n21 = -sy * s;
    let n22 = sy * c;
    let n23 = -sy * (-s * 0.0 + c * 0.0) + 0.0 + offset[1];
    [n11, n21, 0.0, n12, n22, 0.0, n13, n23, 1.0]
}

/// The identity `texture.matrix`.
const UV_IDENTITY: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

struct Loader<'a> {
    doc: Doc<'a>,
    ext: Extensions,
    names: Names,
    /// Texture index → glTF texture (`None`: the texture does not load).
    textures: Vec<Option<Texture>>,
    images: Vec<Image>,
    objects: Vec<Object>,
    geometries: Vec<Geometry>,
    primitive_cache: HashMap<String, usize>,
    /// Source materials (`None`: the default material).
    source_materials: HashMap<Option<usize>, Material>,
    materials: Vec<Material>,
    /// The source material of each final material.
    material_sources: Vec<Option<usize>>,
    final_cache: HashMap<String, usize>,
    lights: Vec<Light>,
    light_defs: Vec<Option<usize>>,
    /// glTF node → its object.
    node_objects: HashMap<usize, usize>,
    selected_parents: Vec<Option<usize>>,
    selected_roots: Vec<usize>,
    /// Per mesh: how many nodes reference it, and uses so far.
    mesh_refs: HashMap<usize, (u32, u32)>,
    light_refs: HashMap<usize, (u32, u32)>,
    camera_refs: HashMap<usize, (u32, u32)>,
    /// Mesh objects built once per mesh (name, kind) before `_getNodeRef`.
    meshes: HashMap<usize, MeshTemplate>,
    skinned_meshes: Vec<bool>,
    bones: Vec<bool>,
    skins_out: Vec<Skin>,
    /// Light objects `_getNodeRef` cloned (several nodes use the light).
    cloned_lights: Vec<usize>,
}

/// A loaded mesh: one object, or a group of primitive objects.
#[derive(Clone)]
struct MeshTemplate {
    /// (name, drawable) per primitive.
    parts: Vec<(String, Drawable)>,
    group: bool,
    /// The group's name (a mesh with several primitives).
    name: String,
}

fn map_def(v: &Value) -> Option<&Value> {
    v.get("index").filter(|i| i.is_u64()).map(|_| v)
}

impl Loader<'_> {
    fn add(&mut self, name: String, kind: ObjectKind) -> usize {
        self.objects.push(Object {
            name,
            kind,
            parent: None,
            children: Vec::new(),
            position: [0.0; 3],
            quaternion: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        });
        self.objects.len() - 1
    }

    fn attach(&mut self, parent: usize, child: usize) {
        if let Some(old) = self.objects[child].parent {
            self.objects[old].children.retain(|&c| c != child);
        }
        self.objects[child].parent = Some(parent);
        self.objects[parent].children.push(child);
    }

    /// `parser.assignTexture` (the map, or `None` when the texture does not
    /// load).
    fn texture(&self, def: &Value) -> Option<MapRef> {
        let t = index(&def["index"])?;
        self.textures.get(t)?.as_ref()?;
        let mut channel = 0;
        if let Some(c) = def["texCoord"].as_u64()
            && c > 0
        {
            channel = c as u32;
        }
        let mut transform = UV_IDENTITY;
        if self.ext.texture_transform
            && let Some(tt) = def
                .get("extensions")
                .and_then(|e| e.get("KHR_texture_transform"))
        {
            let tex_coord = tt["texCoord"].as_u64();
            let untouched = tex_coord.is_none_or(|c| c as u32 == channel)
                && tt.get("offset").is_none()
                && tt.get("rotation").is_none()
                && tt.get("scale").is_none();
            if !untouched {
                if let Some(c) = tex_coord {
                    channel = c as u32;
                }
                let offset = tt.get("offset").map_or([0.0, 0.0], |o| [f(o, 0), f(o, 1)]);
                let rotation = tt["rotation"].as_f64().unwrap_or(0.0);
                let repeat = tt.get("scale").map_or([1.0, 1.0], |s| [f(s, 0), f(s, 1)]);
                transform = uv_matrix(offset, repeat, rotation);
            }
        }
        Some(MapRef {
            texture: t,
            channel,
            transform,
        })
    }

    fn color3(v: &Value, default: Vec3) -> Vec3 {
        match v.as_array() {
            Some(a) if a.len() >= 3 => [f(v, 0), f(v, 1), f(v, 2)],
            _ => default,
        }
    }

    /// `loadMaterial`.
    fn source_material(&self, i: usize) -> Result<Material> {
        let def = self
            .doc
            .json
            .get("materials")
            .and_then(|m| m.get(i))
            .ok_or_else(|| Error(format!("material {i} missing")))?;
        let mut m = Material::standard();
        let exts = def.get("extensions").cloned().unwrap_or(Value::Null);
        let has = |name: &str| exts.get(name).is_some_and(|e| !e.is_null());
        let pbr = def
            .get("pbrMetallicRoughness")
            .cloned()
            .unwrap_or(Value::Null);
        if has("KHR_materials_unlit") {
            if !self.ext.unlit {
                // `extensions[KHR_MATERIALS_UNLIT]` is undefined: the loader throws.
                return err("KHR_materials_unlit is used but not declared");
            }
            m.kind = MaterialType::Basic;
            if let Some(c) = pbr.get("baseColorFactor").filter(|c| c.is_array()) {
                m.color = [f(c, 0), f(c, 1), f(c, 2)];
                m.opacity = f(c, 3);
            }
            if let Some(t) = pbr.get("baseColorTexture").and_then(map_def) {
                m.map = self.texture(t);
            }
        } else {
            if let Some(c) = pbr.get("baseColorFactor").filter(|c| c.is_array()) {
                m.color = [f(c, 0), f(c, 1), f(c, 2)];
                m.opacity = f(c, 3);
            }
            if let Some(t) = pbr.get("baseColorTexture").and_then(map_def) {
                m.map = self.texture(t);
            }
            m.metalness = pbr["metallicFactor"].as_f64().unwrap_or(1.0);
            m.roughness = pbr["roughnessFactor"].as_f64().unwrap_or(1.0);
            if let Some(t) = pbr.get("metallicRoughnessTexture").and_then(map_def) {
                m.metalness_map = self.texture(t);
                m.roughness_map = self.texture(t);
            }
            // `getMaterialType`: any of these extensions makes a physical material.
            let physical = [
                "KHR_materials_clearcoat",
                "KHR_materials_dispersion",
                "KHR_materials_sheen",
                "KHR_materials_transmission",
                "KHR_materials_volume",
                "KHR_materials_ior",
                "KHR_materials_specular",
                "KHR_materials_iridescence",
                "KHR_materials_anisotropy",
                "EXT_materials_bump",
            ];
            if physical.iter().any(|e| has(e)) {
                m.kind = MaterialType::Physical;
            }
            // `extendMaterialParams` in plugin registration order.
            if let Some(e) = exts.get("KHR_materials_clearcoat").filter(|e| !e.is_null()) {
                if let Some(v) = num(&e["clearcoatFactor"]) {
                    m.clearcoat = v;
                }
                if let Some(t) = e.get("clearcoatTexture").and_then(map_def) {
                    m.clearcoat_map = self.texture(t);
                }
                if let Some(v) = num(&e["clearcoatRoughnessFactor"]) {
                    m.clearcoat_roughness = v;
                }
                if let Some(t) = e.get("clearcoatRoughnessTexture").and_then(map_def) {
                    m.clearcoat_roughness_map = self.texture(t);
                }
                if let Some(t) = e.get("clearcoatNormalTexture").and_then(map_def) {
                    m.clearcoat_normal_map = self.texture(t);
                    if let Some(s) = num(&t["scale"]) {
                        m.clearcoat_normal_scale = [s, s];
                    }
                }
            }
            if let Some(e) = exts
                .get("KHR_materials_dispersion")
                .filter(|e| !e.is_null())
            {
                m.dispersion = num(&e["dispersion"]).unwrap_or(0.0);
            }
            if let Some(e) = exts.get("KHR_materials_sheen").filter(|e| !e.is_null()) {
                m.sheen_color = [0.0; 3];
                m.sheen_roughness = 0.0;
                m.sheen = 1.0;
                if e.get("sheenColorFactor").is_some() {
                    m.sheen_color = Self::color3(&e["sheenColorFactor"], [0.0; 3]);
                }
                if let Some(v) = num(&e["sheenRoughnessFactor"]) {
                    m.sheen_roughness = v;
                }
                if let Some(t) = e.get("sheenColorTexture").and_then(map_def) {
                    m.sheen_color_map = self.texture(t);
                }
                if let Some(t) = e.get("sheenRoughnessTexture").and_then(map_def) {
                    m.sheen_roughness_map = self.texture(t);
                }
            }
            if let Some(e) = exts
                .get("KHR_materials_transmission")
                .filter(|e| !e.is_null())
            {
                if let Some(v) = num(&e["transmissionFactor"]) {
                    m.transmission = v;
                }
                if let Some(t) = e.get("transmissionTexture").and_then(map_def) {
                    m.transmission_map = self.texture(t);
                }
            }
            if let Some(e) = exts.get("KHR_materials_volume").filter(|e| !e.is_null()) {
                m.thickness = num(&e["thicknessFactor"]).unwrap_or(0.0);
                if let Some(t) = e.get("thicknessTexture").and_then(map_def) {
                    m.thickness_map = self.texture(t);
                }
                // `extension.attenuationDistance || Infinity`.
                m.attenuation_distance = match num(&e["attenuationDistance"]) {
                    Some(d) if d != 0.0 && !d.is_nan() => d,
                    _ => f64::INFINITY,
                };
                m.attenuation_color = Self::color3(&e["attenuationColor"], [1.0; 3]);
            }
            if let Some(e) = exts.get("KHR_materials_ior").filter(|e| !e.is_null()) {
                m.ior = num(&e["ior"]).unwrap_or(1.5);
            }
            if let Some(e) = exts
                .get("KHR_materials_emissive_strength")
                .filter(|e| !e.is_null())
                && let Some(v) = num(&e["emissiveStrength"])
            {
                m.emissive_intensity = v;
            }
            if let Some(e) = exts.get("KHR_materials_specular").filter(|e| !e.is_null()) {
                m.specular_intensity = num(&e["specularFactor"]).unwrap_or(1.0);
                if let Some(t) = e.get("specularTexture").and_then(map_def) {
                    m.specular_intensity_map = self.texture(t);
                }
                m.specular_color = Self::color3(&e["specularColorFactor"], [1.0; 3]);
                if let Some(t) = e.get("specularColorTexture").and_then(map_def) {
                    m.specular_color_map = self.texture(t);
                }
            }
            if let Some(e) = exts
                .get("KHR_materials_iridescence")
                .filter(|e| !e.is_null())
            {
                if let Some(v) = num(&e["iridescenceFactor"]) {
                    m.iridescence = v;
                }
                if let Some(t) = e.get("iridescenceTexture").and_then(map_def) {
                    m.iridescence_map = self.texture(t);
                }
                if let Some(v) = num(&e["iridescenceIor"]) {
                    m.iridescence_ior = v;
                }
                m.iridescence_thickness_range = [100.0, 400.0];
                if let Some(v) = num(&e["iridescenceThicknessMinimum"]) {
                    m.iridescence_thickness_range[0] = v;
                }
                if let Some(v) = num(&e["iridescenceThicknessMaximum"]) {
                    m.iridescence_thickness_range[1] = v;
                }
                if let Some(t) = e.get("iridescenceThicknessTexture").and_then(map_def) {
                    m.iridescence_thickness_map = self.texture(t);
                }
            }
            if let Some(e) = exts
                .get("KHR_materials_anisotropy")
                .filter(|e| !e.is_null())
            {
                if let Some(v) = num(&e["anisotropyStrength"]) {
                    m.anisotropy = v;
                }
                if let Some(v) = num(&e["anisotropyRotation"]) {
                    m.anisotropy_rotation = v;
                }
                if let Some(t) = e.get("anisotropyTexture").and_then(map_def) {
                    m.anisotropy_map = self.texture(t);
                }
            }
            if let Some(e) = exts.get("EXT_materials_bump").filter(|e| !e.is_null()) {
                m.bump_scale = num(&e["bumpFactor"]).unwrap_or(1.0);
                if let Some(t) = e.get("bumpTexture").and_then(map_def) {
                    m.bump_map = self.texture(t);
                }
            }
        }
        if def["doubleSided"].as_bool() == Some(true) {
            m.double_sided = true;
        }
        match def["alphaMode"].as_str().unwrap_or("OPAQUE") {
            "BLEND" => {
                m.transparent = true;
                m.depth_write = false;
            }
            mode => {
                m.transparent = false;
                if mode == "MASK" {
                    m.alpha_test = def["alphaCutoff"].as_f64().unwrap_or(0.5);
                }
            }
        }
        let basic = m.kind == MaterialType::Basic;
        if !basic {
            if let Some(t) = def.get("normalTexture").and_then(map_def) {
                m.normal_map = self.texture(t);
                m.normal_scale = [1.0, 1.0];
                if let Some(s) = num(&t["scale"]) {
                    m.normal_scale = [s, s];
                }
            }
            if let Some(t) = def.get("occlusionTexture").and_then(map_def) {
                m.ao_map = self.texture(t);
                if let Some(s) = num(&t["strength"]) {
                    m.ao_map_intensity = s;
                }
            }
            if def.get("emissiveFactor").is_some_and(Value::is_array) {
                m.emissive = Self::color3(&def["emissiveFactor"], [0.0; 3]);
            }
            if let Some(t) = def.get("emissiveTexture").and_then(map_def) {
                m.emissive_map = self.texture(t);
            }
        }
        if let Some(name) = def["name"].as_str()
            && !name.is_empty()
        {
            m.name = name.to_owned();
        }
        Ok(m)
    }

    /// The final material of a drawable (`assignFinalMaterial`, applied
    /// `passes` times: an instanced mesh runs it again on the final
    /// material).
    fn final_material(
        &mut self,
        source: Option<usize>,
        mode: Mode,
        geometry: usize,
        passes: u32,
    ) -> Result<usize> {
        let g = &self.geometries[geometry];
        let derivative = g.tangent.is_none();
        let colors = g.color.is_some();
        let flat = g.normal.is_none();
        let key = format!("{source:?}:{mode:?}:{derivative}:{colors}:{flat}:{passes}");
        if let Some(&i) = self.final_cache.get(&key) {
            return Ok(i);
        }
        if !self.source_materials.contains_key(&source) {
            let m = match source {
                Some(i) => self.source_material(i)?,
                None => Material::gltf_default(),
            };
            self.source_materials.insert(source, m);
        }
        let src = &self.source_materials[&source];
        let mut m = match mode {
            Mode::Points | Mode::LineSegments | Mode::LineStrip | Mode::LineLoop => {
                // `Material.prototype.copy` (base properties), color, map.
                let mut p = Material::standard();
                p.kind = if mode == Mode::Points {
                    MaterialType::Points
                } else {
                    MaterialType::Line
                };
                p.name = src.name.clone();
                p.double_sided = src.double_sided;
                p.vertex_colors = src.vertex_colors;
                p.opacity = src.opacity;
                p.transparent = src.transparent;
                p.depth_write = src.depth_write;
                p.alpha_test = src.alpha_test;
                p.color = src.color;
                p.map = src.map.clone();
                if mode == Mode::Points {
                    p.size_attenuation = false;
                }
                p
            }
            Mode::Triangles => src.clone(),
        };
        for _ in 0..passes {
            if derivative || colors || flat {
                if colors {
                    m.vertex_colors = true;
                }
                if flat {
                    m.flat_shading = true;
                }
                if derivative && matches!(m.kind, MaterialType::Standard | MaterialType::Physical) {
                    m.normal_scale[1] *= -1.0;
                    if m.kind == MaterialType::Physical {
                        m.clearcoat_normal_scale[1] *= -1.0;
                    }
                }
            }
        }
        self.materials.push(m);
        self.material_sources.push(source);
        let i = self.materials.len() - 1;
        self.final_cache.insert(key, i);
        Ok(i)
    }

    /// `addPrimitiveAttributes` (with `toTrianglesDrawMode`), cached by
    /// `createPrimitiveKey`.
    fn geometry(&mut self, p: &Value) -> Result<usize> {
        let attrs = p.get("attributes").cloned().unwrap_or(Value::Null);
        let attributes_key = |a: &Value| {
            let mut keys: Vec<(&String, &Value)> = a
                .as_object()
                .map(|o| o.iter().collect())
                .unwrap_or_default();
            keys.sort_by(|x, y| x.0.cmp(y.0));
            keys.iter()
                .map(|(k, v)| format!("{k}:{v};"))
                .collect::<String>()
        };
        let js = |v: &Value| {
            if v.is_null() {
                "undefined".to_owned()
            } else {
                v.to_string()
            }
        };
        let mut key = format!(
            "{}:{}:{}",
            js(&p["indices"]),
            attributes_key(&attrs),
            js(&p["mode"])
        );
        for t in p["targets"].as_array().into_iter().flatten() {
            key.push(':');
            key.push_str(&attributes_key(t));
        }
        if let Some(&g) = self.primitive_cache.get(&key) {
            return Ok(g);
        }
        if p.get("extensions")
            .and_then(|e| e.get("KHR_draco_mesh_compression"))
            .is_some()
        {
            return err("Draco-compressed glTF is not supported");
        }
        // An accessor of a type GLTFLoader has no size for
        // (`WEBGL_TYPE_SIZES[type]` undefined) becomes an attribute with no
        // elements, which no vertex reads: the attribute is absent here. A
        // POSITION without elements leaves the model with no vertices,
        // which `loadGltf` refuses.
        let read = |name: &str| -> Result<Option<Raw>> {
            match index(&attrs[name]) {
                Some(i) => match self.doc.accessor(i) {
                    Ok(a) => Ok(Some(a)),
                    Err(e) if e.0.starts_with(UNKNOWN_TYPE) && name != "POSITION" => Ok(None),
                    Err(e) => Err(e),
                },
                None => Ok(None),
            }
        };
        let position =
            read("POSITION")?.ok_or_else(|| Error("a primitive without POSITION".into()))?;
        if position.size != 3 {
            return err("POSITION must be VEC3");
        }
        let count = position.count();
        let check = |a: &Option<Raw>, name: &str, sizes: &[usize]| -> Result<()> {
            if let Some(a) = a
                && !sizes.contains(&a.size)
            {
                return err(format!("{name} has {} components", a.size));
            }
            Ok(())
        };
        // Keep the source lengths for the final shader's active-attribute
        // decision. CPU poses use padded values; an inactive short attribute
        // does not make WebGL refuse the draw. Short skin attributes throw
        // in `setFromObject` (below).
        let mut short: Vec<(&'static str, usize)> = Vec::new();
        let mut pad = |a: Option<Raw>, name: &'static str| -> Option<Raw> {
            a.map(|mut a| {
                if a.count() < count {
                    short.push((name, a.count()));
                    a.values.resize(count * a.size);
                }
                a
            })
        };
        let normal = pad(read("NORMAL")?, "NORMAL");
        check(&normal, "NORMAL", &[3])?;
        let tangent = pad(read("TANGENT")?, "TANGENT");
        check(&tangent, "TANGENT", &[4])?;
        let mut uv: [Option<Attribute>; 4] = Default::default();
        for (k, slot) in uv.iter_mut().enumerate() {
            let name = ["TEXCOORD_0", "TEXCOORD_1", "TEXCOORD_2", "TEXCOORD_3"][k];
            let a = pad(read(name)?, name);
            check(&a, "TEXCOORD", &[2])?;
            *slot = a.map(|a| a.attribute());
        }
        let color = pad(read("COLOR_0")?, "COLOR_0");
        check(&color, "COLOR_0", &[3, 4])?;
        let joints = read("JOINTS_0")?;
        check(&joints, "JOINTS_0", &[4])?;
        if joints.as_ref().is_some_and(|a| a.count() < count) {
            return err("JOINTS_0 has fewer elements than POSITION");
        }
        let weights = read("WEIGHTS_0")?;
        check(&weights, "WEIGHTS_0", &[4])?;
        if weights.as_ref().is_some_and(|a| a.count() < count) {
            return err("WEIGHTS_0 has fewer elements than POSITION");
        }
        let mut indices = match index(&p["indices"]) {
            Some(i) => {
                let a = self.doc.accessor(i)?;
                Some(
                    (0..a.values.len())
                        .map(|k| a.raw(k) as u32)
                        .collect::<Vec<u32>>(),
                )
            }
            None => None,
        };
        let mode = match p["mode"].as_u64() {
            None | Some(4) => Mode::Triangles,
            Some(0) => Mode::Points,
            Some(1) => Mode::LineSegments,
            Some(2) => Mode::LineLoop,
            Some(3) => Mode::LineStrip,
            Some(m @ (5 | 6)) => {
                // `toTrianglesDrawMode`.
                let src = indices
                    .clone()
                    .unwrap_or_else(|| (0..count as u32).collect());
                let n = src.len().saturating_sub(2);
                let mut tris = Vec::with_capacity(n * 3);
                if m == 6 {
                    for i in 1..=n {
                        tris.extend_from_slice(&[src[0], src[i], src[i + 1]]);
                    }
                } else {
                    for i in 0..n {
                        if i % 2 == 0 {
                            tris.extend_from_slice(&[src[i], src[i + 1], src[i + 2]]);
                        } else {
                            tris.extend_from_slice(&[src[i + 2], src[i + 1], src[i]]);
                        }
                    }
                }
                indices = Some(tris);
                Mode::Triangles
            }
            Some(m) => return err(format!("Primitive mode unsupported: {m}")),
        };
        // An index past the last vertex loads today (the import facts and
        // bounds read positions only); WebGL refuses to draw the primitive.
        if let Some(ix) = &indices
            && ix.iter().any(|&i| i as usize >= count)
        {
            short.insert(0, ("an index past the last vertex", 0));
        }
        // `computeBounds`.
        let bounding_box = {
            let a = &self.doc.json["accessors"][index(&attrs["POSITION"]).unwrap_or(0)];
            let scale = |a: &Value| -> f64 {
                if a["normalized"].as_bool() != Some(true) {
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
            match (a.get("min"), a.get("max")) {
                (Some(min), Some(max)) if min.is_array() && max.is_array() => {
                    let k = scale(a);
                    let mut lo = [f(min, 0), f(min, 1), f(min, 2)];
                    let mut hi = [f(max, 0), f(max, 1), f(max, 2)];
                    if k != 1.0 {
                        for c in 0..3 {
                            lo[c] *= k;
                            hi[c] *= k;
                        }
                    }
                    let mut disp = [0f64; 3];
                    for t in p["targets"].as_array().into_iter().flatten() {
                        let Some(i) = index(&t["POSITION"]) else {
                            continue;
                        };
                        let ta = &self.doc.json["accessors"][i];
                        let (Some(tmin), Some(tmax)) = (
                            ta.get("min").filter(|v| v.is_array()),
                            ta.get("max").filter(|v| v.is_array()),
                        ) else {
                            continue;
                        };
                        let mut v = [
                            f(tmin, 0).abs().max(f(tmax, 0).abs()),
                            f(tmin, 1).abs().max(f(tmax, 1).abs()),
                            f(tmin, 2).abs().max(f(tmax, 2).abs()),
                        ];
                        let k = scale(ta);
                        if k != 1.0 {
                            for x in &mut v {
                                *x *= k;
                            }
                        }
                        for c in 0..3 {
                            disp[c] = disp[c].max(v[c]);
                        }
                    }
                    for c in 0..3 {
                        lo[c] -= disp[c];
                        hi[c] += disp[c];
                    }
                    Some((lo, hi))
                }
                _ => None,
            }
        };
        // `addMorphTargets`.
        let mut morph = Morph::default();
        if let Some(targets) = p["targets"].as_array() {
            let has = |n: &str| targets.iter().any(|t| t.get(n).is_some());
            let (hp, hn, hc) = (has("POSITION"), has("NORMAL"), has("COLOR_0"));
            let list = |name: &str, base: Option<&Raw>| -> Result<Vec<Attribute>> {
                let mut out = Vec::new();
                for t in targets {
                    match index(&t[name]) {
                        Some(i) => out.push(self.doc.accessor(i)?.attribute()),
                        // A missing target attribute is the base attribute
                        // itself (relative: added as is).
                        None => out.push(base.map(Raw::attribute).unwrap_or(Attribute {
                            data: Arc::new(vec![0.0; count * 3]),
                            size: 3,
                        })),
                    }
                }
                Ok(out)
            };
            if hp {
                morph.position = Some(list("POSITION", Some(&position))?);
                morph.source_counts[0] = morph
                    .position
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(Attribute::count)
                    .collect();
            }
            if hn {
                morph.normal = Some(list("NORMAL", normal.as_ref())?);
                morph.source_counts[1] = morph
                    .normal
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(Attribute::count)
                    .collect();
            }
            if hc {
                morph.color = Some(list("COLOR_0", color.as_ref())?);
                morph.source_counts[2] = morph
                    .color
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(Attribute::count)
                    .collect();
            }
        }
        // WebGLMorphtargets packs morph attributes into a texture in JS.
        // Short targets therefore do not bind a short vertex buffer.
        for targets in [&mut morph.position, &mut morph.normal, &mut morph.color] {
            for t in targets.iter_mut().flatten() {
                if t.data.len() < count * t.size {
                    Arc::make_mut(&mut t.data).resize(count * t.size, 0.0);
                }
            }
        }
        let g = Geometry {
            position: position.attribute(),
            normal: normal.map(|a| a.attribute()),
            tangent: tangent.map(|a| a.attribute()),
            uv,
            color: color.map(|a| a.attribute()),
            skin_index: joints.map(|a| Attribute {
                data: Arc::new((0..a.values.len()).map(|k| a.raw(k) as f32).collect()),
                size: 4,
            }),
            skin_weight_bits: weights.as_ref().and_then(Raw::bits),
            skin_weight: weights.map(|a| a.attribute()),
            index: indices.map(Arc::new),
            morph,
            bounding_box,
            mode,
            undrawable: short
                .iter()
                .find(|(name, _)| name.starts_with("an index"))
                .map(|(name, _)| (*name).to_owned()),
            short_attributes: short
                .into_iter()
                .filter(|(name, _)| !name.starts_with("an index"))
                .collect(),
        };
        self.geometries.push(g);
        let i = self.geometries.len() - 1;
        self.primitive_cache.insert(key, i);
        Ok(i)
    }

    /// `SkinnedMesh.normalizeSkinWeights` on a geometry (in place, as on the
    /// loader's shared geometry).
    fn normalize_skin_weights(&mut self, geometry: usize) {
        let g = &mut self.geometries[geometry];
        let bits = g.skin_weight_bits;
        let Some(w) = &mut g.skin_weight else {
            return;
        };
        for v in Arc::make_mut(&mut w.data).chunks_exact_mut(4) {
            let x = [
                f64::from(v[0]),
                f64::from(v[1]),
                f64::from(v[2]),
                f64::from(v[3]),
            ];
            let l = x[0].abs() + x[1].abs() + x[2].abs() + x[3].abs();
            let scale = 1.0 / l;
            let n = if scale.is_infinite() {
                [1.0, 0.0, 0.0, 0.0]
            } else {
                [x[0] * scale, x[1] * scale, x[2] * scale, x[3] * scale]
            };
            for k in 0..4 {
                v[k] = match bits {
                    // `setXYZW` normalizes into the integer array.
                    Some(8) => ((n[k] * 255.0).round() / 255.0) as f32,
                    Some(16) => ((n[k] * 65535.0).round() / 65535.0) as f32,
                    _ => n[k] as f32,
                };
            }
        }
    }

    /// `loadMesh` (names, drawables), once per mesh.
    fn mesh(&mut self, mi: usize) -> Result<MeshTemplate> {
        if let Some(t) = self.meshes.get(&mi) {
            return Ok(t.clone());
        }
        let def = self
            .doc
            .json
            .get("meshes")
            .and_then(|m| m.get(mi))
            .cloned()
            .ok_or_else(|| Error(format!("mesh {mi} missing")))?;
        let skinned = self.skinned_meshes.get(mi).copied().unwrap_or(false);
        let mut parts = Vec::new();
        let base_name = def["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .map_or_else(|| format!("mesh_{mi}"), str::to_owned);
        let weights: Vec<f64> = def["weights"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())
            .unwrap_or_default();
        for p in def["primitives"].as_array().into_iter().flatten() {
            let g = self.geometry(p)?;
            let mode = self.geometries[g].mode;
            let triangles = mode == Mode::Triangles;
            if skinned && triangles {
                self.normalize_skin_weights(g);
            }
            let material = self.final_material(index(&p["material"]), mode, g, 1)?;
            let morph = self.geometries[g].morph.count();
            let mut influences = vec![0.0; morph];
            if morph > 0 {
                for (k, w) in weights.iter().enumerate() {
                    if k < influences.len() {
                        influences[k] = *w;
                    } else {
                        influences.push(*w);
                    }
                }
            }
            let name = self.names.unique(&base_name);
            parts.push((
                name,
                Drawable {
                    geometry: g,
                    material,
                    skin: None,
                    skinned: skinned && triangles,
                    instances: None,
                    morph_influences: influences,
                },
            ));
        }
        let group = parts.len() != 1;
        let t = MeshTemplate {
            parts,
            group,
            name: String::new(),
        };
        self.meshes.insert(mi, t.clone());
        Ok(t)
    }

    /// `_getNodeRef`: a reference after the first of an object several
    /// nodes use gets a copy named `<name>_instance_<n>`; with one
    /// reference the object itself.
    fn node_ref(refs: &mut HashMap<usize, (u32, u32)>, i: usize) -> Option<u32> {
        let e = refs.get_mut(&i)?;
        if e.0 <= 1 {
            return None;
        }
        let n = e.1;
        e.1 += 1;
        Some(n)
    }

    /// `_loadLight` (its object and target).
    fn light(&mut self, li: usize) -> Result<usize> {
        let defs = self.doc.json["extensions"]["KHR_lights_punctual"]["lights"].clone();
        let def = defs
            .get(li)
            .ok_or_else(|| Error(format!("light {li} missing")))?;
        let color = Self::color3(&def["color"], [1.0; 3]);
        let range = def["range"].as_f64().unwrap_or(0.0);
        let (kind, angle, penumbra) = match def["type"].as_str().unwrap_or("") {
            "directional" => (LightKind::Directional, std::f64::consts::FRAC_PI_3, 0.0),
            "point" => (LightKind::Point, std::f64::consts::FRAC_PI_3, 0.0),
            "spot" => {
                let inner = def["spot"]["innerConeAngle"].as_f64().unwrap_or(0.0);
                let outer = def["spot"]["outerConeAngle"]
                    .as_f64()
                    .unwrap_or(std::f64::consts::PI / 4.0);
                (LightKind::Spot, outer, 1.0 - inner / outer)
            }
            t => return err(format!("Unexpected light type: {t}")),
        };
        let light = Light {
            kind,
            color,
            intensity: def["intensity"].as_f64().unwrap_or(1.0),
            distance: if kind == LightKind::Directional {
                0.0
            } else {
                range
            },
            angle,
            penumbra,
        };
        self.lights.push(light);
        let index = self.lights.len() - 1;
        let name = def["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .map_or_else(|| format!("light_{li}"), str::to_owned);
        let name = self.names.unique(&name);
        let o = self.add(name, ObjectKind::Light(index));
        if kind != LightKind::Point {
            let t = self.add(String::new(), ObjectKind::LightTarget);
            self.objects[t].position = [0.0, 0.0, -1.0];
            self.attach(o, t);
        }
        Ok(o)
    }

    /// Copies an object subtree (`object.clone()`), renaming the root.
    fn clone_tree(&mut self, root: usize, name: String) -> usize {
        let o = self.objects[root].clone();
        let copy = self.add(name, o.kind.clone());
        self.objects[copy].position = o.position;
        self.objects[copy].quaternion = o.quaternion;
        self.objects[copy].scale = o.scale;
        for c in o.children {
            let n = self.objects[c].name.clone();
            let cc = self.clone_tree(c, n);
            self.attach(copy, cc);
        }
        copy
    }

    /// `loadNode` (recursively; the node's object).
    fn node(&mut self, ni: usize, depth: usize) -> Result<usize> {
        if let Some(&o) = self.node_objects.get(&ni) {
            return Ok(o);
        }
        if depth > 4096 {
            return err("glTF node hierarchy is cyclic");
        }
        let def = self
            .doc
            .json
            .get("nodes")
            .and_then(|n| n.get(ni))
            .cloned()
            .ok_or_else(|| Error(format!("node {ni} missing")))?;
        // `_loadNodeShallow`: the node's name is taken first.
        let name = match def["name"].as_str() {
            Some(n) if !n.is_empty() => self.names.unique(n),
            _ => String::new(),
        };
        let mut attachments: Vec<usize> = Vec::new();
        if let Some(mi) = index(&def["mesh"]) {
            let template = self.mesh(mi)?;
            let instancing = def
                .get("extensions")
                .and_then(|e| e.get("EXT_mesh_gpu_instancing"))
                .filter(|e| !e.is_null())
                .cloned();
            let triangles_only = template
                .parts
                .iter()
                .all(|(_, d)| self.geometries[d.geometry].mode == Mode::Triangles);
            let instances = match instancing {
                Some(ext) if triangles_only => self.instances(&ext)?,
                _ => None,
            };
            let reference = Self::node_ref(&mut self.mesh_refs, mi);
            let mut parts = Vec::new();
            for (pname, d) in &template.parts {
                let mut d = d.clone();
                let mut pname = pname.clone();
                if let Some(n) = reference
                    && !template.group
                {
                    pname = format!("{pname}_instance_{n}");
                }
                if let Some(inst) = &instances {
                    // `new InstancedMesh(...)` and `assignFinalMaterial` again.
                    let source = self.material_source(d.material);
                    d.material = self.final_material(source, Mode::Triangles, d.geometry, 2)?;
                    d.instances = Some(inst.clone());
                    d.skinned = false;
                }
                parts.push(self.add(pname, ObjectKind::Drawable(d)));
            }
            if let Some(w) = def["weights"].as_array() {
                for &p in &parts {
                    if let ObjectKind::Drawable(d) = &mut self.objects[p].kind {
                        for (k, v) in w.iter().enumerate() {
                            let v = v.as_f64().unwrap_or(0.0);
                            if k < d.morph_influences.len() {
                                d.morph_influences[k] = v;
                            } else if !d.morph_influences.is_empty() {
                                d.morph_influences.push(v);
                            }
                        }
                    }
                }
            }
            if template.group {
                let gname = match reference {
                    Some(n) => format!("{}_instance_{n}", template.name),
                    None => template.name.clone(),
                };
                let g = self.add(gname, ObjectKind::Group);
                for p in parts {
                    self.attach(g, p);
                }
                attachments.push(g);
            } else if let Some(&p) = parts.first() {
                attachments.push(p);
            }
        }
        if let Some(ci) = index(&def["camera"]) {
            // `loadCamera` reads `json.cameras[index]`'s type: a missing
            // camera throws.
            if self.doc.json["cameras"].get(ci).is_none() {
                return err(format!("node {ni}: camera {ci} missing"));
            }
            let cname = self.doc.json["cameras"][ci]["name"]
                .as_str()
                .filter(|n| !n.is_empty())
                .map(|n| self.names.unique(n))
                .unwrap_or_default();
            let cname = match Self::node_ref(&mut self.camera_refs, ci) {
                Some(n) => format!("{cname}_instance_{n}"),
                None => cname,
            };
            attachments.push(self.add(cname, ObjectKind::Camera));
        }
        if let Some(li) = def
            .get("extensions")
            .and_then(|e| e.get("KHR_lights_punctual"))
            .and_then(|l| index(&l["light"]))
        {
            let base = match self.light_defs.get(li).copied().flatten() {
                Some(o) => o,
                None => {
                    let o = self.light(li)?;
                    if self.light_defs.len() <= li {
                        self.light_defs.resize(li + 1, None);
                    }
                    self.light_defs[li] = Some(o);
                    o
                }
            };
            let o = match Self::node_ref(&mut self.light_refs, li) {
                Some(n) => {
                    let nm = format!("{}_instance_{n}", self.objects[base].name);
                    let c = self.clone_tree(base, nm);
                    self.cloned_lights.push(c);
                    c
                }
                None => base,
            };
            attachments.push(o);
        }
        let is_bone = self.bones.get(ni).copied().unwrap_or(false);
        let o = if is_bone {
            let b = self.add(String::new(), ObjectKind::Bone);
            for a in attachments {
                self.attach(b, a);
            }
            b
        } else if attachments.len() > 1 {
            let g = self.add(String::new(), ObjectKind::Group);
            for a in attachments {
                self.attach(g, a);
            }
            g
        } else if let Some(&a) = attachments.first() {
            a
        } else {
            self.add(String::new(), ObjectKind::Node)
        };
        if !name.is_empty() {
            self.objects[o].name = name;
        }
        if let Some(m) = def["matrix"].as_array() {
            // `Matrix4.fromArray`: the first 16 entries; a missing one is
            // `undefined`, NaN in the arithmetic below.
            let mut mat: Mat4 = [f64::NAN; 16];
            for (k, v) in m.iter().take(16).enumerate() {
                mat[k] = js_number(v);
            }
            // `applyMatrix4` on an identity transform: decompose.
            let (t, r, s) = math::decompose(&mat);
            self.objects[o].position = t;
            self.objects[o].quaternion = r;
            self.objects[o].scale = s;
        } else {
            if def["translation"].is_array() {
                let t = &def["translation"];
                self.objects[o].position = [f(t, 0), f(t, 1), f(t, 2)];
            }
            if def["rotation"].is_array() {
                let r = &def["rotation"];
                self.objects[o].quaternion = [f(r, 0), f(r, 1), f(r, 2), f(r, 3)];
            }
            if def["scale"].is_array() {
                let s = &def["scale"];
                self.objects[o].scale = [f(s, 0), f(s, 1), f(s, 2)];
            }
        }
        self.node_objects.insert(ni, o);
        // Children, then the skin (bound before the children are attached).
        let mut children = Vec::new();
        for c in def["children"].as_array().into_iter().flatten() {
            let c = index(c).ok_or_else(|| Error("a node child is not an index".into()))?;
            if c == ni {
                return err(format!("node {ni} is its own child"));
            }
            if self.selected_parents.get(c) != Some(&Some(ni)) {
                continue;
            }
            children.push(self.node(c, depth + 1)?);
        }
        if let Some(si) = index(&def["skin"]) {
            let skin = self.skin(si)?;
            let mut stack = vec![o];
            while let Some(x) = stack.pop() {
                if let ObjectKind::Drawable(d) = &mut self.objects[x].kind
                    && d.skinned
                {
                    d.skin = Some(skin);
                }
                stack.extend(self.objects[x].children.iter().copied());
            }
        }
        for c in children {
            self.attach(o, c);
        }
        Ok(o)
    }

    fn material_source(&self, final_index: usize) -> Option<usize> {
        self.material_sources.get(final_index).copied().flatten()
    }

    /// `EXT_mesh_gpu_instancing` attributes (`None` without any).
    fn instances(&mut self, ext: &Value) -> Result<Option<Arc<Instances>>> {
        let attrs = ext.get("attributes").and_then(Value::as_object);
        let Some(attrs) = attrs.filter(|a| !a.is_empty()) else {
            return Ok(None);
        };
        let mut read = HashMap::new();
        for (k, v) in attrs {
            let i =
                index(v).ok_or_else(|| Error("an instancing attribute is not an index".into()))?;
            read.insert(k.as_str(), self.doc.accessor(i)?);
        }
        let count = read.values().next().map_or(0, Raw::count);
        let mut matrices = Vec::with_capacity(count);
        let mut p = [0.0; 3];
        let mut q: Quat = [0.0, 0.0, 0.0, 1.0];
        let mut s = [1.0; 3];
        for k in 0..count {
            // `fromBufferAttribute` (denormalized), composed in f64, stored f32;
            // a missing attribute keeps the previous instance's value.
            if let Some(t) = read.get("TRANSLATION") {
                p = [t.get(3 * k), t.get(3 * k + 1), t.get(3 * k + 2)];
            }
            if let Some(r) = read.get("ROTATION") {
                q = [
                    r.get(4 * k),
                    r.get(4 * k + 1),
                    r.get(4 * k + 2),
                    r.get(4 * k + 3),
                ];
            }
            if let Some(sc) = read.get("SCALE") {
                s = [sc.get(3 * k), sc.get(3 * k + 1), sc.get(3 * k + 2)];
            }
            let m = math::compose(p, q, s);
            matrices.push(std::array::from_fn(|i| m[i] as f32));
        }
        let colors = read.get("_COLOR_0").map(Raw::attribute);
        Ok(Some(Arc::new(Instances { matrices, colors })))
    }

    /// `loadSkin` (joint objects, f32 inverse bind matrices).
    fn skin(&mut self, si: usize) -> Result<usize> {
        let def = self
            .doc
            .json
            .get("skins")
            .and_then(|s| s.get(si))
            .cloned()
            .ok_or_else(|| Error(format!("skin {si} missing")))?;
        let joints: Vec<usize> = def["joints"]
            .as_array()
            .map(|a| a.iter().filter_map(index).collect())
            .unwrap_or_default();
        let inverses = match index(&def["inverseBindMatrices"]) {
            Some(i) => {
                let a = self.doc.accessor(i)?;
                if a.size != 16 {
                    return err("inverseBindMatrices must be MAT4");
                }
                Some(a)
            }
            None => None,
        };
        let mut bones = Vec::new();
        let mut inv = Vec::new();
        for (k, &j) in joints.iter().enumerate() {
            let o = self.node(j, 0)?;
            bones.push(o);
            // `mat.fromArray(inverseBindMatrices.array, i * 16)`: entries
            // past the array are `undefined` (NaN).
            let m: Mat4 = match &inverses {
                Some(a) => std::array::from_fn(|e| {
                    let at = k * 16 + e;
                    if at < a.values.len() {
                        math::f32_of(a.get(at))
                    } else {
                        f64::NAN
                    }
                }),
                None => math::IDENTITY,
            };
            inv.push(m);
        }
        self.skins_out.push(Skin {
            bones,
            inverses: inv,
        });
        Ok(self.skins_out.len() - 1)
    }
}

/// Loads a model under explicit limits.
pub fn load_with(bytes: &[u8], limits: Limits) -> Result<Model> {
    if bytes.len() > limits.bytes {
        return err("GLB models must be smaller than 128 MB");
    }
    let (json, mut body) = container(bytes)?;
    for dep in json["buffers"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(json["images"].as_array().into_iter().flatten())
    {
        if let Some(uri) = dep["uri"].as_str()
            && !allowed_uri(uri)
        {
            return err(
                "Use a self-contained GLB with embedded textures and geometry. External glTF files are not loaded.",
            );
        }
    }
    let mut predicted = 0u64;
    for a in json["accessors"].as_array().into_iter().flatten() {
        match a["count"].as_u64() {
            Some(c) if c <= limits.accessor_count => {}
            _ => return err("Invalid or oversized glTF geometry"),
        }
    }
    for m in json["meshes"].as_array().into_iter().flatten() {
        for p in m["primitives"].as_array().into_iter().flatten() {
            if let Some(i) = index(&p["attributes"]["POSITION"]) {
                predicted += json["accessors"][i]["count"].as_u64().unwrap_or(0);
            }
        }
    }
    let node_count = json["nodes"].as_array().map_or(0, Vec::len);
    if predicted > limits.vertices || node_count > limits.nodes {
        return err("GLB exceeds the scene complexity budget");
    }
    let used: Vec<String> = json["extensionsUsed"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let required: Vec<String> = json["extensionsRequired"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if used.iter().any(|e| e == "KHR_draco_mesh_compression") {
        return err("Draco-compressed glTF is not supported (no DRACOLoader)");
    }
    for r in &required {
        match r.as_str() {
            "EXT_meshopt_compression" => {
                return err("meshopt-compressed glTF is not supported (no decoder)");
            }
            "KHR_texture_basisu" => return err("KTX2 textures are not supported (no KTX2Loader)"),
            _ => {}
        }
    }
    // Buffers: the GLB body or data URIs.
    let mut buffers = Vec::new();
    for (i, b) in json["buffers"].as_array().into_iter().flatten().enumerate() {
        if b.get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t != "arraybuffer")
        {
            return err("unsupported buffer type");
        }
        match b["uri"].as_str() {
            None if i == 0 => buffers.push(
                body.take()
                    .ok_or_else(|| Error("GLB has no BIN chunk".into()))?,
            ),
            Some(uri) => buffers.push(Cow::Owned(decode_data_uri(uri)?.1)),
            None => return err("a buffer without data"),
        }
    }
    let ext = Extensions {
        unlit: used.iter().any(|e| e == "KHR_materials_unlit"),
        texture_transform: used.iter().any(|e| e == "KHR_texture_transform"),
    };
    let doc = Doc {
        json: &json,
        buffers,
    };
    // Images and textures (`loadTexture`: basisu, WebP, AVIF, then source).
    let mut images = Vec::new();
    for (i, img) in json["images"].as_array().into_iter().flatten().enumerate() {
        let (bytes, mime) = if let Some(v) = index(&img["bufferView"]) {
            (
                doc.view(v)?.to_vec(),
                img["mimeType"].as_str().unwrap_or("").to_owned(),
            )
        } else if let Some(uri) = img["uri"].as_str() {
            let (mime, data) = decode_data_uri(uri)?;
            (data, img["mimeType"].as_str().map_or(mime, str::to_owned))
        } else {
            return err(format!("Image {i} is missing URI and bufferView"));
        };
        images.push(Image {
            bytes: Arc::new(bytes),
            mime,
        });
    }
    let mut textures = Vec::new();
    for t in json["textures"].as_array().into_iter().flatten() {
        let e = &t["extensions"];
        let source = index(&e["EXT_texture_webp"]["source"])
            .or_else(|| index(&e["EXT_texture_avif"]["source"]))
            .or_else(|| index(&t["source"]));
        let sampler = index(&t["sampler"])
            .and_then(|s| json["samplers"].get(s))
            .cloned()
            .unwrap_or(Value::Null);
        textures.push(source.filter(|&s| s < images.len()).map(|image| Texture {
            image,
            sampler: Sampler {
                mag: Filter::from_gl(sampler["magFilter"].as_u64(), Filter::Linear),
                min: Filter::from_gl(sampler["minFilter"].as_u64(), Filter::LinearMipmapLinear),
                wrap_s: Wrap::from_gl(sampler["wrapS"].as_u64()),
                wrap_t: Wrap::from_gl(sampler["wrapT"].as_u64()),
            },
        }));
    }
    // `_markDefs`: bones and skinned meshes; reference counts.
    let nodes = json["nodes"].as_array().cloned().unwrap_or_default();
    let mesh_count = json["meshes"].as_array().map_or(0, Vec::len);
    let mut bones = vec![false; nodes.len()];
    for s in json["skins"].as_array().into_iter().flatten() {
        for j in s["joints"].as_array().into_iter().flatten() {
            if let Some(j) = index(j)
                && j < bones.len()
            {
                bones[j] = true;
            }
        }
    }
    let mut skinned = vec![false; mesh_count];
    let mut mesh_refs: HashMap<usize, (u32, u32)> = HashMap::new();
    let mut camera_refs: HashMap<usize, (u32, u32)> = HashMap::new();
    let mut light_refs: HashMap<usize, (u32, u32)> = HashMap::new();
    for n in &nodes {
        if let Some(m) = index(&n["mesh"]) {
            mesh_refs.entry(m).or_insert((0, 0)).0 += 1;
            if n.get("skin").is_some_and(Value::is_u64) && m < skinned.len() {
                skinned[m] = true;
            }
        }
        if let Some(c) = index(&n["camera"]) {
            camera_refs.entry(c).or_insert((0, 0)).0 += 1;
        }
        if let Some(l) = n
            .get("extensions")
            .and_then(|e| e.get("KHR_lights_punctual"))
            .and_then(|l| index(&l["light"]))
        {
            light_refs.entry(l).or_insert((0, 0)).0 += 1;
        }
    }
    let selection = super::promise_order::selected_parents(&json)
        .ok_or_else(|| Error("glTF node hierarchy is cyclic or invalid".into()))?;
    let mut loader = Loader {
        doc,
        ext,
        names: Names::default(),
        textures,
        images,
        objects: Vec::new(),
        geometries: Vec::new(),
        primitive_cache: HashMap::new(),
        source_materials: HashMap::new(),
        materials: Vec::new(),
        material_sources: Vec::new(),
        final_cache: HashMap::new(),
        lights: Vec::new(),
        light_defs: Vec::new(),
        node_objects: HashMap::new(),
        selected_parents: selection.parents,
        selected_roots: selection.default_roots,
        mesh_refs,
        light_refs,
        camera_refs,
        meshes: HashMap::new(),
        skinned_meshes: skinned,
        bones,
        skins_out: Vec::new(),
        cloned_lights: Vec::new(),
    };
    // `loadScene(json.scene || 0)`.
    let scene_index = json["scene"].as_u64().filter(|&s| s != 0).unwrap_or(0) as usize;
    let scene = json["scenes"]
        .get(scene_index)
        .cloned()
        .ok_or_else(|| Error("glTF has no scene".into()))?;
    let scene_name = scene["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .map(|n| loader.names.unique(n))
        .unwrap_or_default();
    let root = loader.add(scene_name, ObjectKind::Group);
    debug_assert_eq!(root, 0);
    let mut roots = Vec::new();
    for n in scene["nodes"].as_array().into_iter().flatten() {
        let n = index(n).ok_or_else(|| Error("a scene node is not an index".into()))?;
        if !loader.selected_roots.contains(&n) {
            continue;
        }
        roots.push(loader.node(n, 0)?);
    }
    // `loadScene`: `scene.add(node)` for each scene node once every one
    // has loaded, so a scene node another node also lists moves to the
    // scene (and one listed twice moves to the end).
    for r in roots {
        loader.attach(root, r);
    }
    let clips = clips(&mut loader)?;
    let skins = loader.skins_out.clone();
    let cloned_lights = loader.cloned_lights.clone();
    let Loader {
        objects,
        geometries,
        materials,
        textures,
        images,
        lights,
        ..
    } = loader;
    let textures: Vec<Texture> = textures
        .into_iter()
        .map(|t| {
            t.unwrap_or(Texture {
                image: usize::MAX,
                sampler: Sampler {
                    mag: Filter::Linear,
                    min: Filter::LinearMipmapLinear,
                    wrap_s: Wrap::Repeat,
                    wrap_t: Wrap::Repeat,
                },
            })
        })
        .collect();
    let mut model = Model {
        objects,
        geometries,
        materials,
        textures,
        images,
        lights,
        skins,
        clips,
        extensions_used: used,
        fit: Fit {
            center: [0.0; 3],
            radius: 1.0,
        },
        vertices: 0,
        triangles: 0,
        light_targets: Vec::new(),
    };
    // `loadGltf`: counts over meshes, then the fit from `Box3.setFromObject`.
    let mut vertices = 0usize;
    let mut triangles = 0f64;
    for o in &model.objects {
        if let ObjectKind::Drawable(d) = &o.kind {
            let g = &model.geometries[d.geometry];
            if g.mode != Mode::Triangles {
                continue;
            }
            let n = g.vertex_count();
            vertices += n;
            triangles += g.index.as_ref().map_or(n, |index| index.len()) as f64 / 3.0;
        }
    }
    for i in model.traversal() {
        if let ObjectKind::Drawable(d) = &model.objects[i].kind
            && d.skinned
            && d.skin.is_none()
        {
            // `SkinnedMesh.computeBoundingBox` without a skeleton throws.
            return err("a skinned mesh without a skin");
        }
    }
    if vertices == 0 || triangles > limits.triangles as f64 {
        return err("GLB has no supported geometry or exceeds 500,000 triangles");
    }
    // `Box3.setFromObject` poses every skinned vertex
    // (`applyBoneTransform`): a weight other than 0 on a joint the skin
    // does not have reads `bones[index].matrixWorld` of `undefined`.
    for i in model.traversal() {
        if let ObjectKind::Drawable(d) = &model.objects[i].kind
            && d.skinned
            && let Some(s) = d.skin
        {
            let g = &model.geometries[d.geometry];
            let bones = model.skins[s].bones.len();
            if let (Some(si), Some(sw)) = (&g.skin_index, &g.skin_weight) {
                for v in 0..g.vertex_count() {
                    for k in 0..4 {
                        let w = sw.data.get(v * 4 + k).copied().unwrap_or(f32::NAN);
                        let j = si.data.get(v * 4 + k).copied().unwrap_or(f32::NAN);
                        if w != 0.0 && !(j >= 0.0 && (j as usize) < bones && j.fract() == 0.0) {
                            return err("a skinned vertex weights a joint its skin does not have");
                        }
                    }
                }
            }
        }
    }
    let rest = super::pose::Pose::rest(&model);
    // A directional or spot light's `target` on the runtime's clone is a
    // detached copy (`DirectionalLight.copy`: `target.clone()`), whose
    // world matrix stays the one the loader computed (the glTF scene's
    // rest pose; the origin for a light `_getNodeRef` cloned, whose target
    // was copied before any update).
    let rest_world = super::pose::world(&model, &rest, None);
    model.light_targets = vec![None; model.objects.len()];
    for (i, o) in model.objects.iter().enumerate() {
        if !matches!(o.kind, ObjectKind::Light(_)) {
            continue;
        }
        let target = o
            .children
            .iter()
            .find(|&&c| model.objects[c].kind == ObjectKind::LightTarget)
            .copied();
        if let Some(t) = target {
            model.light_targets[i] = Some(if cloned_lights.contains(&i) {
                [0.0; 3]
            } else {
                [rest_world[t][12], rest_world[t][13], rest_world[t][14]]
            });
        }
    }
    let (lo, hi) = super::pose::scene_box(&model, &rest)
        .ok_or_else(|| Error("GLB model has no measurable size".into()))?;
    let center = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ];
    let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let radius = (size[0] * size[0] + size[1] * size[1] + size[2] * size[2]).sqrt() / 2.0;
    if !radius.is_finite() || radius < 1e-10 {
        return err("GLB model has no measurable size");
    }
    model.fit = Fit { center, radius };
    model.vertices = vertices;
    model.triangles = triangles.ceil() as usize;
    Ok(model)
}

/// `loadAnimation` for every animation, tracks bound by name on the
/// runtime's clone (`PropertyBinding.findNode`: depth first from the
/// asset root; unnamed objects are found only by their original `uuid`,
/// which the clone does not keep).
fn clips(loader: &mut Loader<'_>) -> Result<Vec<Clip>> {
    let json = loader.doc.json;
    let mut out = Vec::new();
    // Name → first object in depth-first order.
    let order = {
        let mut o = Vec::new();
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            o.push(i);
            for &c in loader.objects[i].children.iter().rev() {
                stack.push(c);
            }
        }
        o
    };
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for &i in &order {
        let n = &loader.objects[i].name;
        if !n.is_empty() {
            by_name.entry(n.clone()).or_insert(i);
        }
    }
    for (ai, a) in json["animations"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let name = a["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .map_or_else(|| format!("animation_{ai}"), str::to_owned);
        let samplers = a["samplers"].as_array().cloned().unwrap_or_default();
        let mut tracks = Vec::new();
        for c in a["channels"].as_array().into_iter().flatten() {
            let target = &c["target"];
            let Some(node) = index(&target["node"]) else {
                continue;
            };
            let sampler = samplers
                .get(index(&c["sampler"]).unwrap_or(usize::MAX))
                .ok_or_else(|| Error("an animation channel without a sampler".into()))?;
            let property = match target["path"].as_str().unwrap_or("") {
                "translation" => Property::Position,
                "rotation" => Property::Quaternion,
                "scale" => Property::Scale,
                "weights" => Property::MorphTargetInfluences,
                // Unknown paths (`pointer`): `PATH_PROPERTIES[path]` is
                // undefined and the track binds to nothing.
                _ => continue,
            };
            let object = loader.node(node, 0)?;
            let input = loader
                .doc
                .accessor(index(&sampler["input"]).ok_or_else(|| Error("sampler input".into()))?)?;
            let output = loader.doc.accessor(
                index(&sampler["output"]).ok_or_else(|| Error("sampler output".into()))?,
            )?;
            let interpolation = match sampler["interpolation"].as_str() {
                Some("STEP") => Interpolation::Step,
                Some("CUBICSPLINE") => Interpolation::CubicSpline,
                _ => Interpolation::Linear,
            };
            // `_getArrayFromAccessor`: normalized outputs scaled into f32.
            let values: Vec<f32> = (0..output.values.len())
                .map(|i| output.get(i) as f32)
                .collect();
            let times: Vec<f32> = (0..input.values.len())
                .map(|i| input.raw(i) as f32)
                .collect();
            let value_size = if times.is_empty() {
                0
            } else {
                values.len() / times.len()
            };
            let targets: Vec<usize> = if property == Property::MorphTargetInfluences {
                let mut list = Vec::new();
                let mut stack = vec![object];
                while let Some(x) = stack.pop() {
                    if let ObjectKind::Drawable(d) = &loader.objects[x].kind
                        && !d.morph_influences.is_empty()
                    {
                        list.push(x);
                    }
                    for &ch in loader.objects[x].children.iter().rev() {
                        stack.push(ch);
                    }
                }
                list
            } else {
                vec![object]
            };
            let suffix = match property {
                Property::Position => "position",
                Property::Quaternion => "quaternion",
                Property::Scale => "scale",
                Property::MorphTargetInfluences => "morphTargetInfluences",
            };
            for t in targets {
                let tname = loader.objects[t].name.clone();
                tracks.push(Track {
                    name: if tname.is_empty() {
                        format!("<unnamed {t}>.{suffix}")
                    } else {
                        format!("{tname}.{suffix}")
                    },
                    target: if tname.is_empty() {
                        None
                    } else {
                        by_name.get(&tname).copied()
                    },
                    property,
                    interpolation,
                    times: times.clone(),
                    values: values.clone(),
                    value_size,
                });
            }
        }
        let duration = tracks
            .iter()
            .filter_map(|t| t.times.last().map(|v| f64::from(*v)))
            .fold(0.0f64, f64::max);
        out.push(Clip {
            name,
            tracks,
            duration,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    use crate::model_import::glb::GlbWriter;
    use serde_json::{Map, json};

    #[test]
    fn glb_container_borrows_its_bin_chunk() {
        let mut writer = GlbWriter::default();
        writer.view(&[1, 2, 3, 4], None);
        let mut document = Map::new();
        document.insert("asset".into(), json!({"version":"2.0"}));
        let bytes = writer.finish(document);
        let (_, bin) = container(&bytes).unwrap();
        let Cow::Borrowed(bin) = bin.unwrap() else {
            panic!("GLB BIN was copied")
        };
        assert_eq!(bin, [1, 2, 3, 4]);
        assert!(bin.as_ptr() >= bytes.as_ptr());
        assert!((bin.as_ptr() as usize) + bin.len() <= (bytes.as_ptr() as usize) + bytes.len());
    }

    #[test]
    fn sparse_float_accessor_shares_exact_attribute_bits() {
        let original = [f32::from_bits(0x7f80_0001), -0.0, 1.0, 0.0];
        let mut bytes: Vec<u8> = original.iter().flat_map(|v| v.to_le_bytes()).collect();
        let sparse_index = bytes.len();
        bytes.push(2);
        let sparse_value = bytes.len();
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        let json = json!({
            "bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":16},
                {"buffer":0,"byteOffset":sparse_index,"byteLength":1},
                {"buffer":0,"byteOffset":sparse_value,"byteLength":4}
            ],
            "accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"SCALAR",
                "sparse":{"count":1,"indices":{"bufferView":1,"componentType":5121},
                    "values":{"bufferView":2}}}]
        });
        let doc = Doc {
            json: &json,
            buffers: vec![Cow::Borrowed(&bytes)],
        };
        let raw = doc.accessor(0).unwrap();
        let attribute = raw.attribute();
        let Components::Float(values) = &raw.values else {
            panic!("float accessor made integer staging")
        };
        assert!(Arc::ptr_eq(values, &attribute.data));
        assert_eq!(
            attribute.data[0].to_bits(),
            (f64::from(original[0]) as f32).to_bits()
        );
        assert_eq!(attribute.data[1].to_bits(), (-0.0f32).to_bits());
        assert_eq!(attribute.data[2].to_bits(), 2.0f32.to_bits());
        assert_eq!(raw.raw(2), 2.0);
    }

    #[test]
    fn wide_integer_accessor_keeps_exact_index() {
        let bytes = u32::MAX.to_le_bytes();
        let json = json!({
            "bufferViews":[{"buffer":0,"byteLength":4}],
            "accessors":[{"bufferView":0,"componentType":5125,"count":1,"type":"SCALAR"}]
        });
        let doc = Doc {
            json: &json,
            buffers: vec![Cow::Borrowed(&bytes)],
        };
        let raw = doc.accessor(0).unwrap();
        assert_eq!(raw.raw(0), u32::MAX as f64);
    }

    #[test]
    fn integer_normalization_and_padded_matrix_keep_getter_values() {
        let bytes = [128u8, 127, 0, 1, 2, 3, 99, 4, 5, 6, 99, 7, 8, 9, 99];
        let json = json!({
            "bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":3},
                {"buffer":0,"byteOffset":3,"byteLength":12}
            ],
            "accessors":[
                {"bufferView":0,"componentType":5120,"normalized":true,"count":1,"type":"VEC3"},
                {"bufferView":1,"componentType":5121,"count":1,"type":"MAT3"}
            ]
        });
        let doc = Doc {
            json: &json,
            buffers: vec![Cow::Borrowed(&bytes)],
        };
        let normalized = doc.accessor(0).unwrap().attribute();
        assert_eq!(normalized.data.as_slice(), &[-1.0, 1.0, 0.0]);
        let matrix = doc.accessor(1).unwrap().attribute();
        assert_eq!(
            matrix.data.as_slice(),
            &[1., 2., 3., 4., 5., 6., 7., 8., 9.]
        );
    }
}
