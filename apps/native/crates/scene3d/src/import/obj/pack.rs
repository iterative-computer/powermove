//! Reused from pm-geom3d on integration 78294c4c9; retained locally so
//! Studio can use its 512 MiB import budget without changing engine-owned code.
//! `prepareModelImport`: a user-selected OBJ with its MTL and image files
//! becomes one portable GLB. No sibling file is read implicitly and no URL
//! from the model source is fetched.
//!
//! The GLB is what three.js r180's `GLTFExporter` writes for the
//! `OBJLoader` group after the app's material conversion: Phong materials
//! become standard ones (double-sided, metalness 0, roughness
//! `clamp(√(2 / (Ns + 2)), 0.02, 1)`, unnamed), a bump map without a normal
//! map becomes a normal map, images are flipped for glTF's top-left origin,
//! and each material group emits only its own vertices.

use super::super::texture;
use super::js;
use super::loader::{self, Kind, Phong, TextureParams};
use pm_geom3d::model_import::glb::GlbWriter;
use pm_geom3d::model_import::{Error, Result};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// A selected file: its name, MIME type (`File.type`, may be empty), and bytes.
#[derive(Clone, Debug)]
pub struct ImportFile {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// What the import stores.
#[derive(Clone, Debug, PartialEq)]
pub enum Prepared {
    /// The model file as chosen (glTF, GLB, or a lone OBJ for a mesh layer).
    Unchanged(usize),
    /// The packed bundle, named after the OBJ with a `.glb` extension.
    Glb { name: String, bytes: Vec<u8> },
}

const MAX_FILES: usize = 128;
const MAX_TOTAL: usize = 512 * 1024 * 1024;

fn err<T>(m: impl Into<String>) -> Result<T> {
    Err(Error(m.into()))
}

fn ends_with_ci(name: &str, exts: &[&str]) -> bool {
    let lower = name.to_ascii_lowercase();
    exts.iter().any(|e| lower.ends_with(e))
}

/// `leaf(path)`: the lowercased last path segment, percent-decoded.
fn leaf(path: &str) -> Result<String> {
    let decoded = js::decode_uri_component(path).ok_or_else(|| Error("URI malformed".into()))?;
    let decoded = decoded.replace('\\', "/");
    Ok(decoded.rsplit('/').next().unwrap_or("").to_lowercase())
}

/// Blob.text(): UTF-8 with replacement, a leading BOM removed.
fn text(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    s.strip_prefix('\u{FEFF}').unwrap_or(&s).to_string()
}

fn is_image(f: &ImportFile) -> bool {
    matches!(f.mime.as_str(), "image/png" | "image/jpeg" | "image/webp")
        || ends_with_ci(&f.name, &[".png", ".jpg", ".jpeg", ".webp"])
}

/// A decoded texture image (RGBA8, EXIF orientation applied as an `<img>`
/// applies it).
fn decode(bytes: &[u8]) -> Option<image::RgbaImage> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().ok()?;
    decoder.set_limits(limits).ok()?;
    let orientation = decoder.orientation().ok()?;
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    img.apply_orientation(orientation);
    Some(texture::capped(img.into_rgba8()))
}

#[cfg(test)]
fn png(img: &image::RgbaImage) -> Vec<u8> {
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("PNG encoding of an RGBA8 buffer");
    out
}

/// `bumpNormals(texture, strength)`: scalar bump to a tangent-space normal
/// map, central differences with wrap-around, on the image as drawn.
pub fn bump_normals(img: &image::RgbaImage, strength: f64) -> image::RgbaImage {
    let src = texture::capped(img.clone());
    let (w, h) = src.dimensions();
    let (wi, hi) = (i64::from(w), i64::from(h));
    let sample = |x: i64, y: i64| -> f64 {
        let p = src.get_pixel(
            (((x + wi) % wi) as u32).min(w - 1),
            (((y + hi) % hi) as u32).min(h - 1),
        );
        (f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])) / 765.0
    };
    let mut out = image::RgbaImage::new(w, h);
    for y in 0..hi {
        for x in 0..wi {
            let v = [
                (sample(x - 1, y) - sample(x + 1, y)) * strength,
                (sample(x, y - 1) - sample(x, y + 1)) * strength,
                1.0,
            ];
            // Vector3.normalize(): divideScalar(length || 1) as a reciprocal multiply.
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let s = 1.0 / if len == 0.0 || len.is_nan() { 1.0 } else { len };
            let n = v.map(|c| c * s);
            out.put_pixel(
                x as u32,
                y as u32,
                image::Rgba([
                    js::u8_clamp((n[0] * 0.5 + 0.5) * 255.0),
                    js::u8_clamp((n[1] * 0.5 + 0.5) * 255.0),
                    js::u8_clamp((n[2] * 0.5 + 0.5) * 255.0),
                    255,
                ]),
            );
        }
    }
    out
}

/// GLTFExporter's view of a three.js texture.
#[derive(Clone)]
struct Tex {
    /// The selected file (for a loaded texture) or a generated normal map.
    source: TexSource,
    wrap: u32,
    repeat: [f64; 2],
    offset: [f64; 2],
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum TexSource {
    File(usize),
    /// `bumpNormals` of a file with its strength (`f64` bits); each
    /// material conversion draws its own canvas.
    Bump(usize, u64),
    Opacity(Option<usize>, usize),
}

/// The converted material of one primitive.
struct Mat {
    standard: bool,
    name: String,
    color: [f64; 3],
    opacity: f64,
    transparent: bool,
    double_sided: bool,
    roughness: f64,
    emissive: [f64; 3],
    map: Option<Tex>,
    emissive_map: Option<Tex>,
    normal_map: Option<Tex>,
}

const REPEAT: u32 = 10497;
const CLAMP: u32 = 33071;
const LINEAR: u32 = 9729;
const LINEAR_MIPMAP_LINEAR: u32 = 9987;

struct Exporter<'a> {
    w: GlbWriter,
    decoded: &'a HashMap<usize, image::RgbaImage>,
    images: Vec<Value>,
    image_cache: HashMap<(TexSource, bool), usize>,
    image_content: HashMap<[u8; 32], usize>,
    texture_cache: HashMap<(usize, usize), usize>,
    sampler_cache: HashMap<u32, usize>,
    material_cache: HashMap<String, usize>,
    textures: Vec<Value>,
    samplers: Vec<Value>,
    materials: Vec<Value>,
    transform_used: bool,
}

impl Exporter<'_> {
    fn image(&mut self, source: &TexSource, color: bool) -> Result<usize> {
        let key = (source.clone(), color);
        if let Some(&i) = self.image_cache.get(&key) {
            return Ok(i);
        }
        let drawn = match *source {
            TexSource::File(index) => self.decoded[&index].clone(),
            TexSource::Bump(index, strength) => {
                bump_normals(&self.decoded[&index], f64::from_bits(strength))
            }
            TexSource::Opacity(base, index) => {
                let mut opacity = self.decoded[&index].clone();
                for pixel in opacity.pixels_mut() {
                    // OBJ's alphaMap uses green as the linear scalar.
                    pixel[0] = pixel[1];
                }
                texture::with_opacity(base.map(|index| self.decoded[&index].clone()), opacity)
            }
        };
        let flipped = image::imageops::flip_vertical(&drawn);
        let (mime, bytes) = texture::encode(flipped, color).map_err(|e| Error(e.to_string()))?;
        let hash = Sha256::digest(&bytes).into();
        let index = *self.image_content.entry(hash).or_insert_with(|| {
            let view = self.w.view(&bytes, None);
            self.images
                .push(json!({ "mimeType": mime, "bufferView": view }));
            self.images.len() - 1
        });
        self.image_cache.insert(key, index);
        Ok(index)
    }

    fn texture(&mut self, t: &Tex, color: bool) -> Result<Value> {
        let source = self.image(&t.source, color)?;
        let sampler = *self.sampler_cache.entry(t.wrap).or_insert_with(|| {
            self.samplers.push(json!({
                "magFilter": LINEAR, "minFilter": LINEAR_MIPMAP_LINEAR,
                "wrapS": t.wrap, "wrapT": t.wrap,
            }));
            self.samplers.len() - 1
        });
        let index = *self
            .texture_cache
            .entry((source, sampler))
            .or_insert_with(|| {
                self.textures
                    .push(json!({ "sampler": sampler, "source": source }));
                self.textures.len() - 1
            });
        let mut def = Map::new();
        def.insert("index".into(), json!(index));
        def.insert("texCoord".into(), json!(0));
        // applyTextureTransform.
        let mut transform = Map::new();
        if t.offset != [0.0, 0.0] {
            transform.insert("offset".into(), json!(t.offset));
        }
        if t.repeat != [1.0, 1.0] {
            transform.insert("scale".into(), json!(t.repeat));
        }
        if !transform.is_empty() {
            def.insert(
                "extensions".into(),
                json!({ "KHR_texture_transform": transform }),
            );
            self.transform_used = true;
        }
        Ok(Value::Object(def))
    }

    fn material(&mut self, m: &Mat) -> Result<usize> {
        let mut pbr = Map::new();
        let color = [m.color[0], m.color[1], m.color[2], m.opacity];
        if color != [1.0; 4] {
            pbr.insert("baseColorFactor".into(), json!(color));
        }
        if m.standard {
            pbr.insert("metallicFactor".into(), json!(0.0));
            pbr.insert("roughnessFactor".into(), json!(m.roughness));
        } else {
            pbr.insert("metallicFactor".into(), json!(0));
            pbr.insert("roughnessFactor".into(), json!(1));
        }
        if let Some(t) = &m.map {
            let t = self.texture(t, true)?;
            pbr.insert("baseColorTexture".into(), t);
        }
        let mut def = Map::new();
        def.insert("pbrMetallicRoughness".into(), Value::Object(pbr));
        if m.standard {
            if js::max(js::max(m.emissive[0], m.emissive[1]), m.emissive[2]) > 0.0 {
                def.insert("emissiveFactor".into(), json!(m.emissive));
            }
            if let Some(t) = &m.emissive_map {
                let t = self.texture(t, true)?;
                def.insert("emissiveTexture".into(), t);
            }
            if let Some(t) = &m.normal_map {
                let t = self.texture(t, false)?;
                def.insert("normalTexture".into(), t);
            }
        }
        if m.transparent {
            def.insert("alphaMode".into(), json!("BLEND"));
        }
        if m.double_sided {
            def.insert("doubleSided".into(), json!(true));
        }
        if !m.name.is_empty() {
            def.insert("name".into(), json!(m.name));
        }
        let material = Value::Object(def);
        Ok(*self
            .material_cache
            .entry(material.to_string())
            .or_insert_with(|| {
                self.materials.push(material);
                self.materials.len() - 1
            }))
    }
}

/// Float32BufferAttribute: f64 values stored as f32, `N` per item.
fn items<const N: usize>(flat: &[f64]) -> Vec<[f32; N]> {
    flat.chunks_exact(N)
        .map(|c| std::array::from_fn(|i| c[i] as f32))
        .collect()
}

/// GLTFExporter's normal check: when any normal is more than 0.0005 from
/// unit length, every normal is normalized ((1, 0, 0) for zero).
fn export_normals(normals: Vec<[f32; 3]>) -> Vec<[f32; 3]> {
    let len = |v: &[f32; 3]| {
        let [x, y, z] = v.map(f64::from);
        (x * x + y * y + z * z).sqrt()
    };
    if normals.iter().all(|v| (len(v) - 1.0).abs() <= 0.0005) {
        return normals;
    }
    normals
        .iter()
        .map(|v| {
            let d = v.map(f64::from);
            if d == [0.0; 3] {
                [1.0, 0.0, 0.0]
            } else {
                let l = len(v);
                let s = 1.0 / if l == 0.0 || l.is_nan() { 1.0 } else { l };
                d.map(|c| (c * s) as f32)
            }
        })
        .collect()
}

/// `prepareModelImport(files)`.
pub fn prepare_import(files: &[ImportFile]) -> Result<Prepared> {
    if files.is_empty() || files.len() > MAX_FILES {
        return err("Choose a model and its material/texture files");
    }
    if files.iter().map(|f| f.bytes.len()).sum::<usize>() > MAX_TOTAL {
        return err("Model files must be smaller than 512 MB in total");
    }
    let models: Vec<usize> = (0..files.len())
        .filter(|&i| ends_with_ci(&files[i].name, &[".obj", ".glb", ".gltf"]))
        .collect();
    if models.len() != 1 {
        return err("Choose one model at a time with its material and texture files");
    }
    let model = models[0];
    if !ends_with_ci(&files[model].name, &[".obj"]) || files.len() == 1 {
        return Ok(Prepared::Unchanged(model));
    }
    let source = text(&files[model].bytes);
    // Validate geometry before allocating it.
    super::parse_obj(&source).map_err(Error)?;
    let mtls: Vec<&ImportFile> = files
        .iter()
        .filter(|f| ends_with_ci(&f.name, &[".mtl"]))
        .collect();
    if mtls.is_empty() {
        return err("Include the OBJ material (.mtl) file with its textures");
    }
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        let key = leaf(&f.name)?;
        if by_name.insert(key, i).is_some() {
            return err(format!("Duplicate file name: {}", f.name));
        }
    }
    let mtl_text = mtls
        .iter()
        .map(|f| text(&f.bytes))
        .collect::<Vec<_>>()
        .join("\n");
    let infos = loader::parse_mtl(&mtl_text);
    // MaterialCreator.preload(): every material, every texture it names.
    let mut phongs: Vec<Phong> = vec![];
    let mut texture_files: HashMap<String, usize> = HashMap::new();
    for (name, props) in &infos {
        let p = loader::phong(name, Some(props));
        for (_, t) in &p.maps {
            let f = resolve(&t.url, files, &by_name)?;
            texture_files.insert(t.url.clone(), f);
        }
        phongs.push(p);
    }
    let mut decoded = HashMap::new();
    for &f in texture_files.values() {
        if let std::collections::hash_map::Entry::Vacant(e) = decoded.entry(f) {
            e.insert(
                decode(&files[f].bytes)
                    .ok_or_else(|| Error("A selected texture could not be decoded".into()))?,
            );
        }
    }
    let objects = loader::parse_obj(&source);
    let create = |name: &str| -> Phong {
        phongs
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .unwrap_or_else(|| loader::phong(name, None))
    };
    let tex = |t: &TextureParams| Tex {
        source: TexSource::File(texture_files[&t.url]),
        wrap: REPEAT,
        repeat: t.scale,
        offset: t.offset,
    };
    let mut ex = Exporter {
        w: GlbWriter::default(),
        decoded: &decoded,
        images: vec![],
        image_cache: HashMap::new(),
        image_content: HashMap::new(),
        texture_cache: HashMap::new(),
        sampler_cache: HashMap::new(),
        material_cache: HashMap::new(),
        textures: vec![],
        samplers: vec![],
        materials: vec![],
        transform_used: false,
    };
    let mut meshes = vec![];
    let mut nodes = vec![json!({})];
    for o in &objects {
        let multi = o.materials.len() > 1;
        let mut mats = vec![];
        for u in &o.materials {
            let p = create(&u.name);
            let map_of = |slot: &str| p.maps.iter().find(|(s, _)| *s == slot).map(|(_, t)| tex(t));
            let mat = if o.kind == Kind::Mesh {
                let mut normal_map = map_of("normalMap");
                if normal_map.is_none()
                    && let Some(b) = map_of("bumpMap")
                {
                    let TexSource::File(f) = b.source else {
                        unreachable!()
                    };
                    normal_map = Some(Tex {
                        source: TexSource::Bump(f, p.bump_scale.to_bits()),
                        wrap: CLAMP,
                        repeat: [1.0, 1.0],
                        offset: [0.0, 0.0],
                    });
                }
                let mut base_map = map_of("map");
                if let Some(mut opacity) = map_of("alphaMap") {
                    let TexSource::File(alpha) = opacity.source else {
                        unreachable!()
                    };
                    if let Some(base) = &mut base_map {
                        let TexSource::File(color) = base.source else {
                            unreachable!()
                        };
                        base.source = TexSource::Opacity(Some(color), alpha);
                    } else {
                        opacity.source = TexSource::Opacity(None, alpha);
                        base_map = Some(opacity);
                    }
                }
                let roughness = js::max(0.02, js::min(1.0, (2.0 / (p.shininess + 2.0)).sqrt()));
                Mat {
                    standard: true,
                    name: String::new(),
                    color: p.color,
                    opacity: p.opacity,
                    transparent: p.transparent,
                    double_sided: true,
                    roughness,
                    emissive: p.emissive,
                    map: base_map,
                    emissive_map: map_of("emissiveMap"),
                    normal_map,
                }
            } else {
                Mat {
                    standard: false,
                    name: p.name.clone(),
                    color: p.color,
                    opacity: p.opacity,
                    transparent: p.transparent,
                    double_sided: false,
                    roughness: 1.0,
                    emissive: [0.0; 3],
                    map: if o.kind == Kind::Points {
                        map_of("map")
                    } else {
                        None
                    },
                    emissive_map: None,
                    normal_map: None,
                }
            };
            mats.push(mat);
        }
        let positions = items::<3>(&o.vertices);
        let count = positions.len();
        let normals = export_normals(items::<3>(&o.normals));
        let colors = items::<3>(&o.colors);
        let uv = items::<2>(&o.uvs);
        let mode = match o.kind {
            Kind::Mesh => 4,
            Kind::Line => 1,
            Kind::Points => 0,
        };
        let mut primitives = vec![];
        for (mi, group) in o.materials.iter().enumerate() {
            let start = if multi {
                group.group_start.max(0) as usize
            } else {
                0
            };
            let n = if multi {
                (group.group_count.max(0) as usize).min(count.saturating_sub(start))
            } else {
                count
            };
            if n == 0 {
                continue;
            }
            let range = start..start + n;
            let mut attributes = json!({"POSITION": ex.w.floats(&positions[range.clone()], true)});
            if !normals.is_empty() {
                attributes["NORMAL"] = json!(ex.w.floats(&normals[range.clone()], false));
            }
            if !colors.is_empty() {
                attributes["COLOR_0"] = json!(ex.w.floats(&colors[range.clone()], false));
            }
            if o.has_uv_indices {
                attributes["TEXCOORD_0"] = json!(ex.w.floats(&uv[range], false));
            }
            primitives.push(json!({"mode": mode, "attributes": attributes, "material": ex.material(&mats[mi])?}));
        }
        if primitives.is_empty() {
            continue;
        }
        meshes.push(json!({ "primitives": primitives }));
        let mut node = Map::new();
        if !o.name.is_empty() {
            node.insert("name".into(), json!(o.name));
        }
        node.insert("mesh".into(), json!(meshes.len() - 1));
        nodes.push(Value::Object(node));
    }
    if nodes.len() > 1 {
        nodes[0]["children"] = json!((1..nodes.len()).collect::<Vec<_>>());
    }
    let mut doc = Map::new();
    doc.insert("asset".into(), json!({ "version": "2.0", "generator": "Powermove pm-geom3d (three.js r180 GLTFExporter layout)" }));
    if ex.transform_used {
        doc.insert("extensionsUsed".into(), json!(["KHR_texture_transform"]));
    }
    doc.insert("scene".into(), json!(0));
    doc.insert(
        "scenes".into(),
        json!([{ "name": "AuxScene", "nodes": [0] }]),
    );
    doc.insert("nodes".into(), Value::Array(nodes));
    doc.insert("meshes".into(), Value::Array(meshes));
    doc.insert(
        "materials".into(),
        Value::Array(std::mem::take(&mut ex.materials)),
    );
    if !ex.images.is_empty() {
        doc.insert(
            "images".into(),
            Value::Array(std::mem::take(&mut ex.images)),
        );
        doc.insert(
            "textures".into(),
            Value::Array(std::mem::take(&mut ex.textures)),
        );
        doc.insert(
            "samplers".into(),
            Value::Array(std::mem::take(&mut ex.samplers)),
        );
    }
    let bytes = ex.w.finish(doc);
    if bytes.len() > MAX_TOTAL {
        return err("Converted OBJ exceeds 512 MB");
    }
    let name = files[model].name.clone();
    let stem = &name[..name.len() - 4];
    Ok(Prepared::Glb {
        name: format!("{stem}.glb"),
        bytes,
    })
}

/// The URL modifier: texture references resolve only to selected images.
fn resolve(url: &str, files: &[ImportFile], by_name: &HashMap<String, usize>) -> Result<usize> {
    let lower = url.to_ascii_lowercase();
    let scheme = lower
        .find(':')
        .is_some_and(|i| i > 0 && lower[..i].bytes().all(|b| b.is_ascii_lowercase()));
    if url.starts_with("blob:") {
        // A blob URL is passed through and fails to load.
        return err("A selected texture could not be decoded");
    }
    if scheme || url.starts_with("//") {
        return err("Model texture references must be selected local files");
    }
    match by_name.get(&leaf(url)?) {
        Some(&i) if is_image(&files[i]) => Ok(i),
        _ => err(format!("Select the missing texture file: {url}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_geom3d::model_import as gltf;

    fn file(name: &str, bytes: &[u8]) -> ImportFile {
        ImportFile {
            name: name.into(),
            mime: String::new(),
            bytes: bytes.to_vec(),
        }
    }

    fn png_of(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        png(&image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba(f(x, y))
        }))
    }

    const QUAD: &str = "mtllib m.mtl\no Card\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl Paper\nf 1/1 2/2 3/3 4/4\n";

    #[test]
    fn checks_the_selection_like_prepare_model_import() {
        let e = |files: &[ImportFile]| prepare_import(files).unwrap_err().0;
        assert_eq!(e(&[]), "Choose a model and its material/texture files");
        assert_eq!(
            e(&[file("a.obj", b"v"), file("b.glb", b"")]),
            "Choose one model at a time with its material and texture files"
        );
        assert_eq!(
            prepare_import(&[file("a.obj", QUAD.as_bytes())]).unwrap(),
            Prepared::Unchanged(0)
        );
        assert_eq!(
            prepare_import(&[file("x.png", b""), file("a.GLB", b"")]).unwrap(),
            Prepared::Unchanged(1)
        );
        assert_eq!(
            e(&[file("a.obj", QUAD.as_bytes()), file("t.png", b"")]),
            "Include the OBJ material (.mtl) file with its textures"
        );
        assert_eq!(
            e(&[
                file("a.obj", QUAD.as_bytes()),
                file("m.mtl", b""),
                file("dir/M.mtl", b"")
            ]),
            "Duplicate file name: dir/M.mtl"
        );
        let mtl = b"newmtl Paper\nmap_Kd textures/paper.png\n";
        assert_eq!(
            e(&[file("a.obj", QUAD.as_bytes()), file("m.mtl", mtl)]),
            "Select the missing texture file: textures/paper.png"
        );
        assert_eq!(
            e(&[
                file("a.obj", QUAD.as_bytes()),
                file("m.mtl", b"newmtl P\nmap_Kd https://x/a.png\n")
            ]),
            "Model texture references must be selected local files"
        );
        assert_eq!(
            e(&[
                file("a.obj", QUAD.as_bytes()),
                file("m.mtl", mtl),
                file("paper.png", b"not a png")
            ]),
            "A selected texture could not be decoded"
        );
        assert_eq!(
            e(&[file("a.obj", b"v 0 0 0\n"), file("m.mtl", b"")]),
            "OBJ does not contain any faces"
        );
    }

    #[test]
    fn packs_textures_flipped_with_their_transform() {
        let paper = png_of(2, 2, |_, y| {
            if y == 0 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        });
        let mtl = b"newmtl Paper\nKd 1 1 1\nNs 0\nmap_Kd -o 0.25 0 0 paper.png\n";
        let out = prepare_import(&[
            file("card.obj", QUAD.as_bytes()),
            file("m.mtl", mtl),
            file("PAPER.png", &paper),
        ])
        .unwrap();
        let Prepared::Glb { name, bytes } = out else {
            panic!("packed")
        };
        assert_eq!(name, "card.glb");
        let json = gltf::json_of(&bytes).unwrap();
        assert!(json["images"][0].get("extras").is_none());
        let (model, scene, facts) = gltf::import(&bytes).unwrap();
        assert_eq!((facts.vertices, facts.triangles), (6, 2));
        assert_eq!(scene.objects[0].name, "AuxScene");
        let card = scene.objects.iter().find(|o| o.name == "Card").unwrap();
        let m = gltf::material_facts(&model, card).unwrap();
        assert_eq!(
            (m.class, m.side, m.roughness),
            ("MeshStandardMaterial", 2, Some(1.0))
        );
        assert_eq!(m.maps, vec![("map", 0)]);
        let mat = &model.materials[0];
        let t = mat.base_color_texture.as_ref().unwrap();
        assert_eq!(t.transform.unwrap().offset, [0.25, 0.0]);
        assert_eq!(model.textures[0].sampler.wrap_s, REPEAT);
        // The image is stored flipped: blue (OBJ's bottom row) first.
        let img = image::load_from_memory(&model.images[0].bytes)
            .unwrap()
            .to_rgba8();
        assert!(img.get_pixel(0, 0)[2] > 245 && img.get_pixel(0, 0)[0] < 10);
        assert!(img.get_pixel(0, 1)[0] > 245 && img.get_pixel(0, 1)[2] < 10);
    }

    #[test]
    fn bump_maps_become_clamped_normal_maps() {
        // A horizontal ramp: brighter to the right, so normals lean to −x.
        let bump = png_of(4, 3, |x, _| {
            let v = (x * 60) as u8;
            [v, v, v, 255]
        });
        let mtl = b"newmtl Paper\nbump -bm 2 b.png\n";
        let out = prepare_import(&[
            file("card.obj", QUAD.as_bytes()),
            file("m.mtl", mtl),
            file("b.png", &bump),
        ])
        .unwrap();
        let Prepared::Glb { bytes, .. } = out else {
            panic!("packed")
        };
        let (model, _, _) = gltf::import(&bytes).unwrap();
        let mat = &model.materials[0];
        assert!(mat.normal_texture.is_some() && mat.base_color_texture.is_none());
        assert_eq!(model.textures[0].sampler.wrap_s, CLAMP);
        let img = image::load_from_memory(&model.images[0].bytes)
            .unwrap()
            .to_rgba8();
        // x = 1: samples 0 and 120 → dx = −120 / 255 · 2.
        let dx = (0.0 - 120.0 / 255.0) * 2.0f64;
        let len = (dx * dx + 1.0).sqrt();
        let want = |c: f64| js::u8_clamp((c * (1.0 / len) * 0.5 + 0.5) * 255.0);
        assert_eq!(img.get_pixel(1, 0).0, [want(dx), 128, want(1.0), 255]);
    }

    #[test]
    fn oversized_bump_uses_native_normal_map_without_browser_metadata() {
        let bump = png_of(2050, 2, |x, _| [(x % 256) as u8, 0, 0, 255]);
        let out = prepare_import(&[
            file("card.obj", QUAD.as_bytes()),
            file("m.mtl", b"newmtl Paper\nmap_bump -bm 0.6 height.png\n"),
            file("height.png", &bump),
        ])
        .unwrap();
        let Prepared::Glb { bytes, .. } = out else {
            panic!("packed")
        };
        let json = gltf::json_of(&bytes).unwrap();
        assert!(json["materials"][0].get("extras").is_none());
        assert!(json["images"][0].get("extras").is_none());
        let model = gltf::load(&bytes).unwrap();
        let image = image::load_from_memory(&model.images[0].bytes).unwrap();
        assert_eq!(image.width(), 2048);
        assert_eq!(image.height(), 2);
    }
}
